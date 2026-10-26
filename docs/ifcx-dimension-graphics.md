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

The dimension plane remains +Z, with default picture clone state and a zero
picture-block base point. Rigid local/owner preparation also handles in-plane
dimension rotations and qualified orthonormal member frames. Each translation
or matrix evaluation has a hard absolute budget of **1e-7 drawing units**;
this is not a physical-unit or origin-relative tolerance. TwoSum and FMA product
residuals bound arithmetic independently of the inverse check: exact inverse
cancellation is insufficient evidence. Nonfinite results, excessive error and
non-orthonormal frames still omit the complete picture.

The inverse checks are typed. Listed coordinate fields may round within that
budget; axis changes are limited to **1e-12** and their effect at the primitive
radius, corners or stored text layout metrics must also fit the position budget.
Rounded frame arithmetic for paths, Text or marker closures without an extent
qualification remains unsupported. All other state stays exact after explicit
equivalent-representation factoring. Some equivalent frames can consequently
still be conservatively refused. These checks do not certify text glyph contours
or every later block occurrence transform. Tilted dimension planes and nondefault
picture clone transforms remain outside the profile in both directions.

| Part family | Coverage within that profile |
| --- | --- |
| Line, display Point | Bounded coordinates and representable appearance |
| Circle, Arc, Ellipse, EllipseArc | Typed inverse qualification; bounded eligible frame differences |
| Planar polyline | Bulges and effective widths; translations factored into vertices |
| Spatial polyline | Exact vertices and retained appearance |
| Solid, Trace | Exact corners, placement and thickness |
| Text | Representable runs/layout; fitted and aligned Text excluded |
| MText | Representable runs, layout and wrapping; qualified rotated directions and bounded line-spacing conversion |
| Marker block instance | Complete qualified closure; shared definitions retained |
| Hatch, other kinds, unqualified frames or numerical residuals | Whole-picture loss |

An otherwise familiar family can still fail the inverse qualification, for
example because of text normalization, angular snapping or coordinate rounding.
Unsupported visual CAD common state, root/marker record metadata, external block
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

Member XDATA is separate from supported drawing geometry. It is omitted with
located dimension/part/member/application losses; arbitrary application data is
not assumed meaningless or preserved through a hidden channel. Other unsupported
common state still omits the picture. A MText colour stored while canvas fill is
selected is also diagnosed separately, as are discarded cached MText extents and
a conflicting inactive angle when an explicit direction controls orientation.
The selected canvas/explicit fill and effective text direction remain qualified
geometry/layout state. A colour used by an explicit fill cannot be discarded.

The original graphics fixture includes unrepresentable helper/layer semantics.
It intentionally still exercises a whole-picture loss; it is not silently
rewritten into a supposedly equivalent visible-only picture.

## Verification boundary

Tests exercise both directions, complete resource closures, absence/empty states,
scope placement, helper/layer binding, width metadata and atomic failure. Selected
composite pictures also pass actual ASCII DXF, binary DXF and AC1032 DWG output
and production readback before strict IFCX validation. This does not establish
coverage for every DWG version or every host renderer. A local private construction
drawing retains 87 native dimension pictures and 698 ordered members through
both original DXF/DWG inputs and real DXF/DWG readback. Independent ordered
numeric comparison checks coordinates, text directions/layout, appearance and
dimension ownership; counts alone are not fidelity evidence. A simple local
source/readback rendering comparison covers all stored pictures. A bounded
comparison in an unchanged local OpenCAD Studio viewer also checks source DWG
against the actual DWG and DXF readbacks at matching model-space views. It does
not establish universal host or font-contour equivalence. No private source
material is distributed with the tests.
No size/exchange measurement is part of this contribution.
