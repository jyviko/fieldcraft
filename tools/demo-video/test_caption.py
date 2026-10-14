"""Tests of the caption pictures: `python3 -m unittest` in tools/demo-video."""

import struct
import unittest
import zlib
from pathlib import Path

import caption

FONT = Path(__file__).resolve().parents[2] / "crates" / "ui-egui" / "fonts" / "Inter-SemiBold.ttf"


def decode(data):
    """Width, height and RGB rows of a PNG written by `caption.png` (filter 0, one IDAT)."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    at, chunks = 8, {}
    while at < len(data):
        (length,) = struct.unpack_from(">I", data, at)
        tag, body = data[at + 4:at + 8], data[at + 8:at + 8 + length]
        (crc,) = struct.unpack_from(">I", data, at + 8 + length)
        assert crc == zlib.crc32(tag + body) & 0xFFFFFFFF, tag
        chunks[tag] = body
        at += 12 + length
    width, height, depth, kind = struct.unpack_from(">IIBB", chunks[b"IHDR"])
    assert (depth, kind) == (8, 2)
    raw = zlib.decompress(chunks[b"IDAT"])
    stride = 1 + 3 * width
    rows = [raw[y * stride + 1:(y + 1) * stride] for y in range(height)]
    assert all(raw[y * stride] == 0 for y in range(height))
    return width, height, rows


class Fill(unittest.TestCase):
    def test_square_covers_its_pixels_and_partial_edges(self):
        cover = caption.fill([[(1, 1), (3.5, 1), (3.5, 3), (1, 3)]], 5, 4)
        at = lambda x, y: cover[y * 5 + x]
        self.assertAlmostEqual(at(1, 1), 1.0)
        self.assertAlmostEqual(at(2, 2), 1.0)
        self.assertAlmostEqual(at(3, 1), 0.5)
        self.assertAlmostEqual(at(0, 1), 0.0)
        self.assertAlmostEqual(at(4, 2), 0.0)
        self.assertAlmostEqual(at(1, 0), 0.0)
        self.assertAlmostEqual(sum(cover), 2.5 * 2)

    def test_nonzero_winding_fills_overlaps_once_and_leaves_holes(self):
        outer = [(0, 0), (6, 0), (6, 6), (0, 6)]
        hole = [(2, 2), (2, 4), (4, 4), (4, 2)]  # opposite direction
        cover = caption.fill([outer, hole], 6, 6)
        self.assertAlmostEqual(cover[3 * 6 + 3], 0.0)
        self.assertAlmostEqual(cover[1 * 6 + 1], 1.0)
        twice = caption.fill([outer, outer], 6, 6)
        self.assertAlmostEqual(max(twice), 1.0)

    def test_shapes_outside_the_picture_are_clipped(self):
        cover = caption.fill([[(-5, -5), (20, -5), (20, 20), (-5, 20)]], 3, 2)
        self.assertEqual(cover, [1.0] * 6)


class Png(unittest.TestCase):
    def test_round_trip(self):
        canvas = caption.Canvas(3, 2, (0, 0, 0))
        canvas.paint([[(1, 1), (2, 1), (2, 2), (1, 2)]], (255, 0, 0))
        canvas.paint([[(2, 1), (2.5, 1), (2.5, 2), (2, 2)]], (0, 0, 255))  # half a pixel
        width, height, rows = decode(canvas.png())
        self.assertEqual((width, height), (3, 2))
        self.assertEqual(rows[0], bytes(9))
        self.assertEqual(rows[1], bytes((0, 0, 0, 255, 0, 0, 0, 0, 128)))


class Font(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.font = caption.Font.load(FONT)

    def test_metrics(self):
        self.assertGreater(self.font.units_per_em, 0)
        self.assertGreater(self.font.advance(self.font.glyph("m")), self.font.advance(self.font.glyph("i")))
        self.assertEqual(self.font.contours(self.font.glyph(" ")), [])

    def test_outlines_have_their_holes(self):
        size = 100
        polygons, _ = caption.text_polygons(self.font, "o", size, 0, size)
        self.assertEqual(len(polygons), 2)
        cover = caption.fill(polygons, size, size)
        xs = [p[0] for p in polygons[0]]
        ys = [p[1] for p in polygons[0]]
        cx, cy = int((min(xs) + max(xs)) / 2), int((min(ys) + max(ys)) / 2)
        self.assertAlmostEqual(cover[cy * size + cx], 0.0)  # the counter
        self.assertGreater(cover[cy * size + int(min(xs)) + 3], 0.99)  # the stroke

    def test_composite_glyphs(self):
        # An accented letter is a base glyph plus a mark placed by an offset.
        e = self.font.contours(self.font.glyph("e"))
        e_acute = self.font.contours(self.font.glyph("é"))
        self.assertGreater(len(e_acute), len(e))
        self.assertGreater(max(y for c in e_acute for _, y in c), max(y for c in e for _, y in c))

    def test_hostile_data_is_refused(self):
        with self.assertRaises(caption.FontError):
            caption.Font(b"")
        with self.assertRaises(caption.FontError):
            self.font.contours(self.font.num_glyphs)


class CaptionBar(unittest.TestCase):
    def test_text_is_centred_on_the_bar(self):
        font = caption.Font.load(FONT)
        width, height, rows = decode(caption.caption_bar(font, (400, 48), "solve", "static", font_size=20))
        self.assertEqual((width, height), (400, 48))
        bar = bytes(caption.BAR)
        text = [(x, y) for y, row in enumerate(rows) for x in range(width) if row[3 * x:3 * x + 3] != bar]
        xs, ys = [p[0] for p in text], [p[1] for p in text]
        self.assertLessEqual(abs((min(xs) + max(xs)) / 2 - 200), 1.5)
        self.assertLessEqual(abs((min(ys) + max(ys)) / 2 - 24), 1.5)
        self.assertIn(bytes(caption.TOOL), {row[3 * x:3 * x + 3] for row in rows for x in range(width)})


if __name__ == "__main__":
    unittest.main()
