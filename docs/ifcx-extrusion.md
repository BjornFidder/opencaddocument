# Development extrusion conversion coverage

This contribution targets Document main
be6a771679a712f25174d84b90bf8b0a3fb3f5fa and pins the native IFCCAD core to
07e1ef1c4e975afcab0a07012bed8a82a56202cb. Its contract is development profile
0.3.0 / CBOR mapping 3; optional Zstd transport framing remains revision 1.
The codec follows current Document main at
0a5542baa6052a2b07fc700b7ba041549650a997. No local dependency patch is required.
The IFCCAD commit is on a task branch, not a new tagged release.

Signed thickness is carried in both directions for POINT, LINE, CIRCLE, ARC
and planar polylines, including representable saved Dimension graphics.
POINT retains its WCS location. LINE endpoints stay in scope coordinates;
its extrusion direction is independent of the chord and rotates as a vector
under saved-picture placement. Source normals are validated and normalized
at conversion; native storage does not silently normalize directions.

Ordinary unfitted classic 2D POLYLINE maps to PlanarPolyline. Conversion retains
elevation, bulges, closure, pattern generation, thickness and the codec's
effective numeric vertex-width pairs, including zero and dormant final pairs.
DXF may already have resolved defaults into these numbers; DWG numeric leaves
are used literally. Numeric zero is not guessed to mean inheritance.

The import diagnostic states that original default/override presence is
unavailable or unassessed in the codec model. Export uses an equivalent
LWPOLYLINE rather than preserving classic record structure. Native Defaulted
widths are materialized to effective pairs for CAD, with a separate diagnostic.
These diagnostics do not claim an exhaustive preservation audit. Fitted/mesh
and unsupported vertex detail are diagnosed omissions; this slice does not
separately qualify classic paths inside saved picture graphs.

Focused regression sources and the grouped IFCCAD gallery exercise actual
AC1032 ASCII DXF and DWG production write/read, followed by strict native
JSON/CBOR readback. Native tests retain exact width modes and signed-zero bits;
CAD checks compare effective widths and geometric sweep semantics. This is
synthetic correctness evidence, not practice-corpus, pixel, editing-history or
all-version qualification. Review files and the matching local Explorer have
been accepted. A bounded LINE edit/grip/undo smoke is separate application
evidence, not general Document editing qualification. This contribution uses an exact reachable native revision. Upstream review and
IFCCAD/Explorer release or deployment remain separate stages.
