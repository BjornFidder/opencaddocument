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
- The IFCX mapping covers layers, line patterns with their
  SHX and text symbols, text styles, dimension styles, UCSs, the point
  display, block definitions, paper layouts with their sheet and plot
  settings, viewports, model windows and the workspace state, and lines,
  circles, arcs, ellipses, points, 2D and 3D polylines, solids, traces,
  texts, MText, hatches, linear and aligned dimensions and block inserts.
- Stored dimension graphics transfer in both directions using native CAD
  picture blocks within a numerically bounded planar profile. Unsupported pictures are
  omitted as a whole with a loss report; the supported definition is retained
  independently. No graphics are regenerated. See the
  [picture coverage and preservation limits](docs/ifcx-dimension-graphics.md).
- What IFCX has no place for is reported as a loss: other dimension kinds,
  splines, 3D solids and the like, raster sheets, attributes.
- An IFCX file's identity (header id, drawing id) rides on custom document
  properties, so a drawing read from IFCX is written back as the same drawing.

DWG and DXF reading and writing come from
[acadrust](https://github.com/HakanSeven12/acadrust), a fork of the
upstream acadrust crate.
