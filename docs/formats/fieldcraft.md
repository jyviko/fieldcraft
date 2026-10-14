# The `.fieldcraft` file (format `fieldcraft/1`)

A `.fieldcraft` file holds one model: its definition, its mesh, the geometry it is built on and
the commands that built it. It is written and read by `fieldcraft-io` (`write`, `read`; bytes in,
bytes out, so the browser build uses the same code) and by the commands `file.save`,
`file.save_as` and `file.open`.

## Package

The file is a ZIP package with exactly four parts, in this order, all stored uncompressed (so any
unzip tool reads them: `unzip -p bracket.fieldcraft model.json`):

| Part | Content |
|---|---|
| `fieldcraft.json` | Manifest: `format` (`"fieldcraft/1"`), `generator` (`"FieldCraft 0.5.5"`), `geometry`, and `parts`: the byte size and SHA-256 of each other part. |
| `model.json` | The model definition: unit system, sets, materials, sections, reference points, couplings, boundary conditions, loads and steps. Pretty-printed JSON with a fixed key order (maps sorted by name), so a change shows as a line diff. Its form is the model's own JSON form, which commands take as parameters. |
| `mesh.bin` | Nodes, elements and geometry tags, binary (below). |
| `journal.json` | The commands that built the model, `[{"command": id, "params": {...}}, ...]`, one per line. It is a script `fieldcraft-cli run` replays. |

The same model gives the same bytes on every platform: entry times are fixed (1980-01-01),
permissions are fixed, and every map is written in sorted order. To diff two versions with git,
add `*.fieldcraft diff=fieldcraft` to `.gitattributes` and
`git config diff.fieldcraft.textconv "unzip -p"`; the definition and the journal show line by
line, the mesh part as binary.

## Geometry

```json
"geometry": {
  "design": {"path": "plate.solidcraft", "sha256": "…"},
  "meshes": [{"path": "/parts/tet.3mf", "sha256": "…"}]
}
```

`design` is the SolidCraft design (`geometry.open`), `meshes` the face-tagged 3MF files
(`geometry.import.mesh`) in import order. Each records the SHA-256 of the bytes that were opened.
Paths inside the model file's folder are stored relative to it (with `/`), others absolute.
Opening the model opens each file again: a missing, unreadable or changed file (its hash differs)
is reported as a warning and the model still opens; sets bind to geometry names, so a remesh
brings the mesh in line with a changed design. A geometry file that could not be opened keeps its
reference, so saving again does not lose it.

## `mesh.bin`

Little-endian:

```
"MCMESH01"                                    magic (the mesh encoding's version is 01)
u64 nodes, u64 elements, u64 entities, u64 tagged elements, u64 tagged nodes
nodes      × { u32 label, f64 x, f64 y, f64 z }             labels strictly ascending
elements   × { u32 label, u8 kind, u32 node × n(kind) }     labels strictly ascending
entities   × { u8 kind (0 body, 1 face, 2 edge, 3 vertex),
               u16 length, body name, [u16 length, entity name] unless a body }   UTF-8
tagged elements × { u32 element, u32 region (0xFFFFFFFF none),
                    u8 n, n × { u8 face, u32 tag }, u8 n, n × { u8 edge, u32 tag } }
tagged nodes × { u32 node, u32 tag }
```

Element kind codes: 1 rod2, 2 beam2, 3 tri3, 4 tri6, 5 quad4, 6 quad8, 7 tet4, 8 tet10, 9 wedge6,
10 hex8, 11 hex20. Node order is the model's (Abaqus/VTK). A tag indexes the entity list, which
lists each geometry entity once. Coordinates keep every bit (signed zero included).

## Reading a file safely

Every file is treated as hostile:

- The file is at most 8 GiB; the manifest, definition and journal parts at most 256 MiB each. No
  allocation is larger than the input.
- Only the four parts, each once. The ZIP CRC and the manifest's size and SHA-256 are checked for
  every part.
- A `format` other than `fieldcraft/1` is refused: a later `fieldcraft/N` as a version error, any
  other value as not a FieldCraft file. Unknown fields are errors.
- `mesh.bin`: counts are capped (50 million nodes and elements, one million entities) and checked
  against the bytes present before anything is read; labels must ascend; unknown kinds and
  trailing bytes are refused.
- The model is rebuilt through the same checked methods commands use: names, references,
  finite coordinates within ±1e9, element connectivity, tag dimensions. A file cannot hold what
  commands would refuse.

## Saving

`file.save` writes the bytes to a temporary file beside the target, flushes it, keeps the old
file as `<file>.bak`, renames the new file over the target and flushes the folder: a crash at any
moment leaves the old file or the new one. A path that exists and is not a regular file is
refused.

## Autosave and recovery

The desktop app autosaves every 30 seconds while there are unsaved changes, into the recovery
folder (`FIELDCRAFT_RECOVERY_DIR`, else `FieldCraft/recovery` in the per-user data folder):
`<id>.fieldcraft`, `<id>.json` (the model's own path, time, counts) and `<id>.lock`, held while
the app runs. Saving (or undoing back to the saved state) drops the autosave. After a crash, the
next launch reports the entries; `file.recovery.list` lists them, `file.recover` opens one as
unsaved work under its original path, and `file.recovery.discard` deletes them.
