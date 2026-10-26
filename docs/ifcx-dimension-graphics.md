# Stored dimension graphics

The IFCX mapping transfers eligible stored Dimension pictures in both directions
through the existing `DimensionBase.block_name` link to a native CAD BLOCK.
No extra INSERT is added to the owning scope. The picture is independent of the
linear/aligned definition mapping; additional definition kinds can use this
picture path without a new storage model. This contribution adds no definition
kinds, regeneration, freshness assessment, association repair or metadata store.

## Qualified profile

A picture transfers only when every ordered part and its complete reachable
marker-block closure qualifies. Preparation is atomic: an unsupported part
omits the whole picture with a reason, without leaving partial picture resources.
The independently supported definition remains. Malformed block/layer
references, cycles and nonfinite recognized geometry omit the picture the same
way, with the problem as the reason, so one bad picture never fails the whole
read or write. This is a picture conversion boundary, not an
audit of every unsupported CAD entity or arbitrary object in a Document.

The initial profile uses canonical, unrotated +Z frames. Coordinate translations
must be exactly representable (checked with error-free TwoSum); the inverse
typed conversion must retain the canonical geometry and appearance exactly.
There is no new epsilon, snapping allowance or undeclared displacement budget.
The converter can factor an equivalent local origin into coordinates, so exact
field layout and local frame origin need not survive. Qualification does not
establish a numerical budget for every possible later block occurrence transform.

| Part family | Coverage within that profile |
| --- | --- |
| Line, display Point | Exact coordinates and representable appearance |
| Circle, Arc, Ellipse, EllipseArc | Exact typed inverse; +Z frames |
| Planar polyline | Bulges and effective widths; translations factored into vertices |
| Spatial polyline | Exact vertices and retained appearance |
| Solid, Trace | Exact corners, placement and thickness |
| Text | Representable runs/layout; fitted and aligned Text excluded |
| MText | Representable runs, layout and wrapping |
| Marker block instance | Complete qualified closure; shared definitions retained |
| Hatch, other kinds, tilted/rotated frames | Whole-picture loss |

An otherwise familiar family can still fail the inverse qualification, for
example because of text normalization, angular snapping or coordinate rounding.
Unsupported CAD common state, root/marker record metadata, external block
references and external SORTENTSTABLE draw order are also outside this profile.
Explicit part order is retained; an alternate external draw order is not guessed.

## Presence and metadata

Absent graphics remain absent, without a graphics loss. Present-empty graphics
use a linked empty block and remain present-empty. Nonempty graphics retain
ordered parts. A regular INSERT referencing the same block remains an ordinary
entity; private picture ownership does not suppress that use or rely on a name
prefix. Model, Paper and block-definition owner scopes are supported.

CAD pictures have no IFCCAD Generated provenance flag. Importing Generated
graphics reports that metadata loss; pictures captured from CAD are
SourceSnapshot. No claim is made that the definition and picture agree.

Missing native part layers inherit through CAD layer 0. Explicit layer 0 is
accepted only in the qualified Model context where its effective binding is the
same; the loss report records the change to inheritance. In other contexts the
whole picture is omitted. ReferencePoint helpers qualify only with an explicit
Defpoints binding; arbitrary non-displaying helper semantics are not fabricated
from ordinary Points. A display Point on Defpoints is outside the profile.

Defaulted polyline widths materialize their effective widths and report loss of
default/override authoring metadata. All-zero per-vertex widths can factor to a
constant zero width with a presence loss. MText `Some(0)` wrapping can factor to
`None` with a presence loss. These are diagnosed independently of geometry.
Ordinary CAD handles and redundant binding/orientation fields are transport
details, not preservation side channels.

The original graphics fixture includes unrepresentable helper/layer semantics.
It intentionally still exercises a whole-picture loss; it is not silently
rewritten into a supposedly equivalent visible-only picture.

## Verification boundary

Tests exercise both directions, complete resource closures, absence/empty states,
scope placement, helper/layer binding, width metadata and atomic failure. Selected
composite pictures also pass actual ASCII DXF, binary DXF and AC1032 DWG output
and production readback before strict IFCX validation. This does not establish
coverage for every DWG version, every host renderer or representative practice
drawings. No size/exchange measurement is part of this contribution.
