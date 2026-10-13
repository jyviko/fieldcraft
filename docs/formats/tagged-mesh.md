# Face-tagged mesh exchange (3MF with face tags)

Version 1 (2026-10-13). Namespace `https://github.com/jyviko/modelcraft/formats/face-tags/2026/10`.

The second way geometry enters ModelCraft, beside opening a `.solidcraft` design: a triangulated
boundary surface written by another program, each triangle tagged with the B-rep face it lies on.
ModelCraft reads it into a `SurfaceMesh` (`modelcraft_geometry::tagged_mesh`) with no kernel
involved. SolidCraft writes it (`file.export` with `format: "3mf-tagged"`).

## Decision: 3MF Core plus one optional attribute

The file is an ordinary **3MF package** (3MF Consortium, *3D Manufacturing Format Core
Specification* 1.3, publicly available). The face tags are one attribute in a namespace of our
own, which the Core specification allows on `<triangle>` and `<object>` and which any 3MF consumer
ignores unless the namespace is listed in `requiredextensions` (it is not).

Why not a small binary format of our own (versioned header, f64 positions, u32 triangles, u32
tags):

- **It is already written and read.** SolidCraft exports and imports 3MF with hostile-input caps;
  slicers, viewers and other CAD programs write it too. A tagged file opens in all of them, and
  ModelCraft reads untagged 3MF from those programs through the same code ("geometry from other
  tools later").
- **One specification to maintain, published by a consortium**, not a byte layout we would have to
  version, document and keep two implementations of in step.
- **Nothing is lost.** Coordinates are decimal text; written in shortest round-trip form (Rust's
  `{}` for `f64`) every `f64` comes back bit for bit. Triangles and tags are integers.
- The costs (a zip and an XML parser, larger files, slower parsing than raw bytes) are acceptable
  for an exchange file. Both libraries (`zip`, `quick-xml`) are MIT, pure Rust, build for wasm and
  are the ones SolidCraft already uses.

## The package

As 3MF Core: a zip holding `[Content_Types].xml`, `_rels/.rels` (a relationship of type
`http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel` to the model part) and the model
part, conventionally `3D/3dmodel.model`.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US"
       xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
       xmlns:ft="https://github.com/jyviko/modelcraft/formats/face-tags/2026/10">
 <metadata name="Application">SolidCraft</metadata>
 <resources>
  <object id="2" type="model" name="Body1" ft:faces="6">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/>
     <vertex x="10" y="0" z="0"/>
     ...
    </vertices>
    <triangles>
     <triangle v1="0" v2="2" v3="1" ft:face="0"/>
     ...
    </triangles>
   </mesh>
  </object>
 </resources>
 <build>
  <item objectid="2"/>
 </build>
</model>
```

### Face tags

| Attribute | On | Value | Meaning |
|---|---|---|---|
| `ft:faces` | `<object>` | integer ≥ 1 | Number of B-rep faces of the body. Optional; when present every `ft:face` of the object is below it. Faces without triangles still count. |
| `ft:face` | `<triangle>` | integer ≥ 0 | The B-rep face (0-based, the body's own face order) the triangle lies on. |

An object's triangles are either **all tagged or none**. A body whose triangles carry no tags reads
as one face (tag 0). A file mixing tagged and untagged triangles in one object is rejected.

### Writer rules (version 1)

- `unit="millimeter"`, Z up.
- One `<object type="model">` with a `<mesh>` per body, named after the body, one `<item>` per
  object, no `transform`, no components.
- Vertices shared within an object (coincident positions merged), triangles counter-clockwise
  seen from outside, no degenerate triangles; coordinates in shortest round-trip decimal form.
- Colours, if written, use 3MF's own `basematerials` (`pid`/`p1`); they are independent of the tags.
- Deterministic bytes: fixed part order (`[Content_Types].xml`, `_rels/.rels`, the model),
  fixed zip timestamps, no hash-ordered output.

### Reader rules (ModelCraft)

- Every build item becomes one `SurfaceMesh`, in millimetres (`unit` converted), named after its
  object (`Mesh<n>` when unnamed). An item's `transform` is applied; a mirroring transform reverses
  the winding; a singular transform (one that flattens the object) is refused. Components are
  not read yet (an error that says so).
- Untrusted input: file ≤ 512 MB, a part ≤ 1 GB uncompressed, ≤ 10 000 zip entries, ≤ 20 million
  vertices and 20 million triangles in total, XML nesting ≤ 256. Coordinates must be finite with
  magnitude ≤ 10⁹ mm (the FE model's limit), in the file and once placed; vertex indices must exist; tags must be below `ft:faces` when it is given.
  Any violation is an error naming the problem; nothing panics.

## Versions

A later version gets a new namespace (`…/face-tags/<year>/<month>`); readers accept the versions
they know and treat an unknown face-tag namespace as untagged.
