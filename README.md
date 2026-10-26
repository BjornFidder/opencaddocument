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
- A converter writing a non-native format writes the snapped placement. After
  the first save, saving again produces the same bytes.

With the `ifcx` feature, the `Ifcx` format reads and writes IFCX through
[ifccad](https://github.com/OpenAEC-Foundation/ifccad):
- ifccad only reads and writes the files; the mapping to `Document` lives here,
  so neither library depends on the other.
- Layers, text styles, block definitions and model-space lines, circles, arcs,
  ellipses, points, 2D and 3D polylines, 2D solids, traces, single-line texts
  and block inserts map both ways. Everything else (MText, dimensions,
  hatches, paper layouts, attributes, non-continuous linetypes) is reported as
  a loss.
- An IFCX file's identity (header id, drawing id) rides on custom document
  properties, so a drawing read from IFCX is written back as the same drawing.

DWG and DXF reading and writing come from
[acadrust](https://github.com/HakanSeven12/acadrust), a fork of the
upstream acadrust crate.
