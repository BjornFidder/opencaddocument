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

DWG and DXF reading and writing come from
[acadrust](https://github.com/HakanSeven12/opencadcodec), a fork of the
upstream acadrust crate.
