# IFCX annotation contexts

This contribution maps the unpublished IFCCAD profile 0.4.0 / CBOR mapping 4.
Its portable development dependencies are pinned exactly:

- Native `OpenAEC-Foundation/ifccad` commit
  `100deb04052a298654a33446b509194ad0bc03bf`, based on main `908aed6b`.
- Codec `BjornFidder/opencadcodec` commit
  `4018037daf8fd3e40aa1aa6455b80232849a2756`, based on upstream
  `hakanaktt/acadrust` commit `7e7940f6fb17419629958ceeb79dfbd2d954dabb`,
  with the HatchView SoftPointer correction and regression tests.

These are public development revisions, not a native release. The native package
version is unchanged from 2026.45.0, so its version number alone does not identify
profile 0.4.0. A stock checkout uses the matching model without local path patches.
The contribution incorporates Document main `598b1bb`; its intervening changes
only advance the codec pin. The new TABLE/Field features are outside native IFCX
coverage and do not expand this annotation slice.

The initially accepted review files used Document `28f1ad1`, native `8367458`
plus the annotation contract, and codec `0a5542b` plus the pointer correction,
through explicit local Cargo patches. Intermediate qualification used Document
`3a3676b` and codec `ef27529c` plus that correction. Those historical pairs remain
separate from the portable pins above. Explorer's accepted review files and
embedded-host bundle retain their recorded earlier pair. Never edit Cargo's
source cache to substitute development code.

## Retained state

- Drawing-local scales retain names, both authored units, unit designation and
  unused identities; equal ratios remain separate definitions.
- Text and MText retain memberships, independent default, explicit object mode
  and paper orientation, and saved anchors/directions/rectangles/columns.
- All nine Dimension kinds retain contextual XYZ points, text choices, five
  binary override hints with independent activity/value bits, arrow hints and
  independent saved pictures. Canonical geometry, overrides and source links
  remain separate from those snapshots.
- Hatch scale and view collections retain independent defaults. View leaves keep
  repeated/null targets, named views/model-window/Paper-viewport identities,
  view state, explicit pattern values and shared/local boundaries with fourteen
  named classification flags.
- Block references retain full contextual transforms and definition policy.
  TextStyle/BlockDefinition creation policy does not retroactively annotate text.
- Saved drawing policies retain ANNOTATIVEDWG and MSLTSCALE; Model and Paper
  visibility/current scale remain independent. Paper's current scale is owned
  by its overall ID1 viewport, independently of floating viewport choices. Named
  VIEW has its own scale; Model VPORT windows share Model's choice.

## Boundaries and diagnostics

No layout regeneration or editing engine is introduced. Unknown Dimension
reserved bits refuse conversion; binary hints are not promoted to the style's
multi-valued fitting/movement enums. ArcLength reuses the Angular context
mapping under the accepted SDK-based assumption; no independently authored
multi-scale ArcLength source is available yet.

Native ByDefinition with memberships materializes its current effective CAD
object flag and reports the lost future inheritance. Nonempty local Hatch
context source IDs and dormant MText column heights without CAD fields report
explicit losses. Nonzero MText layout-extents caches are omitted with a diagnostic;
rectangles/column semantics remain retained. Unsupported object families stay
outside this contribution. Context pictures use the existing bounded graphics
qualifier and retain its numerical/rendering limitations.

Original BricsCAD files in [this public attachment](https://forum.bricsys.com/discussion/36137/can-i-have-different-multileader-text-box-sizes-at-different-annotation-scales)
prove ANNOTATIVEDWG values 1/0 as DICTIONARYVAR entries under
AcDbVariableDictionary. It is not the annotative marker of ModelSpace's block
record. Those downloaded source files are not committed here.

## Verification

`ifcx_annotation`, `ifcx_annotation_dimensions` and `ifcx_annotation_policy`
exercise memory, actual DWG, ASCII/binary DXF and strict native JSON/CBOR
readback. They compare semantic values and independent defaults rather than
unstable handles. The compact `annotation_gallery` example reproduces the
accepted CAD review files. Native and Explorer qualification remain distinct.
The paired focused suite has 19 passing tests; the user accepted the small CAD
roundtrips and reported opening/converting existing DWG/DXF in local Explorer.
The native source candidate passes its full workspace audit, fmt, Clippy and tests
(629 passed, 2 existing ignored cases), plus the Explorer dependency guard and
frontend tests. Portable Document tests pass with the final Git pins: default 4, serde 4,
serde+IFCX 133, with no ignored tests. Fmt and all-target Clippy also pass. The separately accepted embedded Studio bundle retains its earlier
recorded development pair; production host integration/deployment and editing
qualification are not claimed complete.
