use super::references::{Reference, SourceIndex};
use super::*;
use crate::{
    entities::{EntityCommon, EntityType, Line, MLine, MLineSegment, MLineVertex},
    objects::{MLineStyle, MLineStyleElement, ObjectType},
    tables::{Layer, LayerFlags, LineType, LineTypeElement},
};
use std::collections::{BTreeSet, VecDeque};

fn key(kind: SelectionRecordKind, handle: Handle) -> SelectionRecordKey {
    SelectionRecordKey { kind, handle }
}
fn error(
    kind: SelectionErrorKind,
    source: Option<SelectionRecordKey>,
    slot: Option<SelectionSlot>,
    message: &str,
) -> SelectionError {
    SelectionError {
        kind,
        source,
        slot,
        message: message.into(),
    }
}

pub(super) fn capture(
    doc: &Document,
    request: &CaptureRequest,
) -> Result<CaptureOutcome, SelectionError> {
    if request.namespace.trim().is_empty() {
        return Err(error(
            SelectionErrorKind::InvalidRequest,
            None,
            None,
            "capture requires a nonempty source namespace",
        ));
    }
    let mut roots = BTreeSet::new();
    for h in &request.roots {
        if h.is_null()
            || !roots.insert(*h)
            || request
                .external
                .contains(&key(SelectionRecordKind::Entity, *h))
        {
            return Err(error(
                SelectionErrorKind::InvalidRequest,
                Some(key(SelectionRecordKind::Entity, *h)),
                None,
                "roots must be unique non-null included entities",
            ));
        }
    }
    let selection = DocumentSelection {
        namespace: request.namespace.clone(),
        roots: request
            .roots
            .iter()
            .map(|h| key(SelectionRecordKind::Entity, *h))
            .collect(),
        records: vec![],
        requirements: vec![],
        report: vec![],
        context: SelectionContext {
            insertion_units: doc.header.insertion_units,
            linetype_scale: doc.header.linetype_scale,
        },
    };
    let mut capture = Capture {
        doc,
        request,
        index: SourceIndex::new(doc),
        selection,
        queued: BTreeSet::new(),
        queue: VecDeque::new(),
        external_used: BTreeSet::new(),
    };
    for h in &request.roots {
        let k = key(SelectionRecordKind::Entity, *h);
        if doc.get_entity(*h).is_none() {
            return Err(error(
                SelectionErrorKind::InvalidRequest,
                Some(k),
                None,
                "root entity does not exist",
            ));
        }
        if doc.get_entity(*h).is_some_and(|e| e.common().handle != *h) {
            return Err(error(
                SelectionErrorKind::AmbiguousIdentity,
                Some(k),
                None,
                "source entity lookup disagrees with its actual payload identity",
            ));
        }
        capture.enqueue(k);
    }
    if !request.roots.is_empty() && !(1..=24).contains(&doc.header.insertion_units) {
        capture.evidence(
            None,
            None,
            CaptureEvidenceCode::UnknownUnits,
            "source units are unitless or unrecognized; no units were inferred",
        );
    }
    while let Some(k) = capture.queue.pop_front() {
        capture.record(k)?;
    }
    if capture.external_used != request.external {
        return Err(error(
            SelectionErrorKind::InvalidRequest,
            None,
            None,
            "external boundary contains an unused or nonexistent dependency",
        ));
    }
    Ok(CaptureOutcome {
        selection: capture.selection,
    })
}

struct Capture<'a> {
    doc: &'a Document,
    request: &'a CaptureRequest,
    index: SourceIndex,
    selection: DocumentSelection,
    queued: BTreeSet<SelectionRecordKey>,
    queue: VecDeque<SelectionRecordKey>,
    external_used: BTreeSet<SelectionRecordKey>,
}
impl Capture<'_> {
    fn enqueue(&mut self, k: SelectionRecordKey) {
        if self.queued.insert(k) {
            self.queue.push_back(k);
        }
    }
    fn evidence(
        &mut self,
        source: Option<SelectionRecordKey>,
        slot: Option<SelectionSlot>,
        code: CaptureEvidenceCode,
        message: &str,
    ) {
        self.selection.report.push(CaptureEvidence {
            source,
            slot,
            code,
            message: message.into(),
        });
    }
    fn requirement(&mut self, reference: Reference, disposition: RequirementDisposition) {
        if disposition == RequirementDisposition::Missing {
            if let Some(h) = reference.handle.filter(|h| {
                !h.is_null()
                    && matches!(
                        reference.slot,
                        SelectionSlot::Layer
                            | SelectionSlot::LineType
                            | SelectionSlot::MLineStyle
                            | SelectionSlot::StyleElementLineType(_)
                    )
            }) {
                let k = key(reference.kind, h);
                if self.request.external.contains(&k) {
                    self.external_used.insert(k);
                }
            }
            self.evidence(
                Some(reference.source),
                Some(reference.slot.clone()),
                CaptureEvidenceCode::MissingDependency,
                "supported source dependency is missing; original reference retained",
            );
        }
        self.selection.requirements.push(SelectionRequirement {
            source: reference.source,
            slot: reference.slot,
            source_handle: reference.handle,
            source_name: reference.name,
            disposition,
        });
    }
    fn reference(&mut self, r: Reference) -> Result<(), SelectionError> {
        let name = r.name.as_deref().unwrap_or("");
        let handle = r.handle.filter(|h| !h.is_null());
        let matches: Vec<Handle> = match r.kind {
            SelectionRecordKind::Layer => self
                .doc
                .layers
                .iter()
                .filter(|v| v.name.to_uppercase() == name.to_uppercase())
                .map(|v| v.handle)
                .collect(),
            SelectionRecordKind::LineType => self
                .doc
                .line_types
                .iter()
                .filter(|v| v.name.to_uppercase() == name.to_uppercase())
                .map(|v| v.handle)
                .collect(),
            SelectionRecordKind::Object => self
                .doc
                .objects
                .values()
                .filter_map(|v| match v {
                    ObjectType::MLineStyle(s) if s.name.to_uppercase() == name.to_uppercase() => {
                        Some(s.handle)
                    }
                    _ => None,
                })
                .collect(),
            SelectionRecordKind::Entity => unreachable!("entities are roots, not dependency slots"),
        };
        if matches.len() > 1 {
            return Err(error(
                SelectionErrorKind::AmbiguousIdentity,
                Some(r.source),
                Some(r.slot),
                "dependency name is ambiguous",
            ));
        }
        let resolved = if let Some(h) = handle {
            let actual_name = match r.kind {
                SelectionRecordKind::Layer => self
                    .doc
                    .layers
                    .iter()
                    .find(|v| v.handle == h)
                    .map(|v| v.name.as_str()),
                SelectionRecordKind::LineType => self
                    .doc
                    .line_types
                    .iter()
                    .find(|v| v.handle == h)
                    .map(|v| v.name.as_str()),
                SelectionRecordKind::Object => match self.doc.objects.get(&h) {
                    Some(ObjectType::MLineStyle(s)) => Some(s.name.as_str()),
                    _ => None,
                },
                SelectionRecordKind::Entity => None,
            };
            if let Some(actual_name) = actual_name {
                self.index.unique(key(r.kind, h), Some(r.slot.clone()))?;
                if !name.is_empty() && actual_name.to_uppercase() != name.to_uppercase() {
                    return Err(error(
                        SelectionErrorKind::ReferenceConflict,
                        Some(r.source),
                        Some(r.slot),
                        "dependency name and handle disagree",
                    ));
                }
                Some(h)
            } else {
                if self.index.counts.contains_key(&h) {
                    return Err(error(
                        SelectionErrorKind::ReferenceConflict,
                        Some(r.source),
                        Some(r.slot),
                        "handle exists in an incompatible source role",
                    ));
                }
                // A dangling non-null handle never falls back to a name.
                None
            }
        } else {
            matches.first().copied()
        };
        let Some(h) = resolved else {
            self.requirement(r, RequirementDisposition::Missing);
            return Ok(());
        };
        let k = key(r.kind, h);
        self.index.unique(k, Some(r.slot.clone()))?;
        if self.request.external.contains(&k) {
            self.external_used.insert(k);
            self.requirement(r, RequirementDisposition::External(k));
        } else {
            self.requirement(r, RequirementDisposition::Included(k));
            self.enqueue(k);
        }
        Ok(())
    }
    fn linetype(
        &mut self,
        source: SelectionRecordKey,
        slot: SelectionSlot,
        handle: Option<Handle>,
        name: &str,
    ) -> Result<(), SelectionError> {
        let role = if name.is_empty() || name.eq_ignore_ascii_case("BYLAYER") {
            Some(ContextRole::ByLayer)
        } else if name.eq_ignore_ascii_case("BYBLOCK") {
            Some(ContextRole::ByBlock)
        } else {
            None
        };
        if let Some(role) = role {
            if let Some(h) = handle.filter(|h| !h.is_null()) {
                let expected = if role == ContextRole::ByLayer {
                    "BYLAYER"
                } else {
                    "BYBLOCK"
                };
                match self.doc.line_types.iter().find(|v| v.handle == h) {
                    Some(v) if v.name.eq_ignore_ascii_case(expected) => self
                        .index
                        .unique(key(SelectionRecordKind::LineType, h), Some(slot.clone()))?,
                    Some(_) => {
                        return Err(error(
                            SelectionErrorKind::ReferenceConflict,
                            Some(source),
                            Some(slot),
                            "linetype role and handle disagree",
                        ))
                    }
                    None if self.index.counts.contains_key(&h) => {
                        return Err(error(
                            SelectionErrorKind::ReferenceConflict,
                            Some(source),
                            Some(slot),
                            "linetype handle points to another kind",
                        ))
                    }
                    None => {
                        self.requirement(
                            Reference {
                                source,
                                slot,
                                handle,
                                name: Some(name.into()),
                                kind: SelectionRecordKind::LineType,
                            },
                            RequirementDisposition::Missing,
                        );
                        return Ok(());
                    }
                }
            }
            self.requirement(
                Reference {
                    source,
                    slot,
                    handle,
                    name: Some(name.into()),
                    kind: SelectionRecordKind::LineType,
                },
                RequirementDisposition::Context(role),
            );
            Ok(())
        } else {
            self.reference(Reference {
                source,
                slot,
                handle,
                name: Some(name.into()),
                kind: SelectionRecordKind::LineType,
            })
        }
    }
    fn common(
        &mut self,
        k: SelectionRecordKey,
        c: &mut EntityCommon,
    ) -> Result<(), SelectionError> {
        let EntityCommon {
            handle: _,
            layer,
            layer_handle,
            color: _,
            line_weight: _,
            linetype,
            linetype_handle,
            linetype_scale: _,
            transparency: _,
            color_name: _,
            invisible: _,
            extended_data,
            graphic_data,
            reactors,
            xdictionary_handle,
            owner_handle,
            color_book_handle,
            full_visual_style_handle,
            face_visual_style_handle,
            edge_visual_style_handle,
            material_flags,
            material_handle,
            shadow_flags: _,
            plotstyle_flags,
            plotstyle_handle,
            entity_mode,
            r12_metadata,
            has_ds_data,
            raw_record,
        } = c;
        if !extended_data.records().is_empty()
            || !extended_data.raw_dwg_eed.is_empty()
            || !reactors.is_empty()
            || xdictionary_handle.is_some()
            || [
                *color_book_handle,
                *full_visual_style_handle,
                *face_visual_style_handle,
                *edge_visual_style_handle,
                *material_handle,
                *plotstyle_handle,
            ]
            .iter()
            .any(Option::is_some)
            || *material_flags > 1
            || *plotstyle_flags > 1
            || r12_metadata.is_some()
            || entity_mode.is_some_and(|v| v != 0 && v != 2)
        {
            return Err(error(
                SelectionErrorKind::UnsupportedContent,
                Some(k),
                None,
                "entity contains attachments or resource/mode semantics outside capture coverage",
            ));
        }
        let model = self.doc.block_records.get("*Model_Space").ok_or_else(|| {
            error(
                SelectionErrorKind::UnsupportedContent,
                Some(k),
                Some(SelectionSlot::Owner),
                "source has no ModelSpace block",
            )
        })?;
        if self
            .doc
            .block_records
            .iter()
            .filter(|b| b.name.eq_ignore_ascii_case("*Model_Space"))
            .count()
            != 1
        {
            return Err(error(
                SelectionErrorKind::AmbiguousIdentity,
                Some(k),
                Some(SelectionSlot::Owner),
                "source ModelSpace name is ambiguous",
            ));
        }
        self.index.unique(
            key(SelectionRecordKind::Object, model.handle),
            Some(SelectionSlot::Owner),
        )?;
        if owner_handle.is_null()
            || *owner_handle != model.handle
            || (!self.doc.header.model_space_block_handle.is_null()
                && self.doc.header.model_space_block_handle != model.handle)
        {
            return Err(error(
                SelectionErrorKind::UnsupportedContent,
                Some(k),
                Some(SelectionSlot::Owner),
                "only unambiguous ModelSpace roots are supported",
            ));
        }
        if self
            .index
            .memberships
            .get(&k.handle)
            .is_some_and(|owners| owners.len() != 1 || owners[0] != model.handle)
        {
            return Err(error(
                SelectionErrorKind::ReferenceConflict,
                Some(k),
                Some(SelectionSlot::Owner),
                "source block membership contradicts the root's ModelSpace owner",
            ));
        }
        self.requirement(
            Reference {
                source: k,
                slot: SelectionSlot::Owner,
                handle: Some(*owner_handle),
                name: Some(model.name.clone()),
                kind: SelectionRecordKind::Object,
            },
            RequirementDisposition::Context(ContextRole::ModelSpace),
        );
        self.reference(Reference {
            source: k,
            slot: SelectionSlot::Layer,
            handle: *layer_handle,
            name: Some(layer.clone()),
            kind: SelectionRecordKind::Layer,
        })?;
        self.linetype(k, SelectionSlot::LineType, *linetype_handle, linetype)?;
        if raw_record.take().is_some() {
            self.evidence(
                Some(k),
                None,
                CaptureEvidenceCode::ExcludedRawRecord,
                "DWG raw record excluded from owned typed capture",
            );
        }
        if graphic_data.take().is_some() {
            self.evidence(
                Some(k),
                None,
                CaptureEvidenceCode::ExcludedGraphics,
                "saved graphics excluded from owned typed capture",
            );
        }
        if *has_ds_data {
            *has_ds_data = false;
            self.evidence(
                Some(k),
                None,
                CaptureEvidenceCode::ExcludedDerivedData,
                "derived data-store flag excluded",
            );
        }
        Ok(())
    }
    fn record(&mut self, k: SelectionRecordKey) -> Result<(), SelectionError> {
        self.index.audit(self.doc, k)?;
        let payload = match k.kind {
            SelectionRecordKind::Entity => {
                match self.doc.get_entity(k.handle).expect("validated root") {
                    EntityType::Line(line) => {
                        let Line {
                            common: _,
                            start: _,
                            end: _,
                            thickness: _,
                            normal: _,
                        } = line;
                        let mut line = line.clone();
                        self.common(k, &mut line.common)?;
                        SelectionPayload::Line(Box::new(line))
                    }
                    EntityType::MLine(mline) => {
                        let MLine {
                            common: _,
                            flags: _,
                            justification: _,
                            normal: _,
                            scale_factor: _,
                            start_point: _,
                            style_handle,
                            style_name,
                            style_element_count: _,
                            vertices,
                        } = mline;
                        for MLineVertex {
                            position: _,
                            direction: _,
                            miter: _,
                            segments,
                        } in vertices
                        {
                            for MLineSegment {
                                parameters: _,
                                area_fill_parameters: _,
                            } in segments
                            {}
                        }
                        let (style_handle, style_name) = (*style_handle, style_name.clone());
                        let mut mline = mline.clone();
                        self.common(k, &mut mline.common)?;
                        let effective = if style_handle.is_none_or(|h| h.is_null()) {
                            self.evidence(Some(k),Some(SelectionSlot::MLineStyle),CaptureEvidenceCode::ResolvedSourceDefault,"absent/null entity style handle uses the actual source header style");
                            Some(self.doc.header.current_multiline_style_handle)
                        } else {
                            style_handle
                        };
                        // A null current source style is a gap, never a name/default repair.
                        if effective.is_some_and(|h| h.is_null()) {
                            self.requirement(
                                Reference {
                                    source: k,
                                    slot: SelectionSlot::MLineStyle,
                                    handle: effective,
                                    name: Some(style_name),
                                    kind: SelectionRecordKind::Object,
                                },
                                RequirementDisposition::Missing,
                            );
                        } else {
                            self.reference(Reference {
                                source: k,
                                slot: SelectionSlot::MLineStyle,
                                handle: effective,
                                name: Some(style_name),
                                kind: SelectionRecordKind::Object,
                            })?;
                        }
                        SelectionPayload::MLine(Box::new(mline))
                    }
                    _ => {
                        return Err(error(
                            SelectionErrorKind::UnsupportedContent,
                            Some(k),
                            None,
                            "only Line and MLine roots are supported",
                        ))
                    }
                }
            }
            SelectionRecordKind::Layer => {
                let layer = self
                    .doc
                    .layers
                    .iter()
                    .find(|v| v.handle == k.handle)
                    .expect("resolved layer");
                let Layer {
                    handle: _,
                    name,
                    flags,
                    color: _,
                    color_name: _,
                    book_name: _,
                    line_type,
                    line_weight: _,
                    plot_style,
                    is_plottable: _,
                    transparency: _,
                    description: _,
                    material,
                    plotstyle_handle,
                    xref_block_record_handle,
                } = layer;
                let LayerFlags {
                    frozen: _,
                    locked: _,
                    frozen_in_new_viewport: _,
                    off: _,
                    xref_dependent,
                } = flags;
                if *xref_dependent
                    || name.contains('|')
                    || !material.is_null()
                    || !plotstyle_handle.is_null()
                    || !xref_block_record_handle.is_null()
                    || !plot_style.is_empty()
                {
                    return Err(error(SelectionErrorKind::UnsupportedContent,Some(k),None,"xref, material and plot-style layer dependencies are outside capture coverage"));
                }
                self.linetype(k, SelectionSlot::LineType, None, line_type)?;
                SelectionPayload::Layer(Box::new(layer.clone()))
            }
            SelectionRecordKind::LineType => {
                let linetype = self
                    .doc
                    .line_types
                    .iter()
                    .find(|v| v.handle == k.handle)
                    .expect("resolved linetype");
                let LineType {
                    handle: _,
                    name,
                    description: _,
                    elements,
                    pattern_length: _,
                    alignment: _,
                    xref_dependent,
                    xref_block_record_handle,
                } = linetype;
                if *xref_dependent
                    || name.contains('|')
                    || !xref_block_record_handle.is_null()
                    || elements
                        .iter()
                        .any(|LineTypeElement { length: _, complex }| complex.is_some())
                    || self.doc.dgn_ls_definitions.contains_key(&k.handle)
                    || self.doc.dgn_ls_components.contains_key(&k.handle)
                {
                    return Err(error(
                        SelectionErrorKind::UnsupportedContent,
                        Some(k),
                        None,
                        "complex, DGN and xref linetypes are outside capture coverage",
                    ));
                }
                SelectionPayload::LineType(Box::new(linetype.clone()))
            }
            SelectionRecordKind::Object => {
                let Some(ObjectType::MLineStyle(style)) = self.doc.objects.get(&k.handle) else {
                    unreachable!("resolved style")
                };
                let MLineStyle {
                    handle: _,
                    owner: _,
                    name: _,
                    description: _,
                    elements,
                    start_angle: _,
                    end_angle: _,
                    fill_color: _,
                    flags: _,
                } = style;
                self.registration(k, style)?;
                for (
                    i,
                    MLineStyleElement {
                        offset: _,
                        color: _,
                        linetype,
                    },
                ) in elements.iter().enumerate()
                {
                    self.linetype(k, SelectionSlot::StyleElementLineType(i), None, linetype)?;
                }
                SelectionPayload::MLineStyle(Box::new(style.clone()))
            }
        };
        self.selection
            .records
            .push(SelectionRecord { key: k, payload });
        Ok(())
    }
    fn registration(
        &mut self,
        k: SelectionRecordKey,
        style: &MLineStyle,
    ) -> Result<(), SelectionError> {
        let r = Reference {
            source: k,
            slot: SelectionSlot::StyleRegistration,
            handle: Some(style.owner),
            name: Some(style.name.clone()),
            kind: SelectionRecordKind::Object,
        };
        if style.owner.is_null() {
            self.requirement(r, RequirementDisposition::Missing);
            return Ok(());
        }
        if style.owner != self.doc.header.acad_mlinestyle_dict_handle {
            return Err(error(
                SelectionErrorKind::ReferenceConflict,
                Some(k),
                Some(SelectionSlot::StyleRegistration),
                "style owner does not identify source ACAD_MLINESTYLE dictionary",
            ));
        }
        let Some(ObjectType::Dictionary(dictionary)) = self.doc.objects.get(&style.owner) else {
            if self.index.counts.contains_key(&style.owner) {
                return Err(error(
                    SelectionErrorKind::ReferenceConflict,
                    Some(k),
                    Some(SelectionSlot::StyleRegistration),
                    "style dictionary handle has an incompatible role",
                ));
            }
            self.requirement(r, RequirementDisposition::Missing);
            return Ok(());
        };
        self.index.unique(
            key(SelectionRecordKind::Object, style.owner),
            Some(SelectionSlot::StyleRegistration),
        )?;
        // The header is a hint; the named-object dictionary establishes this role.
        let nod_handle = self.doc.header.named_objects_dict_handle;
        let Some(ObjectType::Dictionary(nod)) = self.doc.objects.get(&nod_handle) else {
            if self.index.counts.contains_key(&nod_handle) {
                return Err(error(
                    SelectionErrorKind::ReferenceConflict,
                    Some(k),
                    Some(SelectionSlot::StyleRegistration),
                    "named-object dictionary handle has an incompatible role",
                ));
            }
            self.requirement(r, RequirementDisposition::Missing);
            return Ok(());
        };
        self.index.unique(
            key(SelectionRecordKind::Object, nod_handle),
            Some(SelectionSlot::StyleRegistration),
        )?;
        if dictionary.owner != nod_handle {
            return Err(error(
                SelectionErrorKind::ReferenceConflict,
                Some(k),
                Some(SelectionSlot::StyleRegistration),
                "MLINESTYLE dictionary is not owned by the named-object dictionary",
            ));
        }
        let registrations: Vec<_> = nod
            .entries
            .iter()
            .filter(|(name, h)| name.eq_ignore_ascii_case("ACAD_MLINESTYLE") || *h == style.owner)
            .collect();
        if registrations.len() > 1 {
            return Err(error(
                SelectionErrorKind::AmbiguousIdentity,
                Some(k),
                Some(SelectionSlot::StyleRegistration),
                "ambiguous ACAD_MLINESTYLE dictionary role",
            ));
        }
        match registrations.first() {
            None => {
                self.requirement(r, RequirementDisposition::Missing);
                return Ok(());
            }
            Some((name, h))
                if name.eq_ignore_ascii_case("ACAD_MLINESTYLE") && *h == style.owner => {}
            _ => {
                return Err(error(
                    SelectionErrorKind::ReferenceConflict,
                    Some(k),
                    Some(SelectionSlot::StyleRegistration),
                    "named-object dictionary and MLINESTYLE role disagree",
                ))
            }
        }
        let entries: Vec<_> = dictionary
            .entries
            .iter()
            .filter(|(name, h)| *h == k.handle || name.to_uppercase() == style.name.to_uppercase())
            .collect();
        if entries.len() > 1 {
            return Err(error(
                SelectionErrorKind::AmbiguousIdentity,
                Some(k),
                Some(SelectionSlot::StyleRegistration),
                "style has ambiguous dictionary registration",
            ));
        }
        match entries.first() {
            None => self.requirement(r, RequirementDisposition::Missing),
            Some((name, h))
                if *h == k.handle && name.to_uppercase() == style.name.to_uppercase() =>
            {
                self.requirement(
                    r,
                    RequirementDisposition::Context(ContextRole::MLineStyleDictionary),
                )
            }
            _ => {
                return Err(error(
                    SelectionErrorKind::ReferenceConflict,
                    Some(k),
                    Some(SelectionSlot::StyleRegistration),
                    "style name/handle and dictionary registration disagree",
                ))
            }
        }
        Ok(())
    }
}
