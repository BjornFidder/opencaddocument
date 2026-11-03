# Font and SHX request mapping

This contribution uses IFCCAD profile 0.5.0 / CBOR5 on the annotation-context
basis. Cargo.toml pins the native resource contribution and unchanged codec
`4018037d`; no local source patch is needed. It builds on the annotation
contribution in upstream PR #10. These development Git revisions are not releases.

STYLE descriptor semantics follow the Open Design Alliance DWG specification
5.4.1, section 20.4.56, printed page 163 (PDF page 162):
https://www.opendesign.com/files/guestdownloads/OpenDesign_Specification_for_.dwg_files.pdf

Bold uses 0x02000000; italic 0x01000000; pitch 0x03; family 0xf0; charset
0xff00. The native pitch field carries the existing combined pitch/family
request (mask 0xf3). Other bits are diagnosed on import. Unsupported native
charset/pitch values refuse export. A descriptor without a family cannot be
physically retained by this codec and is refused. Absent face fields on family
requests materialize CAD defaults with retained-content notices.

SHX requests retain their full strings and independent shape-STYLE handles,
even for equal names. Redundant native optional presence can materialize away
in CAD with a notice; the request remains. No filesystem lookup occurs. Missing
or wrong-role internal STYLE links fail structurally.

A selected MText font name that cannot be emitted exactly refuses conversion.
Additional CAD or Big Font selectors are explicitly diagnosed. No font
substitution or embedded source storage is introduced.

Roundtrip tests cover AC1032 DWG and ASCII/binary DXF with production CAD
readers and strict native readback. A compact gallery and sample_AC1032 have
also completed user review. These are bounded storage/mapping claims, not
qualification of glyph rendering or PDF output.
