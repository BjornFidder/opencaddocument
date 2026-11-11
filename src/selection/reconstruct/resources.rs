use crate::io::dxf::code_page::{
    dwg_code_page_index, encode_legacy_string, encoding_from_dwg_code_page,
};
use crate::objects::{Dictionary, ObjectType};
use crate::tables::{Layer, LineType};
use crate::DxfVersion;

use super::super::references::SourceIndex;
use super::*;

pub(super) struct Target {
    pub name: String,
    pub handle: Handle,
}

impl Target {
    fn new(name: &str, handle: Handle) -> Self {
        Self {
            name: name.into(),
            handle,
        }
    }
}

pub(super) enum Binding {
    Included(SelectionRecordKey),
    Existing(Target),
}

impl Binding {
    pub fn name<'a>(&'a self, planned: &'a BTreeMap<SelectionRecordKey, Resource>) -> &'a str {
        match self {
            Self::Included(key) => &planned[key].name,
            Self::Existing(target) => &target.name,
        }
    }

    fn existing_handle(&self, planned: &BTreeMap<SelectionRecordKey, Resource>) -> Option<Handle> {
        match self {
            Self::Included(key) => planned[key].handle,
            Self::Existing(target) => Some(target.handle),
        }
    }

    pub fn handle(&self, allocated: &BTreeMap<SelectionRecordKey, Handle>) -> Option<Handle> {
        match self {
            Self::Included(key) => allocated.get(key).copied(),
            Self::Existing(target) => Some(target.handle),
        }
    }
}

pub(super) struct Resource {
    pub name: String,
    pub handle: Option<Handle>,
    pub payload: Option<SelectionPayload>,
    dependencies: BTreeSet<SelectionRecordKey>,
}

pub(super) struct Catalog<'a> {
    pub doc: &'a Document,
    index: &'a SourceIndex,
    pub request: &'a ReconstructionRequest,
    pub records: BTreeMap<SelectionRecordKey, &'a SelectionPayload>,
    slots: BTreeMap<SelectionRecordKey, Vec<&'a SelectionRequirement>>,
    names: NameInventory,
    // The request fixes every resource action and binding. Only root reachability
    // varies, so both successful plans and blockers can be reused by key.
    pub planned: BTreeMap<SelectionRecordKey, Resource>,
    failures: BTreeMap<SelectionRecordKey, ReconstructionBlocker>,
}

fn fold(name: &str) -> String {
    name.to_uppercase()
}
fn same(a: &str, b: &str) -> bool {
    fold(a) == fold(b)
}

fn simple_layer(v: &Layer) -> bool {
    !v.flags.xref_dependent && v.xref_block_record_handle.is_null() && !v.name.contains('|')
}

fn simple_pattern(v: &LineType) -> bool {
    !v.xref_dependent
        && v.xref_block_record_handle.is_null()
        && !v.name.contains('|')
        && v.pattern_length.is_finite()
        && v.elements
            .iter()
            .all(|e| e.complex.is_none() && e.length.is_finite())
}

pub(super) struct NameInventory {
    used: BTreeSet<(SelectionRecordKind, String)>,
    complete: bool,
}

impl NameInventory {
    pub fn read(doc: &Document) -> Self {
        use SelectionRecordKind::{Layer, LineType, Object};
        let mut used = BTreeSet::new();
        let mut mark = |kind, name: &str| {
            used.insert((kind, fold(name)));
        };
        mark(Layer, &doc.header.current_layer_name);
        mark(Object, &doc.header.multiline_style);
        for name in [
            &doc.header.current_linetype_name,
            &doc.header.dim_linetype_name,
            &doc.header.dim_linetype1_name,
            &doc.header.dim_linetype2_name,
        ] {
            mark(LineType, name);
        }
        let mut complete = true;
        for entity in doc.entities() {
            let common = entity.common();
            mark(Layer, &common.layer);
            mark(LineType, &common.linetype);
            if let EntityType::MLine(v) = entity {
                mark(Object, &v.style_name);
            }
            complete &= matches!(
                entity,
                EntityType::Line(_)
                    | EntityType::MLine(_)
                    | EntityType::Block(_)
                    | EntityType::BlockEnd(_)
                    // Viewport resource links outside the common fields are
                    // handles or belong to the separate plot-style domain.
                    | EntityType::Viewport(_)
            ) && common.raw_record.is_none()
                && common.extended_data.records().is_empty()
                && common.extended_data.raw_dwg_eed.is_empty();
        }
        for layer in doc.layers.iter() {
            mark(Layer, &layer.name);
            mark(LineType, &layer.line_type);
            complete &= simple_layer(layer);
        }
        for pattern in doc.line_types.iter() {
            mark(LineType, &pattern.name);
            complete &= simple_pattern(pattern);
        }
        let style_dict = doc.header.acad_mlinestyle_dict_handle;
        let annotation_variables =
            doc.get_subdictionary("AcDbVariableDictionary")
                .and_then(|handle| match doc.objects.get(&handle) {
                    Some(ObjectType::Dictionary(dictionary)) => Some(dictionary),
                    _ => None,
                });
        for (&key, object) in &doc.objects {
            match object {
                ObjectType::MLineStyle(style) => {
                    mark(Object, &style.name);
                    for e in &style.elements {
                        mark(LineType, &e.linetype);
                    }
                }
                ObjectType::Dictionary(d) => {
                    if key == style_dict {
                        for (name, _) in &d.entries {
                            mark(Object, name);
                        }
                    }
                }
                ObjectType::MultiLeaderStyle(_)
                | ObjectType::TableStyle(_)
                | ObjectType::Layout(_)
                | ObjectType::PlaceHolder(_)
                | ObjectType::Scale(_)
                | ObjectType::DictionaryWithDefault(_)
                | ObjectType::SortEntitiesTable(_) => {}
                // These drawing variables name an annotation scale or store
                // flags. Other dictionary variables can carry unknown names.
                ObjectType::DictionaryVariable(variable)
                    if variable.schema_number == 0
                        && matches!(
                            variable.name.as_str(),
                            "CANNOSCALE" | "ANNOTATIVEDWG" | "MSLTSCALE" | "ANNOALLVISIBLE"
                        )
                        && annotation_variables.is_some_and(|dictionary| {
                            dictionary.handle == variable.owner_handle
                                && exact_entry(dictionary, &variable.name, key)
                                && dictionary.entries.iter().filter(|(_, h)| *h == key).count() == 1
                        }) => {}
                _ => complete = false,
            }
        }
        complete &=
            doc.object_xdata.is_empty() && doc.block_records.iter().all(|v| v.xref_path.is_empty());
        Self { used, complete }
    }

    fn check(
        &self,
        doc: &Document,
        key: SelectionRecordKey,
        name: &str,
    ) -> Result<(), ReconstructionBlocker> {
        let code = if !fits_dwg_name(doc, name) {
            Some((
                ReconstructionBlockerKind::NameConflict,
                "resource name exceeds DWG text length",
            ))
        } else if name.trim().is_empty()
            || name.trim() != name
            || name
                .chars()
                .any(|c| c.is_control() || "<>/\\\":;?*|=,".contains(c))
            || self.used.contains(&(key.kind, fold(name)))
        {
            Some((
                ReconstructionBlockerKind::NameConflict,
                "invalid, occupied or already referenced resource name",
            ))
        } else if !self.complete {
            Some((
                ReconstructionBlockerKind::UnsupportedNameCoverage,
                "target has carriers outside the qualified name inventory",
            ))
        } else {
            None
        };
        match code {
            Some((kind, msg)) => Err(blocked(kind, key, None, None, msg)),
            None => Ok(()),
        }
    }
}

fn fits_dwg_name(doc: &Document, name: &str) -> bool {
    // DWG names use signed 16-bit text lengths. Leave room for a terminator.
    const MAX_LENGTH: usize = i16::MAX as usize - 1;
    if name.encode_utf16().take(MAX_LENGTH + 1).count() > MAX_LENGTH {
        return false;
    }
    if doc.version >= DxfVersion::AC1021 {
        return true;
    }
    // Legacy DWG counts encoded bytes, including escapes for unmappable text.
    let encoding = encoding_from_dwg_code_page(dwg_code_page_index(&doc.header.code_page));
    encode_legacy_string(name, encoding).len() <= MAX_LENGTH
}

impl<'a> Catalog<'a> {
    pub fn new(
        selection: &'a DocumentSelection,
        doc: &'a Document,
        index: &'a SourceIndex,
        request: &'a ReconstructionRequest,
        records: BTreeMap<SelectionRecordKey, &'a SelectionPayload>,
    ) -> Self {
        let mut slots = BTreeMap::<_, Vec<_>>::new();
        for req in selection.requirements() {
            slots.entry(req.source).or_default().push(req);
        }
        Self {
            doc,
            index,
            request,
            records,
            slots,
            names: NameInventory::read(doc),
            planned: BTreeMap::new(),
            failures: BTreeMap::new(),
        }
    }

    pub fn direct(&self, kind: SelectionRecordKind, h: Handle) -> Option<Target> {
        if h.is_null()
            || self.index.counts.get(&h) != Some(&1)
            || self.index.inconsistent.contains(&h)
        {
            return None;
        }
        let name = match kind {
            SelectionRecordKind::Layer => {
                let v = self
                    .doc
                    .layers
                    .iter()
                    .find(|v| v.handle == h && simple_layer(v))?;
                (self.doc.layers.get(&v.name)?.handle == h).then_some(&v.name)?
            }
            SelectionRecordKind::LineType => {
                let v = self
                    .doc
                    .line_types
                    .iter()
                    .find(|v| v.handle == h && simple_pattern(v))?;
                (self.doc.line_types.get(&v.name)?.handle == h).then_some(&v.name)?
            }
            _ => return None,
        };
        Some(Target::new(name, h))
    }

    pub fn builtin(&self, role: ContextRole) -> Option<Target> {
        let name = match role {
            ContextRole::ByLayer => "ByLayer",
            ContextRole::ByBlock => "ByBlock",
            _ => return None,
        };
        self.direct(
            SelectionRecordKind::LineType,
            self.doc.line_types.get(name)?.handle,
        )
    }

    pub fn slot(
        &self,
        key: SelectionRecordKey,
        slot: &SelectionSlot,
    ) -> Option<&SelectionRequirement> {
        self.slots
            .get(&key)?
            .iter()
            .find(|req| &req.slot == slot)
            .copied()
    }

    pub fn binding(
        &mut self,
        key: SelectionRecordKey,
        slot: &SelectionSlot,
        kind: SelectionRecordKind,
        used: &mut BTreeSet<SelectionRecordKey>,
    ) -> Result<Binding, ReconstructionBlocker> {
        use ReconstructionBlockerKind as K;
        let error = |code, target, msg| blocked(code, key, Some(slot.clone()), target, msg);
        let req = self
            .slot(key, slot)
            .ok_or_else(|| error(K::MissingDependency, None, "required slot is absent"))?;
        match req.disposition {
            RequirementDisposition::Included(dep) => {
                let resource = self.resource(dep)?;
                used.insert(dep);
                used.extend(resource.dependencies.iter().copied());
                Ok(Binding::Included(dep))
            }
            RequirementDisposition::Context(role) => self
                .builtin(role)
                .map(Binding::Existing)
                .ok_or_else(|| error(K::InvalidBinding, None, "required builtin is unavailable")),
            RequirementDisposition::Missing => {
                Err(error(K::MissingDependency, None, "unresolved capture gap"))
            }
            RequirementDisposition::External(_) if kind == SelectionRecordKind::Object => {
                Err(error(
                    K::IncompatibleResource,
                    None,
                    "MLINE needs an included style definition",
                ))
            }
            RequirementDisposition::External(_) => {
                let h = self
                    .request
                    .external
                    .iter()
                    .find(|b| b.source == key && b.slot == *slot)
                    .map(|b| b.target)
                    .ok_or_else(|| {
                        error(
                            K::MissingBinding,
                            None,
                            "no binding for external requirement",
                        )
                    })?;
                self.direct(kind, h).map(Binding::Existing).ok_or_else(|| {
                    error(
                        K::InvalidBinding,
                        Some(h),
                        "external target is stale or incompatible",
                    )
                })
            }
        }
    }

    fn resource(&mut self, key: SelectionRecordKey) -> Result<&Resource, ReconstructionBlocker> {
        if let Some(failure) = self.failures.get(&key) {
            return Err(failure.clone());
        }
        if !self.planned.contains_key(&key) {
            match self.plan_resource(key) {
                Ok(resource) => {
                    self.planned.insert(key, resource);
                }
                Err(failure) => {
                    self.failures.insert(key, failure.clone());
                    return Err(failure);
                }
            }
        }
        Ok(&self.planned[&key])
    }

    fn plan_resource(
        &mut self,
        key: SelectionRecordKey,
    ) -> Result<Resource, ReconstructionBlocker> {
        use ReconstructionBlockerKind as K;
        let fail = |code, target, message| blocked(code, key, None, target, message);
        let action = self.request.resources.get(&key).ok_or_else(|| {
            fail(
                K::MissingResourceAction,
                None,
                "choose explicit reuse or creation",
            )
        })?;
        let mut payload = self.records[&key].clone();
        let mut references = Vec::new();
        let mut dependencies = BTreeSet::new();
        match &mut payload {
            SelectionPayload::Layer(v) => {
                let b = self.binding(
                    key,
                    &SelectionSlot::LineType,
                    SelectionRecordKind::LineType,
                    &mut dependencies,
                )?;
                b.name(&self.planned).clone_into(&mut v.line_type);
                references.push(b);
            }
            SelectionPayload::MLineStyle(v) => {
                if self
                    .slot(key, &SelectionSlot::StyleRegistration)
                    .is_none_or(|r| {
                        r.disposition
                            != RequirementDisposition::Context(ContextRole::MLineStyleDictionary)
                    })
                {
                    return Err(fail(
                        K::MissingDependency,
                        None,
                        "source style registration is missing",
                    ));
                }
                if !v.start_angle.is_finite()
                    || !v.end_angle.is_finite()
                    || v.elements.iter().any(|e| !e.offset.is_finite())
                {
                    return Err(fail(K::InvalidGeometry, None, "nonfinite style parameters"));
                }
                for (i, e) in v.elements.iter_mut().enumerate() {
                    let b = self.binding(
                        key,
                        &SelectionSlot::StyleElementLineType(i),
                        SelectionRecordKind::LineType,
                        &mut dependencies,
                    )?;
                    b.name(&self.planned).clone_into(&mut e.linetype);
                    references.push(b);
                }
            }
            SelectionPayload::LineType(v) if simple_pattern(v) => {}
            _ => {
                return Err(fail(
                    K::IncompatibleResource,
                    None,
                    "resource is outside supported payload scope",
                ))
            }
        }
        let (name, handle, payload) = match action {
            ResourceAction::Create { name } => {
                self.names.check(self.doc, key, name)?;
                (name.clone(), None, Some(payload))
            }
            ResourceAction::Reuse(h) => {
                let target = self
                    .compatible(key, *h, payload, &references)
                    .ok_or_else(|| {
                        fail(
                            K::IncompatibleResource,
                            Some(*h),
                            "target resource differs from captured definition after remapping",
                        )
                    })?;
                (target.name, Some(target.handle), None)
            }
        };
        Ok(Resource {
            name,
            handle,
            payload,
            dependencies,
        })
    }

    fn compatible(
        &self,
        key: SelectionRecordKey,
        h: Handle,
        payload: SelectionPayload,
        refs: &[Binding],
    ) -> Option<Target> {
        if refs
            .iter()
            .any(|r| r.existing_handle(&self.planned).is_none())
            || self
                .index
                .audit(
                    self.doc,
                    SelectionRecordKey {
                        kind: key.kind,
                        handle: h,
                    },
                )
                .is_err()
        {
            return None;
        }
        match payload {
            SelectionPayload::Layer(mut v) => {
                let binding = self.direct(key.kind, h)?;
                let target = self.doc.layers.get(&binding.name)?;
                let pattern = self.doc.line_types.get(&target.line_type)?;
                if refs.first()?.existing_handle(&self.planned) != Some(pattern.handle) {
                    return None;
                }
                v.handle = h;
                v.name.clone_from(&target.name);
                v.line_type.clone_from(&target.line_type);
                (*v == *target).then_some(binding)
            }
            SelectionPayload::LineType(mut v) => {
                let binding = self.direct(key.kind, h)?;
                let target = self.doc.line_types.get(&binding.name)?;
                v.handle = h;
                v.name.clone_from(&target.name);
                (*v == *target
                    && v.pattern_length.to_bits() == target.pattern_length.to_bits()
                    && v.elements
                        .iter()
                        .zip(&target.elements)
                        .all(|(a, b)| a.length.to_bits() == b.length.to_bits()))
                .then_some(binding)
            }
            SelectionPayload::MLineStyle(mut v) => {
                let dict = style_dictionary(self.doc).ok()??;
                let ObjectType::MLineStyle(target) = self.doc.objects.get(&h)? else {
                    return None;
                };
                let ObjectType::Dictionary(dictionary) = self.doc.objects.get(&dict)? else {
                    return None;
                };
                if target.owner != dict || !exact_entry(dictionary, &target.name, h) ||
                    v.elements.len() != target.elements.len() ||
                    self.doc.objects.values().filter(|o|
                        matches!(o, ObjectType::MLineStyle(style) if same(&style.name, &target.name))).count() != 1 {
                    return None;
                }
                for ((element, current), reference) in
                    v.elements.iter_mut().zip(&target.elements).zip(refs)
                {
                    let lt = self.doc.line_types.get(&current.linetype)?;
                    if reference.existing_handle(&self.planned) != Some(lt.handle)
                        || element.offset.to_bits() != current.offset.to_bits()
                    {
                        return None;
                    }
                    element.linetype.clone_from(&current.linetype);
                }
                v.handle = h;
                v.owner = dict;
                v.name.clone_from(&target.name);
                (*v == *target
                    && v.start_angle.to_bits() == target.start_angle.to_bits()
                    && v.end_angle.to_bits() == target.end_angle.to_bits())
                .then(|| Target::new(&target.name, h))
            }
            _ => None,
        }
    }
}

fn exact_entry(dict: &Dictionary, name: &str, h: Handle) -> bool {
    let mut entries = dict.entries.iter().filter(|(n, _)| same(n, name));
    entries
        .next()
        .is_some_and(|(n, target)| n == name && *target == h)
        && entries.next().is_none()
}

pub(super) fn style_dictionary(doc: &Document) -> Result<Option<Handle>, ReconstructionError> {
    let fail =
        || ReconstructionError::UnsafeTarget("inconsistent ACAD_MLINESTYLE registration".into());
    let nod = doc.named_objects_dictionary().ok_or_else(fail)?;
    if nod.handle.is_null()
        || !nod.owner.is_null()
        || nod.handle != doc.header.named_objects_dict_handle
    {
        return Err(fail());
    }
    let h = doc.header.acad_mlinestyle_dict_handle;
    if h.is_null() {
        if nod
            .entries
            .iter()
            .any(|(name, _)| same(name, "ACAD_MLINESTYLE"))
        {
            return Err(fail());
        }
        return Ok(None);
    }
    let Some(ObjectType::Dictionary(d)) = doc.objects.get(&h) else {
        return Err(fail());
    };
    if d.handle != h || d.owner != nod.handle || !exact_entry(nod, "ACAD_MLINESTYLE", h) {
        return Err(fail());
    }
    Ok(Some(h))
}

pub(super) fn create_style_dictionary(
    doc: &mut Document,
    h: Handle,
) -> Result<(), ReconstructionError> {
    let nod = doc
        .named_objects_dictionary_mut()
        .ok_or_else(|| construction("missing NOD"))?;
    let mut dict = Dictionary::new();
    dict.handle = h;
    dict.owner = nod.handle;
    nod.add_entry("ACAD_MLINESTYLE", h);
    doc.header.acad_mlinestyle_dict_handle = h;
    doc.objects.insert(h, ObjectType::Dictionary(dict));
    Ok(())
}

pub(super) fn insert(
    doc: &mut Document,
    resource: &mut Resource,
    h: Handle,
    dictionary: Option<Handle>,
) -> Result<(), ReconstructionError> {
    match resource.payload.take() {
        Some(SelectionPayload::LineType(mut v)) => {
            v.handle = h;
            v.name.clone_from(&resource.name);
            doc.line_types.add(*v).map_err(construction)?;
        }
        Some(SelectionPayload::Layer(mut v)) => {
            v.handle = h;
            v.name.clone_from(&resource.name);
            doc.layers.add(*v).map_err(construction)?;
        }
        Some(SelectionPayload::MLineStyle(mut v)) => {
            let owner =
                dictionary.ok_or_else(|| construction("missing MLINESTYLE dictionary handle"))?;
            let Some(ObjectType::Dictionary(d)) = doc.objects.get_mut(&owner) else {
                return Err(construction("missing MLINESTYLE dictionary"));
            };
            v.handle = h;
            v.owner = owner;
            v.name.clone_from(&resource.name);
            d.add_entry(v.name.clone(), h);
            doc.objects.insert(h, ObjectType::MLineStyle(*v));
        }
        _ => return Err(construction("unplanned resource creation")),
    }
    Ok(())
}

pub(super) fn name_key(key: SelectionRecordKey, name: &str) -> (SelectionRecordKind, String) {
    (key.kind, fold(name))
}
