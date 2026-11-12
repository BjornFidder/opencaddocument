# Independent V1 selection fixtures

Run `python make_fixtures.py` from any directory to reproduce the checked-in
fixtures. The generator uses Python's standard library and independently writes
MessagePack markers and the envelope. Rust tests read fixed bytes.

| Fixture | Bytes | SHA-256 |
| --- | ---: | --- |
| `empty.msgpack` | 125 | `3479f222bcc149f0379691175f9bad8b28b8d9e60c99dce4cf2b0625a06b136f` |
| `mixed.msgpack` | 4635 | `8f5c819f7bfa1866620be675fc9ef4d3956bbacd681cd11897ca353db6852008` |

The empty selection has units 4 and scale 1. The mixed selection has three
roots, six records, 15 requirements and three evidence entries. Its MLINEs
share style 34, refer externally to layer 48 and retain a dangling linetype
handle 68 on one root. The first MLINE uses the captured source default and
has a handle above 2^53. The fixture also carries negative zero, infinity,
a NaN payload and an explicit line-weight variant.
