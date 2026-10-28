# opencaddocument

The shared drawing model of the OpenCAD projects.

The model is DWG-centred. `Document` holds a drawing the way DWG describes it,
and other formats convert to and from it through the `Format` contract (read,
write, loss report).

Where formats describe the same concept differently, the model keeps one
representation and converts at the edge. `Placement` is the first example:
- It stores the DWG form (OCS origin, normal, rotation), so DWG values pass
  through unchanged.
- Other forms, such as an origin with X and Y axes, are converted.
- The IFCX converter writes the directly calculated frame. Explicit
  `Placement::snapped` quantizes OCS parameters and can change world geometry
  substantially near an arbitrary-axis boundary. Repeated conversion does not
  promise byte-identical output; see [placement numerics](docs/ifcx-numerics.md).

With the `ifcx` feature, the `Ifcx` format reads and writes IFCX through
[ifccad](https://github.com/OpenAEC-Foundation/ifccad):
- ifccad only reads and writes the files; the mapping to `Document` lives here,
  so neither library depends on the other.
- The IFCX mapping covers layers, line patterns with their
  SHX and text symbols, text styles, dimension styles, UCSs, the point
  display, block definitions, paper layouts with their sheet and plot
  settings, viewports, model windows and the workspace state, and lines,
  circles, arcs, ellipses, points, 2D and 3D polylines, solids, traces,
  texts, MText, hatches, all nine ordinary dimension kinds and block inserts.
  The development Dimension expansion uses IFCCAD profile 0.2.0/CBOR2; see
  [qualified kinds and limitations](docs/ifcx-dimension-kinds.md).
- Stored dimension graphics transfer in both directions using native CAD
  picture blocks within a supported planar profile. Unsupported pictures are
  omitted as a whole with a loss report; the supported definition is retained
  independently. Retained numerical observations are explicitly identified in the
  diagnostic list. No graphics are regenerated. See the
  [picture coverage and preservation limits](docs/ifcx-dimension-graphics.md).
- What IFCX has no place for is reported as a loss: unsupported dimension state,
  splines, 3D solids and the like, raster sheets, attributes.
- An IFCX file's identity (header id, drawing id) rides on custom document
  properties, so a drawing read from IFCX is written back as the same drawing.

DWG and DXF reading and writing come from
[acadrust](https://github.com/HakanSeven12/acadrust), a fork of the
upstream acadrust crate.

## Logical IFCCAD conversion

With `ifcx` enabled, `Ifcx::import_ifccad` and `Ifcx::export_ifccad` convert
directly between `ifccad::IfccadDocument` and `Document`. They share the file
adapter's mapping, rounding and loss reporting; no JSON serialization/parsing
is needed between the models. Import validates its input and export validates
its result. The `ifccad` dependency is re-exported so callers can use the exact
same model types without a separate dependency revision.

```rust
use opencaddocument::{ifccad, Document, Ifcx};

let source = Document::new();
let exported = Ifcx.export_ifccad(&source).unwrap();
let imported = Ifcx.import_ifccad(&exported.drawing).unwrap();
// Inspect exported.losses and imported.losses for unsupported content.

// Serialize only when bytes are needed for a file, download or preview.
let encoded = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
```

The outcomes include `entity_ids` (source CAD handle to exported IFCCAD ID)
and `entity_handles` (source IFCCAD ID to imported CAD handle). These cover
converted entity definitions in Model, Paper and shared blocks. Omitted
entities and generated dimension-picture primitives have no separate entry.
The maps describe one conversion, not stable identities across edits or an
expanded list of block occurrences. The typed route converts the logical
drawing only; it does not preserve or audit an enclosing IFCX source graph.

`Format::read` and `Format::write` remain the IFCX byte-level entry points and
delegate to these same conversion methods.
