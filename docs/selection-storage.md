# DocumentSelection storage V1

The `selection-storage` Cargo feature enables `selection` and exposes
`opencaddocument::selection_storage::{encode, decode}`. A selection can be
reopened after the source drawing has been dropped. The feature uses private V1
DTOs and named MessagePack maps. Model Serde can be enabled separately.

```rust
use opencaddocument::{
    entities::{EntityType, Line},
    selection::{CaptureRequest, DocumentSelection},
    selection_storage::{decode, encode},
    Document, Vector3,
};

let bytes = {
    let mut source = Document::new();
    let handle = source.add_entity(EntityType::Line(Line::from_points(
        Vector3::ZERO, Vector3::new(1.0, 0.0, 0.0),
    )))?;
    let request = CaptureRequest::new("drawing-a", vec![handle]);
    let capture = DocumentSelection::capture(&source, &request)?;
    encode(&capture.selection)?
};

let selection = decode(&bytes)?;
assert_eq!(selection.namespace(), "drawing-a");
assert_eq!(selection.roots().len(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The stored selection contains source payloads, requirements, context and
capture evidence. A known missing requirement stays missing. Destination
handles, current bindings, placement and draw order belong to a later
reconstruction request. Applications can retain the original bytes if a version
is unsupported or an otherwise valid selection cannot be reconstructed.

V1 retains the identity and geometry needed for later reconstruction. Identity
placement preserves all captured numeric bits. A later translation can use
the stored coordinates and source units. Target names, placement and draw
order belong to the reconstruction request. Any insertion into an existing
SortEnts order requires a check against the current target document.

## Envelope

| Offset | Size | Value |
| --- | ---: | --- |
| 0 | 8 | ASCII `OCADSEL` followed by byte `00` |
| 8 | 2 | Big-endian unsigned version, `0001` |
| 10 | 1 | Selection kind, `01` |
| 11 | 4 | Big-endian unsigned body length |
| 15 | Body length | Exactly one MessagePack body |

Read the full header first, then validate magic, version, kind, size and exact
frame length in that order. Unsupported versions and kinds have separate
errors. A truncated header or value yields `Truncated`. A second body value or
bytes after the frame yield `TrailingData`. Invalid markers, types, tags and
map fields yield `InvalidBody`. Inconsistent selection content yields
`InvalidSelection`. The error exposes its category and a byte offset or JSON
Pointer when available.

Both routes cap the body at 64 MiB, container depth at 16, each array or map
at 262,144 elements or entries, the total number of values at 1,048,576
(including maps, arrays and keys), and UTF-8 per string at 1,048,576 bytes.
Limits are checked before allocations based on wire lengths. Excess yields
`LimitExceeded`.

## Wire grammar

Maps use exactly the named keys listed below. The writer follows the order
listed and uses the shortest integer markers. The reader accepts key
permutations and nonminimal integer encodings of in-range values. Each key
appears exactly once. `T?` denotes a required key with a value of type `T`
or `nil`. Arrays and tagged tuples have the specified order and arity.

Unsigned and signed integer types have their usual bit-width bounds. A handle
`h` is a full-width unsigned integer. Record keys require a nonzero handle.
`bits64` is an unsigned integer containing the exact IEEE binary64 bits.
`vec3` is an array of three `bits64` values in x/y/z order. `count32` and
`index32` use unsigned 32-bit integers. A count above that range cannot be
encoded. `char32` is a Unicode scalar encoded as an unsigned 32-bit integer.
Use string, boolean and nil markers for their respective types. Binary,
extension and physical float markers are rejected.

| Type | Fields in writer order |
| --- | --- |
| Body | `namespace: str`, `roots: [key]`, `records: [record]`, `requirements: [requirement]`, `report: [evidence]`, `context: context` |
| `context` | `insertion_units: i16`, `linetype_scale: bits64` |
| `key` | `kind: str`, `handle: h` |
| `record` | `key: key`, `payload: [type: str, fields: map]` |
| `requirement` | `source: key`, `slot: slot`, `source_handle: h?`, `source_name: str?`, `disposition: disposition` |
| `evidence` | `source: key?`, `slot: slot?`, `code: str`, `message: str` |

Key kinds are `entity`, `layer`, `linetype`, `object`. Each key is scoped
to the namespace in the body. Namespace must be nonempty after trimming.
The original text and all list orders are preserved.

| Payload tag | Key kind | Fields in writer order |
| --- | --- | --- |
| `line` | `entity` | `common: common`, `start: vec3`, `end: vec3`, `thickness: bits64`, `normal: vec3` |
| `mline` | `entity` | `common: common`, `flags: i16`, `justification: str`, `normal: vec3`, `scale_factor: bits64`, `start_point: vec3`, `style_handle: h?`, `style_name: str`, `style_element_count: count32`, `vertices: [vertex]` |
| `mline_style` | `object` | `handle: h`, `owner: h`, `name: str`, `description: str`, `elements: [style_element]`, `start_angle: bits64`, `end_angle: bits64`, `fill_color: color`, `flags: style_flags` |
| `layer` | `layer` | `handle: h`, `name: str`, `flags: layer_flags`, `color: color`, `color_name: str?`, `book_name: str?`, `line_type: str`, `line_weight: line_weight`, `is_plottable: bool`, `transparency: transparency`, `description: str` |
| `linetype` | `linetype` | `handle: h`, `name: str`, `description: str`, `elements: [pattern_element]`, `pattern_length: bits64`, `alignment: char32` |

The `common` map contains `handle: h`, `layer: str`, `layer_handle: h?`,
`color: color`, `line_weight: line_weight`, `linetype: str`,
`linetype_handle: h?`, `linetype_scale: bits64`, `transparency: transparency`,
`color_name: str?`, `invisible: bool`, `owner_handle: h`,
`material_flags: u8`, `shadow_flags: u8`, `plotstyle_flags: u8`,
`entity_mode: u8?` in that order. Owner is nonzero. Material and plotstyle
flags must be 0 or 1. Entity mode permits nil, 0 and 2.

| Nested map | Fields in writer order |
| --- | --- |
| `vertex` | `position: vec3`, `direction: vec3`, `miter: vec3`, `segments: [segment]` |
| `segment` | `parameters: [bits64]`, `area_fill_parameters: [bits64]` |
| `style_element` | `offset: bits64`, `color: color`, `linetype: str` |
| `style_flags` | Booleans `fill_on`, `display_joints`, `start_square_cap`, `start_inner_arcs_cap`, `start_round_cap`, `end_square_cap`, `end_inner_arcs_cap`, `end_round_cap` |
| `layer_flags` | Booleans `frozen`, `locked`, `frozen_in_new_viewport`, `off` |
| `pattern_element` | `length: bits64` |

Justification tags are `top`, `zero`, `bottom`. MLINE flags retain all 16
bits. Float bits preserve signed zero, infinities and NaN payloads. Storage
applies no geometric normalization or inferred counts.

| Tagged value | Tuples |
| --- | --- |
| Color | `["by_layer"]`, `["none"]`, `["by_block"]`, `["index", u8]`, `["rgb", u8, u8, u8]` |
| Line weight | `["by_layer"]`, `["by_block"]`, `["default"]`, `["value", i16]` |
| Transparency | `["by_layer"]`, `["by_block"]`, `["explicit", u8]` |
| Slot | `["owner"]`, `["layer"]`, `["linetype"]`, `["mline_style"]`, `["style_element_linetype", index32]`, `["style_registration"]` |
| Disposition | `["included", key]`, `["external", key]`, `["missing"]`, `["context", role]` |

Context role strings are `model_space`, `mline_style_dictionary`,
`by_layer` and `by_block`. Evidence code strings are `missing_dependency`,
`excluded_raw_record`, `excluded_graphics`, `excluded_derived_data`,
`resolved_source_default` and `unknown_units`. Preserve the message verbatim.

The projection includes the values admitted by the [capture field audit](selection-fields.md).
Entity XDATA/EED and reactors are empty. Graphics, raw records, R12 metadata,
extension dictionaries and the excluded object reference options are absent,
and the derived data-store flag is false. Included layers have empty plot style
and zero material, plotstyle and xref handles. Included layers and linetypes
have no xref flag or pipe in the name. Linetype complex elements are absent.
Decode constructs those explicit values. Encode checks them before projection.

## Selection validity

- Roots are unique and exactly identify the drawable records. Record keys agree
  with payload kinds and handles. Record and external identities are nonzero and
  unambiguous across kind domains. Each included non-root record is reachable
  from a root through included requirements.
- Each entity has one owner, layer and linetype slot. MLINE also has a style
  slot. A layer has one linetype slot. An MLINESTYLE has a registration slot and
  a linetype slot for every element. Linetypes have no requirements. Each
  source/slot pair is unique. Incoming slots sharing an endpoint remain separate.
- Every root owner is the same source ModelSpace handle with a ModelSpace
  context. Each nonzero style owner is the same source style dictionary.
  Registration carries the style name and owner, with a dictionary context or
  a known gap. Context owner identities are separate from payload identities.
- Resource requirements preserve source name and handle from the corresponding
  payload field. Layer and style-element linetype slots have a nil source handle.
  The MLINE style slot has the effective source handle, including a zero
  missing style. An explicit original nonzero style handle agrees with it.
  An absent or zero original handle retains source-default evidence.
- Included and external endpoints match the slot kind and an explicit nonzero
  source handle. Included targets exist in the selection. External targets
  are outside it. An included target name agrees with a nonempty supplied name
  by D1's uppercase comparison. A linetype name that is empty or ByLayer
  (ASCII-case insensitive) denotes ByLayer, and ByBlock denotes ByBlock.
  These roles can be missing only for a dangling nonzero source handle.
  A missing nonzero endpoint cannot name a known included or external identity.
  Reopen preserves a gap even if another record has a matching name.
- Missing and default evidence each identify the corresponding source/slot
  exactly once. Exclusion evidence locates an entity with a nil slot, once per
  code per entity. Unit evidence has both locations nil and appears exactly
  once for nonempty roots when the units code is outside 1 through 24.

Changing fields, tags, absence rules or wire semantics requires a separate
version decision. The envelope selects the reader before it interprets the
payload. Re-encoding a valid alternative marker or map order yields the writer's
V1 representation. Independent fixtures and their source generator live under
`tests/fixtures/selection/v1/`.
