# IFCX placement numerics

The IFCX mapping writes the directly calculated world origin and perpendicular
axes of a CAD placement. It does not automatically quantize that placement's
OCS origin, normal or rotation. The inverse mapping uses the existing shared
`Placement` conversion. Public APIs and the format contract are unchanged.

This matters at the arbitrary-axis algorithm's reference-axis boundaries:
rounding a normal across `abs(nx) < 1/64 && abs(ny) < 1/64` changes the OCS
basis. Reinterpreting the old OCS origin and angle in that new basis can move
geometry by metres. A drawing that becomes byte-stable after that first save
can still be geometrically wrong.

`Placement::snapped(position, angle)` remains an explicit OCS-parameter
quantizer. Its arguments are not bounds on world-space displacement. Default
IFCX conversion does not call it, and repeated conversion does not promise
byte-identical frames.

## Signed INSERT scales

The CAD model's INSERT setters currently replace scales whose absolute value
is below `1e-12` with positive `1e-12`. IFCX permits smaller nonzero scales.
The mapping compares every getter's bits with its offered source value after
setting all three components. A replacement omits the instance with a scale
loss diagnostic; it never silently changes magnitude or handedness. Ordinary
representable signed and nonuniform scales pass through exactly. This check
also participates in the existing atomic dimension-picture preparation: an
unrepresentable marker omits the whole picture and retains the definition.

## Conversion policy

The IFCX mapping no longer calls `snap`, `settle` or `Placement::snapped`.
Required binary64 calculations for frames, ellipse axes, text lengths, MText
spacing/oblique units, Hatch pattern factoring/sweeps and plot units pass their
results directly to the destination. Discrete colour and line-weight encoding,
angle periodicity and stored user snap settings keep their existing meaning.
Only exactly coincident Hatch arc ends mean a full turn; tiny nonzero sweeps
remain partial arcs.

Dimension pictures follow the same finite arithmetic policy. Their frames use
shared `IfccadPlacement::coordinate_frame()` validity and the original validated
axes. A separate absolute residual/extent budget is not an acceptance condition.
Valid finite results, including collapsed lines accepted by ordinary geometry,
can be retained. Invalid output and unsupported content still omit the whole
picture. See [stored dimension graphics](ifcx-dimension-graphics.md).

This removes avoidable quantization, not the rounding inherent in binary64.
It introduces no runtime accuracy guarantee or general overflow repair. A +Z
local dimension can live inside an obliquely placed ordinary block; the picture
profile does not gain tilted dimension planes or new entity families.

## Regression evidence and limits

`tests/ifcx_placement_3d.rs` checks native-first and CAD-first source geometry
before the first conversion, every first output and three successive cycles.
Original-source error and additional drift since the first output use separate
comparisons with the same case budget; the budget does not accumulate per cycle.
Every native result goes through the production reader with `RejectConflicts`
and strict drawing validation. Byte equality is not the acceptance criterion.

The public, synthetic cases cover positive and negative X/Y AA boundaries,
their adjacent binary64 values, both poles and near-poles, oblique frames,
separate frame/instance rotation, up to three block levels, nonzero block base
points, mirrors on each axis, nonuniform scales, scales `0.001` and `1000`, and
physically equivalent millimetre/centimetre/metre/inch drawings. An additional
cross-unit comparison checks the ordinary geometry in millimetres. The line is 40 m,
and the scale-1000 case reaches 40 km. Terrain cases use large translated
coordinates. Model and paper occurrences share one block definition.

World observations include line endpoints, full-circle affine conics, arc
coefficients and sweep, dimension reference/line points, and an ordered picture
with a line, circle, point and signed nonuniform marker with a nonzero base.
All four physically equivalent unit cases contain the picture, including
fractional marker coordinates. Three additional wide cases have eleven ordered
parts: the original four plus Arc, Ellipse, bulged planar polyline, spatial
polyline, Solid, Text and MText. They include ordinary counterparts with the same
geometry and an unbraced MText source. World checks include curve interiors and
text anchors, direction and height; they do not evaluate glyph outlines.
The parent block transform is applied once to the definition and once to the
picture geometry. Picture attachment, part order, appearance and style binding
are also checked. Matching is by scope and kind; missing or ambiguous content
fails. Transport checks use actual ASCII DXF, binary DXF and AC1032 DWG writers
and readers for AA, nested, and wide/MText cases, plus shared model/paper
ownership. Reader statistics must show completed streams without skipped records
or recovered errors. This is a focused correctness suite, not a practice-file benchmark.

`tests/fixtures/numerics-3d-cases.json` freezes the binary64 inputs.
`scripts/generate_numerics_3d_reference.py` evaluates them using standard-library
Decimal arithmetic at 100 digits, without production placement calls. The
oracle records the input SHA-256. At a discrete boundary the recorded reference
axis follows binary64 normalization of the source normal; continuous geometry
is then evaluated at high precision. Native and CAD source references are
calculated separately. The Rust observer independently walks actual block
ownership and geometry; it does not use `Placement` or CAD transform helpers.

Full circles allow a common parameter-phase change. Center error plus conic
coefficient norm bounds the corresponding point error; arcs also account for
sweep change. These are high-precision numerical references, not outward-rounded
interval certificates. Test budgets are `1e-6 mm` for bounded building/AA cases,
`1e-5 mm` for terrain/nested cases and `1e-4 mm` for the 40 km case. They are
test-specific budgets, not a new runtime tolerance or a general error guarantee.
Glyph contours, arbitrary nesting and arbitrarily large coordinates are not
certified by this suite.

To regenerate the oracle manually after deliberately changing frozen inputs:

```text
python scripts/generate_numerics_3d_reference.py
cargo test --locked --features ifcx --test ifcx_placement_3d
```

The generator is not a build or CI step. `--init` intentionally recreates the
input cases as well; ordinary regression runs only read the committed fixtures.
