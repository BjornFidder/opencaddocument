//! Selected FIELD storage. Capture/reconstruction never evaluate or stamp text.
use super::*;
use acadrust::{
    document::FieldDef,
    entities::{CellValue, CellValueType, ValueUnitType},
    objects::{Field, FieldChildValue, FieldList, ObjectType},
};
use ifccad::{
    IfccadAttributeFieldAuxiliaryValue as Auxiliary, IfccadAttributeFieldCache as Cache,
    IfccadAttributeFieldCodePart as Part, IfccadAttributeFieldEvaluator as Evaluator,
    IfccadAttributeFieldGraph as Graph, IfccadAttributeFieldRecord as Record,
    IfccadAttributeFieldValue as Value,
};
use std::collections::{BTreeSet, HashMap};

fn invalid(m: &str) -> Error {
    annotation::invalid(m)
}
fn cache_out(v: &CellValue, display: &str) -> Result<Cache, Error> {
    let value = match v.value_type {
        CellValueType::Unknown if v.raw_type_code == 0 => Value::Empty,
        CellValueType::String => Value::Text(v.text.clone()),
        CellValueType::Long
            if v.numeric_value.is_finite()
                && v.numeric_value.fract() == 0.
                && v.numeric_value.abs() <= 9007199254740992. =>
        {
            Value::Integer(v.numeric_value as i64)
        }
        CellValueType::Double if v.numeric_value.is_finite() => Value::Real(v.numeric_value),
        CellValueType::Handle => Value::Object(
            v.handle_value
                .ok_or_else(|| invalid("empty field object cache"))?
                .value(),
        ),
        _ => {
            return Err(invalid(&format!(
                "attribute field has unsupported cached value type {:?}, raw {}, numeric {}",
                v.value_type, v.raw_type_code, v.numeric_value
            )))
        }
    };
    if (v.handle_value.is_some() && v.value_type != CellValueType::Handle)
        || !v.binary_value.is_empty()
    {
        return Err(invalid("attribute field cache has unsupported payload"));
    }
    Ok(Cache {
        value,
        display: display.into(),
        flags: v.flags,
        unit_type: v.raw_unit_type_code,
        format: v.format.clone(),
        formatted_value: v.formatted_value.clone(),
    })
}
fn cache_in(v: &Cache, objects: &HashMap<u64, Handle>) -> Result<CellValue, Error> {
    let mut result = match &v.value {
        Value::Object(id) => {
            let mut c = CellValue::new();
            c.value_type = CellValueType::Handle;
            c.raw_type_code = 64;
            c.handle_value = Some(
                *objects
                    .get(id)
                    .ok_or_else(|| invalid("missing field cache object"))?,
            );
            c
        }
        Value::Empty => CellValue::new(),
        Value::Text(t) => CellValue::text(t),
        Value::Integer(i) if i.unsigned_abs() <= 9007199254740992 => CellValue::integer(*i),
        Value::Real(n) => CellValue::number(*n),
        Value::Boolean(_) => {
            return Err(invalid(
                "boolean attribute field cache has no selected CAD type",
            ))
        }
        _ => {
            return Err(invalid(
                "integer attribute field cache exceeds exact CAD numeric range",
            ))
        }
    };
    result.flags = v.flags;
    result.raw_unit_type_code = v.unit_type;
    result.unit_type = ValueUnitType::from(v.unit_type as u32);
    result.format = v.format.clone();
    result.formatted_value = v.formatted_value.clone();
    Ok(result)
}
fn code_out(
    code: &str,
    children: &[Handle],
    objects: &[Handle],
    ids: &HashMap<Handle, u64>,
    definition: bool,
) -> Result<Vec<Part>, Error> {
    let mut out = vec![];
    let mut literal = String::new();
    let mut rest = code;
    while !rest.is_empty() {
        let mut token = None;
        for (prefix, kind) in [("%<\\_FldIdx ", 0), ("%<\\_ObjIdx ", 1), ("%<\\_ObjId ", 2)] {
            if let Some(t) = rest.strip_prefix(prefix) {
                let end = t
                    .find(">%")
                    .ok_or_else(|| invalid("unterminated attribute field reference"))?;
                let n: u64 = t[..end]
                    .trim()
                    .parse()
                    .map_err(|_| invalid("invalid attribute field index"))?;
                let part = match kind {
                    0 => Part::Child(
                        *ids.get(
                            children
                                .get(n as usize)
                                .ok_or_else(|| invalid("unresolved field child slot"))?,
                        )
                        .ok_or_else(|| invalid("missing field child"))?,
                    ),
                    1 => Part::Object(
                        objects
                            .get(n as usize)
                            .ok_or_else(|| invalid("unresolved field object slot"))?
                            .value(),
                    ),
                    _ => Part::Object(n),
                };
                token = Some((prefix.len() + end + 2, part));
                break;
            }
        }
        if rest.starts_with("?BlockRefId") {
            if !definition {
                return Err(invalid("unresolved block-instance field reference"));
            }
            token = Some(("?BlockRefId".len(), Part::CurrentBlockReference));
        }
        if let Some((len, part)) = token {
            if !literal.is_empty() {
                out.push(Part::Literal(std::mem::take(&mut literal)));
            }
            out.push(part);
            rest = &rest[len..];
        } else {
            let ch = rest.chars().next().unwrap();
            literal.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    if !literal.is_empty() {
        out.push(Part::Literal(literal));
    }
    Ok(out)
}
pub(super) fn capture(
    w: &mut Writer<'_>,
    host: Handle,
    definition: bool,
) -> Result<Option<Graph>, Error> {
    if host.is_null() {
        return Ok(None);
    }
    let mut roots = vec![];
    for o in w.document.objects.values() {
        if let ObjectType::Dictionary(d) = o {
            if d.owner == host {
                if let Some(h) = d.get("ACAD_FIELD") {
                    let Some(ObjectType::Dictionary(fd)) = w.document.objects.get(&h) else {
                        return Err(invalid("unresolved attribute field dictionary"));
                    };
                    if fd.entries.len() != 1 {
                        return Err(invalid(
                            "attribute has unsupported additional field dictionary entries",
                        ));
                    }
                    roots.push(
                        fd.get("TEXT")
                            .ok_or_else(|| invalid("attribute field has no TEXT root"))?,
                    );
                }
            }
        }
    }
    if roots.is_empty() {
        return Ok(None);
    }
    if roots.len() != 1 {
        return Err(invalid("multiple attribute field roots"));
    }
    let root = roots[0];
    let mut ids = HashMap::new();
    let mut pending = vec![root];
    let mut ordered = vec![];
    while let Some(h) = pending.pop() {
        if ids.contains_key(&h) {
            continue;
        }
        let Some(ObjectType::Field(f)) = w.document.objects.get(&h) else {
            return Err(invalid("unresolved attribute field object"));
        };
        ids.insert(h, ordered.len() as u64);
        ordered.push(h);
        for c in f.child_fields.iter().rev() {
            let Some(ObjectType::Field(child)) = w.document.objects.get(c) else {
                return Err(invalid("unresolved field child"));
            };
            if child.owner != h {
                return Err(invalid("field child ownership mismatch"));
            }
            pending.push(*c);
        }
    }
    let mut records = vec![];
    for h in ordered {
        let ObjectType::Field(f) = &w.document.objects[&h] else {
            unreachable!()
        };
        let evaluator = match f.evaluator_id.as_str() {
            "_text" => Evaluator::TextTemplate,
            "AcVar" | "AcVar.16.2" if f.code.contains("Filename") => Evaluator::DrawingVariable,
            "AcObjProp" | "AcObjProp.16.2" if f.code.contains(".Length") => {
                Evaluator::ObjectProperty
            }
            _ => {
                return Err(invalid(
                    "attribute field evaluator is outside selected Filename/Length storage",
                ))
            }
        };
        if !f.xdata.records().is_empty() {
            return Err(invalid("attribute field XData is outside selected storage"));
        }
        records.push(Record {
            id: ids[&h],
            evaluator,
            source_evaluator: f.evaluator_id.clone(),
            code: code_out(
                &f.code,
                &f.child_fields,
                &f.referenced_objects,
                &ids,
                definition,
            )?,
            format: f.format.clone(),
            cache: cache_out(&f.value, &f.value_string)
                .map_err(|e| invalid(&format!("field {} / {}: {e}", f.evaluator_id, f.code)))?,
            evaluation_options: f.evaluation_option,
            filing_options: f.filing_option,
            state: f.state,
            evaluation_status: f.evaluation_status,
            error_code: f.evaluation_error_code,
            error_message: f.evaluation_error_message.clone(),
            auxiliary_values: f
                .child_values
                .iter()
                .map(|v| {
                    Ok(Auxiliary {
                        key: v.key.clone(),
                        cache: cache_out(&v.value, "")
                            .map_err(|e| invalid(&format!("field auxiliary {}: {e}", v.key)))?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        });
    }
    Ok(Some(Graph {
        root: 0,
        next_id: records.len() as u64,
        records,
    }))
}
pub(super) fn bind(graph: &mut Option<Graph>, ids: &HashMap<Handle, u64>) -> Result<(), Error> {
    if let Some(g) = graph {
        for r in &mut g.records {
            for c in std::iter::once(&mut r.cache)
                .chain(r.auxiliary_values.iter_mut().map(|a| &mut a.cache))
            {
                if let Value::Object(id) = &mut c.value {
                    *id = *ids
                        .get(&Handle::new(*id))
                        .ok_or_else(|| invalid("field cache target is not represented natively"))?;
                }
            }
            for p in &mut r.code {
                if let Part::Object(id) = p {
                    *id = *ids.get(&Handle::new(*id)).ok_or_else(|| {
                        invalid("attribute field target is not represented natively")
                    })?;
                }
            }
        }
    }
    Ok(())
}
pub(super) fn reconstruct(
    doc: &mut Document,
    host: Handle,
    g: Option<&Graph>,
    objects: &HashMap<u64, Handle>,
) -> Result<(), Error> {
    let Some(g) = g else { return Ok(()) };
    for (dxf, cpp) in [("FIELD", "AcDbField"), ("FIELDLIST", "AcDbFieldList")] {
        if !doc.classes.contains(dxf) {
            let mut c = acadrust::classes::DxfClass::new(dxf, cpp);
            c.proxy_flags = acadrust::classes::ProxyFlags(1152);
            doc.classes.add_or_update(c);
        }
    }
    let extension = doc.ensure_extension_dictionary(host);
    super::attributes::common_mut(doc, host)
        .ok_or_else(|| invalid("missing field host"))?
        .xdictionary_handle = Some(extension);
    let dictionary = annotation::child_dictionary(doc, extension, "ACAD_FIELD");
    let handles: HashMap<_, _> = g
        .records
        .iter()
        .map(|r| (r.id, doc.allocate_handle()))
        .collect();
    if let Some(ObjectType::Dictionary(d)) = doc.objects.get_mut(&dictionary) {
        d.insert("TEXT", handles[&g.root]);
    }
    let mut parents = HashMap::new();
    for r in &g.records {
        for p in &r.code {
            if let Part::Child(c) = p {
                parents.insert(*c, r.id);
            }
        }
    }
    let mut all = vec![];
    for r in &g.records {
        let mut child_fields = vec![];
        let mut referenced_objects = vec![];
        let mut code = String::new();
        for p in &r.code {
            match p {
                Part::Literal(s) => code.push_str(s),
                Part::Child(c) => {
                    let h = handles[c];
                    let index = child_fields
                        .iter()
                        .position(|x| *x == h)
                        .unwrap_or_else(|| {
                            child_fields.push(h);
                            child_fields.len() - 1
                        });
                    code.push_str(&format!("%<\\_FldIdx {index}>%"));
                }
                Part::Object(id) => {
                    let h = *objects
                        .get(id)
                        .ok_or_else(|| invalid("missing reconstructed field object"))?;
                    let index = referenced_objects
                        .iter()
                        .position(|x| *x == h)
                        .unwrap_or_else(|| {
                            referenced_objects.push(h);
                            referenced_objects.len() - 1
                        });
                    code.push_str(&format!("%<\\_ObjIdx {index}>%"));
                }
                Part::CurrentBlockReference => code.push_str("?BlockRefId"),
            }
        }
        let evaluator_id = if r.source_evaluator.is_empty() {
            match r.evaluator {
                Evaluator::TextTemplate => "_text",
                Evaluator::DrawingVariable => "AcVar",
                Evaluator::ObjectProperty => "AcObjProp",
            }
            .into()
        } else {
            r.source_evaluator.clone()
        };
        let h = handles[&r.id];
        let owner = parents.get(&r.id).map(|p| handles[p]).unwrap_or(dictionary);
        let field = Field {
            handle: h,
            owner,
            evaluator_id,
            code,
            format: r.format.clone(),
            child_fields,
            referenced_objects,
            evaluation_option: r.evaluation_options,
            filing_option: r.filing_options,
            state: r.state,
            evaluation_status: r.evaluation_status,
            evaluation_error_code: r.error_code,
            evaluation_error_message: r.error_message.clone(),
            value: cache_in(&r.cache, objects)?,
            value_string: r.cache.display.clone(),
            value_string_length: r.cache.display.encode_utf16().count() as i32,
            child_values: r
                .auxiliary_values
                .iter()
                .map(|a| {
                    Ok(FieldChildValue {
                        key: a.key.clone(),
                        value: cache_in(&a.cache, objects)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            ..Default::default()
        };
        doc.fields.insert(
            h,
            FieldDef {
                handle: h,
                owner,
                evaluator: field.evaluator_id.clone(),
                code: field.code.clone(),
                objects: field.referenced_objects.clone(),
            },
        );
        doc.objects.insert(h, ObjectType::Field(field));
        all.push(h);
    }
    let nod = doc.header.named_objects_dict_handle;
    let existing = doc
        .objects
        .iter()
        .find_map(|(h, o)| matches!(o, ObjectType::FieldList(_)).then_some(*h));
    let list = existing.unwrap_or_else(|| {
        let h = doc.allocate_handle();
        doc.objects.insert(
            h,
            ObjectType::FieldList(FieldList {
                handle: h,
                owner: nod,
                ..Default::default()
            }),
        );
        if let Some(ObjectType::Dictionary(d)) = doc.objects.get_mut(&nod) {
            d.insert("ACAD_FIELDLIST", h);
        }
        h
    });
    if let Some(ObjectType::FieldList(l)) = doc.objects.get_mut(&list) {
        let mut seen: BTreeSet<_> = l.fields.iter().copied().collect();
        for h in all {
            if seen.insert(h) {
                l.fields.push(h);
            }
        }
    }
    Ok(())
}
