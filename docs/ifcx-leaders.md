# Leader-family IFCX mapping (development pair)

The selected native contract is IFCCAD profile 0.7.0 / CBOR7. The mapping uses
the existing `Ifcx` typed and file routes; the core remains independent of CAD.
The native dependency is pinned to `b76d41a661ff8d67bb91d0c7a4294da68714d197` and the codec to
`c67e07e67534e90abb1a6e85ad9b78ac57ca43b9` in the explicit development fork. The Document
candidate builds on upstream `49ec1ce3`; codec builds on upstream `9debd2c3`.
The checked graphics contribution is tracked in
[acadrust PR #341](https://github.com/hakanaktt/acadrust/pull/341).
These pins are review dependencies, not a published native release.

LEADER keeps its ordinary same-owner annotation target, including a shared
TOLERANCE/AcDbFcf target. Targets and MLeader label-to-ATTDEF bindings resolve
after the complete entity graph. MLeader content, branches, labels, contexts
and graphics remain owned parts rather than additional ordinary entities.
Labels use direct ATTDEF identities; no tag or source-index matching substitutes
for that link. Source group/line indices and native local keys are independent.

DIMLDRBLK is a separate style/entity setting. Supported explicit equal DSTYLE
values retain presence in both directions; DSTYLE parsing is shared with
dimensions. Existing unrepresentable dimension-style controls are diagnosed.
MLeader settings/styles share their conversion, and source override bits stay
separate from values. Source enums are checked before legacy defaulting.
Text/block matrices and base/scale presentation values are not regenerated.
Owned text/block placement is converted between owner coordinates and its
presentation frame exactly once. Stored text rotation remains independent of
the text direction.

Saved graphic streams are captured atomically with explicit table-index binding,
faithful legacy codepages and typed commands. Unknown/malformed commands or
unbound referenced resources are errors; unrelated unused CAD defaults need no
native binding. Generated picture provenance normalizes to SourceSnapshot on
CAD reimport and is diagnosed. Source snapshots do not prove rendering fidelity
or freshness after editing; the host owns invalidation and history.

Physical output preflights unsupported states. Simultaneous stored text and
block content cannot survive the current text-else-block writer and is refused.
Native owned tolerance content is stored and validated, but physical conversion
remains explicitly unqualified. The codec H1 type3/text-context experiment only
proves its own tag/string retention. It does not infer a DIMSTYLE or independent
CAD payload. See the codec's `src/docs/mleader-tolerance-payload.md` issue brief.

`examples/leaders_gallery.rs` creates one grouped gallery, strict IFCX/CBOR
readback and production AC1032 DWG/ASCII DXF/binary DXF readback. An optional
single practice path performs a bounded correctness roundtrip, not a benchmark
or corpus sweep. Review reports and generated files stay outside version control.
Reported source losses outside this family remain explicit.
