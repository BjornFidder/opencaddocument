# Selection capture field audit

This is the capture boundary against acadrust revision `cec07ce`. Exhaustive
struct patterns in capture code force review when supported carriers acquire
new fields. These classifications concern owned capture, not a durable byte
layout or a promise to insert arbitrary values into another drawing. See
[selection storage](selection-storage.md) for the durable V1 representation.

| Carrier | Retained typed values | Reference/context rules and exclusions |
| --- | --- | --- |
| `Line` | start, end, thickness, normal | common fields below |
| `MLine` | flags, justification, normal, scale_factor, start_point, style_element_count, vertices | style_handle/style_name retained; separate effective style requirement |
| `MLineVertex` | position, direction, miter, segments | no reference fields |
| `MLineSegment` | parameters, area_fill_parameters | complete float bits, no sorting or regeneration |
| `objects::MLineStyle` | handle, owner, name, description, elements, start_angle, end_angle, fill_color, flags | owner and relevant dictionary key classified as StyleRegistration context/gap; owner/name/handle contradictions fail |
| `objects::MLineStyleElement` | offset, color, linetype | one linetype requirement per element |
| `Layer` | handle, name, flags, color, color_name, book_name, line_type, line_weight, is_plottable, transparency, description | line_type requirement; non-null material/plotstyle/xref handles, nonempty plot_style, xref flag/name are unsupported |
| `LayerFlags` | frozen, locked, frozen_in_new_viewport, off | xref_dependent must be false |
| `LineType` | handle, name, description, elements, pattern_length, alignment | xref_dependent/name and xref_block_record_handle guarded; DGN backings/attachments unsupported |
| `LineTypeElement` | length | complex must be absent; embedded shapes/text/style references unsupported |
| source header | insertion_units, linetype_scale | current_multiline_style_handle used only for actual MLINE default resolution; ModelSpace/style/NOD handles establish roles, not whole header copies |

## EntityCommon

| Fields | Classification |
| --- | --- |
| handle | actual source identity, globally unique among database entities, objects, table records and controls; lookup identity must agree |
| layer, layer_handle | original values retained; included/external/missing Layer slot |
| linetype, linetype_handle | original values retained; simple linetype slot or ByLayer/ByBlock context; dangling handles remain gaps |
| owner_handle | retained source identity; must establish unambiguous ModelSpace context, compatible with source header and any explicit block memberships; no siblings copied |
| entity_mode | absent, owned (0), model (2) retained with ModelSpace owner; paper/unknown modes unsupported |
| color, line_weight, linetype_scale, transparency, color_name, invisible, shadow_flags | typed values retained without normalization |
| material_flags, plotstyle_flags | inheritance scalars 0/1 retained; reserved or object-reference modes unsupported |
| color_book_handle, full_visual_style_handle, face_visual_style_handle, edge_visual_style_handle, material_handle, plotstyle_handle | must be absent; no object reference silently discarded, including explicit null optional references |
| extended_data | both typed records and raw_dwg_eed must be empty |
| reactors, xdictionary_handle | must be empty/absent |
| r12_metadata | must be absent; not treated as disposable raw backing |
| raw_record, graphic_data, has_ds_data | removed only from owned copy, reported separately; typed values remain authoritative |

## Identity, side views and dictionary roles

The inventory counts actual entity records including structural markers and
all table records, supplemented by object map keys/intrinsic handles and table
controls. Only reached identities are required to be unambiguous. The source's
entity lookup must resolve the same handle as the payload. Conflicting block
memberships are rejected. Source values are not rewritten to repair the model.

On included payloads, semantic-inventory relationships and non-entity extended
data, public object XDATA/raw EED, reactors and extension-dictionary lookup are
audited. Any owned extension dictionary suffices to reject unsupported attached
semantics; the API does not pick one of several dictionaries to preserve.

ModelSpace and ACAD_MLINESTYLE dictionary registration are contextual roles, not
copied owner records. For a style, the header, dictionary ownership, NOD role
entry and relevant style entry must agree. Missing supported registrations stay
gaps; conflicting/duplicate relevant registrations fail. Unrelated dictionary
entries are not traversed. Dictionary cloning flags and unrelated dictionary
attachments are not part of a copied style payload or generic dictionary
preservation. External dependencies are identity-checked boundaries whose
payload/attachments/outgoing dependencies are the caller's responsibility.

Record payloads preserve original fields, while requirements retain interpreted
roles and effective defaults separately. No sanitization other than the three
declared entity storage/derived fields is allowed. New supported kinds, slots,
attachments, storage or reconstruction require their own reviewed boundary.
