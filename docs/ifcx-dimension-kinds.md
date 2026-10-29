# Development Dimension conversion coverage

Development core: IFCCAD bce9da06a8e1684e86e35f0302e3870141e6f9f9
(profile 0.2.0, CBOR 2). This contribution is prepared against Document
fdd1d67404b4f7f269ebd8bbe5361d2485cccb62, including PR #5/#6. The CAD codec
is deliberately pinned to unmodified 501b694eee3433b157e56a32da4268dc8b3b5bd3,
the revision used for the accepted CAD review files. Upstream's newer 299dc75
codec update is a separate qualification step; merging this contribution must
not silently substitute it. Both dependencies are full portable Git revisions,
without a local source patch. Explorer/Studio adoption is recorded separately
in IFCCAD.

The candidate requires profile 0.2.0; it does not silently read the published
profile 0.1.0 with different semantics. The old tagged alpha remains separate.

## Definitions

Radius/diameter, two-line/three-point angles, absolute ordinate, positive
quarter-arc length and jogged radius have actual ASCII DXF, binary DXF and
AC1032 DWG readback cases. The ordinary integer-coordinate cases also exercise
four consecutive physical cycles: independent 3-4-5, perpendicular-ray, X-ordinate
and radius-times-quarter-turn references, measurements within two ULP and
per-coordinate bounds of 2/4/8/16 ULP at extent 10 in successive cycles. Those
bounds apply only to the named axis-aligned cases in the test; they establish no
universal placement or conversion tolerance.

Definitions retain distinct references and presentation positions. Radial CAD
helpers measure a spatial distance, so unequal native reference heights are
diagnosed. Angle export compares the selected CAD sector with the native angle;
unsupported major sectors are diagnosed. Absolute ordinate is supported;
signed mode is diagnosed.

The two-line arc-point field in unmodified codec 501b694 has a raw-XY DWG
boundary. This candidate qualifies that subtype only in the +Z, elevation-zero
presentation plane and diagnoses other planes instead of guessing WCS/OCS
semantics. Other new subtypes choose an actual presentation reference for the
native plane, retaining independent reference heights; nonplanar text/leader/jog
state that cannot fit the native presentation contract is diagnosed.

Negative arc spans and source partial-arc state not
represented by the native definition are diagnosed. Arc inputs require a finite
positive explicit span no larger than TAU, equal endpoint radii within the named
two-ULP scalar bound, and endpoint/parameter coherence within two ULP at the
local coordinate/radius extent. Contradictory endpoints are not used to infer
another interval. Major/full positive arc cells require their own qualification;
quarter-arc evidence does not establish those cells. Dedicated positive radius-
100 major (3PI/2) and explicit full-turn tests now retain both measure and span
through all three physical formats; negative spans remain diagnosed.

Measurement caches are derived CAD fields; native measurement uses definition
values. Styles, authored text and overrides use the common existing pipeline.
New-kind conversion explicitly diagnoses authored override presence that the
effective-value CAD mapping normalizes, including an explicit equal-to-parent
reset in either direction. The effective value remains; this is separate from
native IFCX/CBOR storage, which retains authored presence exactly.
There is no text-to-geometry inference, font resolution or regeneration engine.

## Optional source relations

DIMASSOC contains four optional point-reference slots and typed object snaps,
separately from DIMENSION definition points and its picture block. See the
[Autodesk DIMASSOC reference](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-DXF/files/GUID-C0B96256-A911-4B4D-85E6-EB4AF2C91E27.htm).
The module maps kind-specific roles instead of truncating all kinds to two
references. Existing Start/End on Line, Center on Circle/Arc and Position on
Point transfer. Native circle/arc/segment parameter selectors are stored and
validated in IFCCAD; their physical CAD encoding remains unqualified and is
explicitly diagnosed rather than guessed. Arc-center uses slot 1, ArcStart slot
2 and ArcEnd slot 3; slot 0 is the presentation-only leader point.

No relation is invented from coincident geometry. Native stale definitions do
not refresh from sources. Unsupported snap state, chained/path/intersection
references, trans-space state, ambiguous stale endpoint snaps and omitted or
cross-owner sources produce diagnostics while retaining an eligible definition.
Line Start/End export uses explicit vertex subentity identity (type 1,
markers 0/1), so editor refresh keeps the ends distinct when both roles refer
to one line. Import retains these endpoint identities even when cached snap
coordinates predate a source edit. Unspecified legacy endpoint references
still use stored endpoint coordinates; unsupported marker/type/snap combinations
remain diagnosed. The physical exchange regression covers ASCII/binary DXF and
AC1032 DWG before and after a source edit. This repairs endpoint identity;
general automatic update and interactive undo behavior remain host concerns.
The role/slot aliases are independently specified by the
[Bricsys TD_Mgd 25.3 enumeration](https://developer.bricsys.com/bricscad/help/en_US/V25/DevRef/source/html/b3505fcb-6bf5-cf6e-22f5-439c4447ce82.htm).
Arc center/start/end have a literal-mask-14 test and actual file readback;
slot 0 is not used for ArcStart. Full host associative update/reactor behavior
is not qualified. Same-writer/reader roundtrips alone do not establish it.

## Presentation and pictures

The existing atomic picture path applies independently to all nine kinds. The
3x3 gallery test retains each ordered complete picture through all three actual
file types. Its deliberately generated illustrative content is a test fixture,
not a production Dimension generator. CAD cannot retain IFCCAD's Generated
provenance; export reports that loss and later CAD capture uses SourceSnapshot.
Finite derived-coordinate rounding observations remain separate diagnostics.
Unsupported parts/resources omit the complete picture while a valid independent
definition can survive. See [picture coverage](ifcx-dimension-graphics.md).

Unmodified codec 501b694 resolves a DWG null picture pointer to synthetic *U0.
The Document DWG-reader wrapper converts that derived name to absence only when
no real *U0 block exists, preserving the source raw record. Other dangling
picture links remain errors; this is not an empty-picture approximation.

## Review files

`cargo run --locked --features serde,ifcx --example dimension_review -- NEW_DIR
[source.dwg]` refuses an existing output directory. It produces the nine-kind
gallery, individual absent-picture cases, and the supplied sample through
CAD -> native IFCCAD -> CAD. Each has strict native IFCX/CBOR readback, actual
returned DWG/ASCII/binary DXF, diagnostic lists and independent definition/
measurement/picture reports. A task packaging step adds exact input hashes,
dependency/config/tool identities and verification evidence before delivery.

Source `sample_AC1032` contains eleven dimensions in seven kinds (three linear,
two aligned, two ordinate, one each angular-two-lines, angular-three-points,
radius and diameter). The first development smoke retains all eleven through
the native intermediate and each returned format. Eight pictures transfer;
three source picture headers are outside the qualified profile. Other unsupported
sample entities and annotation scaling remain reported losses. This is not a
lossless whole-DWG conversion claim. Arc length/jogged radius are covered by the
synthetic gallery rather than that source sample. CAD product/build, repair
notices, displayed measurements/text/arrows, placement and re-save behavior
remain user review, and must precede Explorer/Studio reconnection.
