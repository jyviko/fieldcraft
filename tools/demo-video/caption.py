"""Caption pictures for the demo video, drawn in the Python standard library.

A minimal TrueType reader (the `cmap` format 4, `loca`, `glyf` simple and composite outlines and
`hmtx` advances of the TrueType specification), a scanline rasteriser (non-zero winding,
sub-scanlines with exact horizontal coverage) and a PNG writer (`zlib`, `struct`). It draws the
caption bar under the app window with the interface's own font, `crates/ui-egui/fonts/Inter-SemiBold.ttf` (SIL OFL
1.1, ATTRIBUTION.md), and ffmpeg composites it with its `overlay` filter.

No hinting, kerning or shaping: captions are short ASCII-and-punctuation labels.
"""

import math
import struct
import zlib

# Points per quadratic segment when flattening an outline.
CURVE_STEPS = 8
# Sub-scanlines per pixel row (vertical anti-aliasing; horizontal coverage is exact).
SUBSAMPLES = 5
# Nesting of composite glyphs a font may use before we refuse it.
MAX_COMPOSITE_DEPTH = 8


class FontError(ValueError):
    pass


class Font:
    """The outlines and advances of a TrueType (`glyf`) font."""

    def __init__(self, data):
        self.data = data
        if len(data) < 12:
            raise FontError("not a font file")
        (num_tables,) = struct.unpack_from(">H", data, 4)
        self.tables = {}
        for i in range(num_tables):
            tag, _checksum, offset, length = struct.unpack_from(">4sIII", data, 12 + 16 * i)
            if offset + length > len(data):
                raise FontError(f"table {tag!r} runs past the end of the file")
            self.tables[tag.decode("latin-1")] = (offset, length)
        for tag in ("head", "hhea", "hmtx", "maxp", "cmap", "loca", "glyf"):
            if tag not in self.tables:
                raise FontError(f"no {tag} table (only TrueType outlines are supported)")
        head = self.tables["head"][0]
        (self.units_per_em,) = struct.unpack_from(">H", data, head + 18)
        (loc_format,) = struct.unpack_from(">h", data, head + 50)
        (self.num_glyphs,) = struct.unpack_from(">H", data, self.tables["maxp"][0] + 4)
        hhea = self.tables["hhea"][0]
        (self.num_h_metrics,) = struct.unpack_from(">H", data, hhea + 34)
        loca = self.tables["loca"][0]
        if loc_format == 0:
            self.loca = [2 * v for v in struct.unpack_from(f">{self.num_glyphs + 1}H", data, loca)]
        else:
            self.loca = list(struct.unpack_from(f">{self.num_glyphs + 1}I", data, loca))
        self.cmap = self._read_cmap()

    @classmethod
    def load(cls, path):
        with open(path, "rb") as f:
            return cls(f.read())

    def _read_cmap(self):
        """The Unicode BMP subtable (format 4) as a code point -> glyph map."""
        data = self.data
        base = self.tables["cmap"][0]
        (count,) = struct.unpack_from(">H", data, base + 2)
        for i in range(count):
            platform, encoding, offset = struct.unpack_from(">HHI", data, base + 4 + 8 * i)
            sub = base + offset
            if (platform, encoding) in ((3, 1), (0, 3)) and struct.unpack_from(">H", data, sub)[0] == 4:
                return self._read_cmap4(sub)
        raise FontError("no Unicode BMP (format 4) cmap subtable")

    def _read_cmap4(self, sub):
        data = self.data
        (seg_x2,) = struct.unpack_from(">H", data, sub + 6)
        segs = seg_x2 // 2
        ends = struct.unpack_from(f">{segs}H", data, sub + 14)
        starts = struct.unpack_from(f">{segs}H", data, sub + 16 + seg_x2)
        deltas = struct.unpack_from(f">{segs}h", data, sub + 16 + 2 * seg_x2)
        range_at = sub + 16 + 3 * seg_x2
        ranges = struct.unpack_from(f">{segs}H", data, range_at)
        table = {}
        for i in range(segs):
            for code in range(starts[i], ends[i] + 1):
                if code == 0xFFFF:
                    continue
                if ranges[i] == 0:
                    glyph = (code + deltas[i]) & 0xFFFF
                else:
                    at = range_at + 2 * i + ranges[i] + 2 * (code - starts[i])
                    (glyph,) = struct.unpack_from(">H", data, at)
                    if glyph:
                        glyph = (glyph + deltas[i]) & 0xFFFF
                if glyph:
                    table[code] = glyph
        return table

    def glyph(self, char):
        """The glyph index of a character (0, the missing glyph, when the font has none)."""
        return self.cmap.get(ord(char), 0)

    def advance(self, glyph):
        hmtx = self.tables["hmtx"][0]
        index = min(glyph, self.num_h_metrics - 1)
        return struct.unpack_from(">H", self.data, hmtx + 4 * index)[0]

    def contours(self, glyph, depth=0):
        """The glyph's outline as closed polylines in font units (y up)."""
        if depth > MAX_COMPOSITE_DEPTH:
            raise FontError("composite glyphs nested too deeply")
        if not 0 <= glyph < self.num_glyphs:
            raise FontError(f"glyph {glyph} out of range")
        start, end = self.loca[glyph], self.loca[glyph + 1]
        if start == end:
            return []  # no outline (a space)
        at = self.tables["glyf"][0] + start
        (n,) = struct.unpack_from(">h", self.data, at)
        if n >= 0:
            return self._simple(at, n)
        return self._composite(at + 10, depth)

    def _simple(self, at, n):
        data = self.data
        ends = struct.unpack_from(f">{n}H", data, at + 10)
        points = ends[-1] + 1 if n else 0
        at += 10 + 2 * n
        (instructions,) = struct.unpack_from(">H", data, at)
        at += 2 + instructions
        flags = []
        while len(flags) < points:
            flag = data[at]
            at += 1
            repeat = 1
            if flag & 8:
                repeat += data[at]
                at += 1
            flags.extend([flag] * repeat)
        flags = flags[:points]

        def coords(short_bit, same_bit):
            nonlocal at
            values, value = [], 0
            for flag in flags:
                if flag & short_bit:
                    delta = data[at]
                    at += 1
                    value += delta if flag & same_bit else -delta
                elif not flag & same_bit:
                    value += struct.unpack_from(">h", data, at)[0]
                    at += 2
                values.append(value)
            return values

        xs = coords(2, 16)
        ys = coords(4, 32)
        contours, first = [], 0
        for last in ends:
            pts = [(xs[i], ys[i], bool(flags[i] & 1)) for i in range(first, last + 1)]
            first = last + 1
            if pts:
                contours.append(_flatten(pts))
        return contours

    def _composite(self, at, depth):
        data = self.data
        contours = []
        while True:
            flags, glyph = struct.unpack_from(">HH", data, at)
            at += 4
            if flags & 1:
                arg1, arg2 = struct.unpack_from(">hh", data, at)
                at += 4
            else:
                arg1, arg2 = struct.unpack_from(">bb", data, at)
                at += 2
            if not flags & 2:
                raise FontError("composite glyphs placed by point matching are not supported")
            a, b, c, d = 1.0, 0.0, 0.0, 1.0
            if flags & 8:
                a = d = _f2dot14(data, at)
                at += 2
            elif flags & 0x40:
                a, d = _f2dot14(data, at), _f2dot14(data, at + 2)
                at += 4
            elif flags & 0x80:
                a, b, c, d = (_f2dot14(data, at + 2 * k) for k in range(4))
                at += 8
            for contour in self.contours(glyph, depth + 1):
                contours.append([(a * x + c * y + arg1, b * x + d * y + arg2) for x, y in contour])
            if not flags & 0x20:
                return contours


def _f2dot14(data, at):
    return struct.unpack_from(">h", data, at)[0] / 16384.0


def _flatten(pts):
    """A closed quadratic B-spline contour (on- and off-curve points) as a polyline."""
    # Start on an on-curve point; with none, at the midpoint of the first two off-curve ones.
    start = next((i for i, p in enumerate(pts) if p[2]), None)
    if start is None:
        (x0, y0, _), (x1, y1, _) = pts[0], pts[1 % len(pts)]
        pts = [((x0 + x1) / 2, (y0 + y1) / 2, True)] + pts
        start = 0
    pts = pts[start:] + pts[:start]
    out = [(pts[0][0], pts[0][1])]
    control = None
    for x, y, on in pts[1:] + pts[:1]:
        if on:
            if control is None:
                out.append((x, y))
            else:
                out.extend(_quad(out[-1], control, (x, y)))
            control = None
        else:
            if control is not None:
                mid = ((control[0] + x) / 2, (control[1] + y) / 2)
                out.extend(_quad(out[-1], control, mid))
            control = (x, y)
    return out


def _quad(p0, p1, p2):
    pts = []
    for k in range(1, CURVE_STEPS + 1):
        t = k / CURVE_STEPS
        u = 1 - t
        pts.append((u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]))
    return pts


def fill(polygons, width, height):
    """Coverage (0..1, row-major) of closed polygons in pixel coordinates (y down), non-zero rule."""
    cover = [0.0] * (width * height)
    edges = []
    for poly in polygons:
        for (x0, y0), (x1, y1) in zip(poly, poly[1:] + poly[:1]):
            if y0 != y1:
                edges.append((x0, y0, x1, y1, 1 if y1 > y0 else -1))
    if not edges:
        return cover
    top = max(0, int(math.floor(min(min(e[1], e[3]) for e in edges))))
    bottom = min(height, int(math.ceil(max(max(e[1], e[3]) for e in edges))))
    weight = 1.0 / SUBSAMPLES
    for row in range(top, bottom):
        base = row * width
        for s in range(SUBSAMPLES):
            y = row + (s + 0.5) / SUBSAMPLES
            crossings = []
            for x0, y0, x1, y1, wind in edges:
                if (y0 <= y < y1) or (y1 <= y < y0):
                    crossings.append((x0 + (y - y0) * (x1 - x0) / (y1 - y0), wind))
            crossings.sort()
            winding = 0
            for (xa, wind), (xb, _) in zip(crossings, crossings[1:]):
                winding += wind
                if winding:
                    _span(cover, base, width, xa, xb, weight)
    return cover


def _span(cover, base, width, xa, xb, weight):
    """Add `weight` times the covered fraction of each pixel in [xa, xb) of one row."""
    xa, xb = max(xa, 0.0), min(xb, float(width))
    if xb <= xa:
        return
    first, last = int(xa), min(int(math.ceil(xb)) - 1, width - 1)
    if first == last:
        cover[base + first] += weight * (xb - xa)
        return
    cover[base + first] += weight * (first + 1 - xa)
    for px in range(first + 1, last):
        cover[base + px] += weight
    cover[base + last] += weight * (xb - last)


def text_width(font, text, size):
    return sum(font.advance(font.glyph(ch)) for ch in text) * size / font.units_per_em


def text_polygons(font, text, size, x, baseline):
    """The outlines of a line of text at `size` pixels per em, its baseline at `baseline`."""
    scale = size / font.units_per_em
    polygons = []
    for ch in text:
        glyph = font.glyph(ch)
        for contour in font.contours(glyph):
            polygons.append([(x + px * scale, baseline - py * scale) for px, py in contour])
        x += font.advance(glyph) * scale
    return polygons, x


class Canvas:
    """An opaque RGB picture that shapes are painted over, anti-aliased by their coverage."""

    def __init__(self, width, height, background):
        self.width, self.height = width, height
        self.rgb = [list(background) for _ in range(width * height)]

    def paint(self, polygons, color):
        """Paint polygons in an sRGB colour (r, g, b bytes)."""
        cover = fill(polygons, self.width, self.height)
        for i, c in enumerate(cover):
            if c > 0.0:
                c = min(c, 1.0)
                self.rgb[i] = [d + (s - d) * c for d, s in zip(self.rgb[i], color)]

    def png(self):
        rows = []
        for y in range(self.height):
            row = bytearray(b"\0")  # filter: none
            for px in self.rgb[y * self.width:(y + 1) * self.width]:
                row.extend(min(255, int(round(v))) for v in px)
            rows.append(bytes(row))
        return png(self.width, self.height, b"".join(rows))


def png(width, height, scanlines):
    """An 8-bit RGB PNG from filtered scanlines (each row prefixed by its filter byte)."""

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(scanlines, 9)) + chunk(b"IEND", b"")


# The caption bar's look: the app's dark panel colour, the tool name in its accent blue.
BAR = (22, 26, 34)
TOOL = (110, 168, 255)
TEXT = (236, 239, 244)
SEPARATOR = "  \u00b7  "


def caption_bar(font, size, tool, detail, font_size=22):
    """A caption bar of `size` (width, height) pixels: the MCP tool's name, then what it does,
    centred."""
    width, height = size
    rest = SEPARATOR + detail if detail else ""
    text_w = text_width(font, tool + rest, font_size)
    canvas = Canvas(width, height, BAR)
    # The baseline centres the font's cap height (about 0.73 em for Inter) in the bar.
    baseline = height / 2 + 0.73 * font_size / 2
    polygons, x = text_polygons(font, tool, font_size, (width - text_w) / 2, baseline)
    canvas.paint(polygons, TOOL)
    if rest:
        polygons, _ = text_polygons(font, rest, font_size, x, baseline)
        canvas.paint(polygons, TEXT)
    return canvas.png()
