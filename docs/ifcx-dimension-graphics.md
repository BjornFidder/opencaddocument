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
picture-block base point. Rigid local/owner preparation uses shared frame
validation and the original axes; no separate absolute accuracy, reach/extent
or inverse-residual gate is imposed on pictures. Finite valid arithmetic follows
the corresponding ordinary primitive policy. Nonfinite/invalid output and
unsupported semantic state still omit the complete picture.

A +Z dimension inside an obliquely placed ordinary block is supported: the
occurrence applies to its definition and attached picture together. A tilted
dimension's own plane and nondefault clone/root-base transforms stay outside the
profile. See [conversion numerics](ifcx-numerics.md).

The inverse comparison is a closed typed semantic check. Only enumerated derived
fields (transformed positions/frames, required curve/angle/spacing derivations)
may differ. Kind, full/partial arc meaning and direction, direct radii/scales,
ordered topology, bulges, effective widths, text, style/layer bindings and other
non-derived state remain protected. It is not a generic float mask or an accuracy
certificate. Missing/ambiguous groups fail the independent world regression tests.

MText comparison folds native inheritance directly and compares complete typed
paragraph/run/stack content. CAD spelling uses a checked mode of the same markup
parser: equivalent braces/default formatting qualify; unknown unescaped controls
and malformed/uncovered parameters do not prove equivalence. The ordinary parser
remains lenient. Native fields that cannot be emitted, such as unsupported
paragraph formatting, cannot disappear through an emitter/parser normalization.
Oblique-angle unit conversion may change its numeric value while preserving
presence and every other formatting field. Entity spacing/columns/layout stay
separate. No glyph contour or host rendering equivalence is certified.

| Part family | Coverage within that profile |
| --- | --- |
| Line, display Point | Finite valid coordinates and representable appearance |
| Circle, Arc, Ellipse, EllipseArc | Derived frame/parameter arithmetic; direct radii and discrete curve meaning protected |
| Planar polyline | Ordered vertices, bulges and effective widths; eligible origins factored into vertices |
| Spatial polyline | Ordered vertices, closure/generation and retained appearance |
| Solid, Trace | Ordered corner slots, derived placement and direct thickness |
| Text | Effective runs/decorations and layout; fitted and aligned Text excluded |
| MText | Effective typed content, styles, layout, wrapping, direction and spacing |
| Marker block instance | Complete qualified closure; shared definitions and direct signed scales retained |
| Hatch, other kinds or unsupported semantic state | Whole-picture loss |

Unsupported visual CAD common state, root/marker record metadata, external block
references and external SORTENTSTABLE draw order remain outside the profile.
Explicit part order is retained; alternate external draw order is not guessed.

## Numerical observations

Each successfully retained picture may produce one diagnostic containing
`graphics retained with numerical rounding`, the first part/marker path and
observed categories (position, direction, derived length/angle or spacing).
It is emitted only for an observed arithmetic residual or a changed listed
derived value. Trigonometry alone does not trigger it; an unchanged identity
produces no numeric note. The diagnostic claims no tolerance or maximum error.

`Vec<Loss>` therefore also carries explicitly identified retention observations.
A nonempty list does not imply an omitted picture. Content-loss gates use a
separate internal store; retained notes cannot cause omission. Resources and notes
are staged together: a later failure publishes the omission reason and no
retained-rounding note or partial picture resource. Existing content-loss strings
and provenance/cache metadata reports keep their meaning.

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
coverage for every DWG version or every host renderer. The numerical regression
suite checks 47 frozen spatial cases, including 11-part pictures beside equivalent
ordinary geometry, physically equivalent units, and source/first/third-cycle
world observations. These checks are independent of production placement math;
they do not certify arbitrary drawings or glyph contours.

Separate earlier coverage used a local private construction
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
