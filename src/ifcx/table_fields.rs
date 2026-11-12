//! Stored field graphs for table cells. Rebinding never evaluates a field.
use super::*;
use acadrust::entities::table::CellValue;
use acadrust::objects::{Field, FieldChildValue, ObjectType};
use ifccad::{
    IfccadAttributeFieldCodePart as AttributeCode, IfccadTableCellReference as Cells,
    IfccadTableFieldAuxiliaryValue as Auxiliary, IfccadTableFieldCache as Cache,
    IfccadTableFieldCodePart as Part, IfccadTableFieldGraph as Graph,
    IfccadTableFieldRecord as Record, IfccadTableRange as Range, IfccadTableValue as Value,
};

pub(super) fn capture(document: &Document, root: Handle, owner: Handle) -> Result<Graph, Error> {
    let mut pending = vec![(root, owner)];
    let mut ids = HashMap::new();
    let mut fields = Vec::new();
    while let Some((handle, owner)) = pending.pop() {
        let Some(ObjectType::Field(field)) = document.objects.get(&handle) else {
            return Err(annotation::invalid("missing table FIELD object"));
        };
        let id = fields.len() as u64 + 1;
        if ids.insert(handle, id).is_some() || field.owner != owner {
            return Err(annotation::invalid("invalid table FIELD ownership"));
        }
        pending.extend(
            field
                .child_fields
                .iter()
                .rev()
                .map(|child| (*child, handle)),
        );
        fields.push(field);
    }
    let records = fields
        .into_iter()
        .map(|field| {
            let mut code = Vec::new();
            let mut object_targets: Vec<_> =
                field.referenced_objects.iter().map(Handle::value).collect();
            for part in attribute_fields::code_out(
                &field.code,
                &field.child_fields,
                &field.referenced_objects,
                &ids,
                false,
            )? {
                match part {
                    AttributeCode::Child(id) => code.push(Part::Child(id)),
                    AttributeCode::Object(id) => {
                        if !object_targets.contains(&id) {
                            object_targets.push(id);
                        }
                        code.push(Part::Object(id));
                    }
                    AttributeCode::Literal(text) => code.push(Part::Literal(text)),
                    AttributeCode::CurrentBlockReference => {
                        return Err(annotation::invalid(
                            "table FIELD uses current INSERT context",
                        ));
                    }
                }
            }
            if field.evaluator_id.eq_ignore_ascii_case("AcExpr") {
                code = formula_references(code);
            }
            Ok(Record {
                id: ids[&field.handle],
                evaluator: field.evaluator_id.clone(),
                code,
                children: field
                    .child_fields
                    .iter()
                    .map(|handle| ids[handle])
                    .collect(),
                object_targets,
                format: field.format.clone(),
                evaluation_options: field.evaluation_option,
                filing_options: field.filing_option,
                state: field.state,
                evaluation_status: field.evaluation_status,
                error_code: field.evaluation_error_code,
                error_message: field.evaluation_error_message.clone(),
                cache: cache_out(&field.value, &field.value_string)?,
                auxiliary_values: field
                    .child_values
                    .iter()
                    .map(|value| {
                        Ok(Auxiliary {
                            key: value.key.clone(),
                            cache: cache_out(&value.value, "")?,
                        })
                    })
                    .collect::<Result<_, Error>>()?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Graph {
        root: ids[&root],
        next_id: ids.len() as u64 + 1,
        records,
    })
}

fn cache_out(value: &CellValue, display: &str) -> Result<Cache, Error> {
    Ok(Cache {
        value: table::value_out(value)?,
        display: display.into(),
        unit: table::unit_out(value.unit_type),
        value_format: value.format.clone(),
        formatted_value: value.formatted_value.clone(),
        flags: value.flags,
    })
}

fn formula_references(mut code: Vec<Part>) -> Vec<Part> {
    // Object binding was parsed first, so cross-table references are a
    // literal / object / literal triple before cell addresses are tokenized.
    let mut index = 0;
    while index + 2 < code.len() {
        let [Part::Literal(left), Part::Object(target), Part::Literal(right)] =
            &code[index..index + 3]
        else {
            index += 1;
            continue;
        };
        let Some(left) = left
            .strip_suffix("Table(")
            .or_else(|| left.strip_suffix("table("))
        else {
            index += 1;
            continue;
        };
        let Some(right) = right.strip_prefix(").") else {
            index += 1;
            continue;
        };
        let Some((end, range)) = cell_range(right) else {
            index += 1;
            continue;
        };
        let replacement = [
            Part::Literal(left.to_owned()),
            Part::Cells(Cells {
                table_id: Some(*target),
                range,
            }),
            Part::Literal(right[end..].to_owned()),
        ];
        code.splice(index..index + 3, replacement);
        index += 2;
    }
    let mut out = Vec::new();
    for part in code {
        let Part::Literal(text) = part else {
            out.push(part);
            continue;
        };
        let mut start = 0;
        let mut cursor = 0;
        let mut quoted = false;
        while cursor < text.len() {
            let ch = text[cursor..].chars().next().unwrap();
            if ch == '"' {
                quoted = !quoted;
            }
            let boundary = cursor == 0 || !text.as_bytes()[cursor - 1].is_ascii_alphanumeric();
            if !quoted && boundary && !text[..cursor].ends_with('\\') {
                if let Some((length, range)) = cell_range(&text[cursor..]) {
                    if start < cursor {
                        out.push(Part::Literal(text[start..cursor].to_owned()));
                    }
                    out.push(Part::Cells(Cells {
                        table_id: None,
                        range,
                    }));
                    cursor += length;
                    start = cursor;
                    continue;
                }
            }
            cursor += ch.len_utf8();
        }
        if start < text.len() {
            out.push(Part::Literal(text[start..].to_owned()));
        }
    }
    out
}

fn cell_range(text: &str) -> Option<(usize, Range)> {
    fn cell(text: &str) -> Option<(usize, usize, usize)> {
        let letters = text.bytes().take_while(u8::is_ascii_alphabetic).count();
        if letters == 0 {
            return None;
        }
        let digits = text[letters..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        let end = letters + digits;
        if digits == 0
            || text
                .as_bytes()
                .get(end)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return None;
        }
        let col = text[..letters]
            .bytes()
            .try_fold(0_usize, |n, b| {
                n.checked_mul(26)?
                    .checked_add(usize::from(b.to_ascii_uppercase() - b'A' + 1))
            })?
            .checked_sub(1)?;
        let row = text[letters..end].parse::<usize>().ok()?.checked_sub(1)?;
        Some((end, row, col))
    }
    let (first, row, col) = cell(text)?;
    let (end, bottom, right) = if let Some(next) = text[first..].strip_prefix(':') {
        let (second, row, col) = cell(next)?;
        (first + 1 + second, row, col)
    } else {
        (first, row, col)
    };
    Some((
        end,
        Range {
            top_row: row.min(bottom),
            left_column: col.min(right),
            bottom_row: row.max(bottom),
            right_column: col.max(right),
        },
    ))
}

pub(super) fn bind_targets(graph: &mut Graph, ids: &HashMap<Handle, u64>) -> Result<(), Error> {
    let bind = |id: &mut u64| {
        *id = ids
            .get(&Handle::from(*id))
            .copied()
            .ok_or_else(|| annotation::invalid("missing native table FIELD target"))?;
        Ok::<_, Error>(())
    };
    for record in &mut graph.records {
        for id in &mut record.object_targets {
            bind(id)?;
        }
        for code in &mut record.code {
            match code {
                Part::Object(id) => bind(id)?,
                Part::Cells(Cells {
                    table_id: Some(id), ..
                }) => bind(id)?,
                _ => {}
            }
        }
        for cache in std::iter::once(&mut record.cache).chain(
            record
                .auxiliary_values
                .iter_mut()
                .map(|value| &mut value.cache),
        ) {
            if let Value::Object(id) = &mut cache.value {
                bind(id)?;
            }
        }
    }
    Ok(())
}

pub(super) fn restore(
    document: &mut Document,
    graph: &Graph,
    owner: Handle,
    targets: &HashMap<u64, Handle>,
) -> Result<Handle, Error> {
    let handles: HashMap<_, _> = graph
        .records
        .iter()
        .map(|record| (record.id, document.allocate_handle()))
        .collect();
    let mut owners = HashMap::from([(graph.root, owner)]);
    for record in &graph.records {
        for child in &record.children {
            owners.insert(*child, handles[&record.id]);
        }
    }
    let mut fields = Vec::with_capacity(graph.records.len());
    for record in &graph.records {
        let mut objects = record
            .object_targets
            .iter()
            .map(|id| lookup(targets, *id))
            .collect::<Result<Vec<_>, _>>()?;
        let mut code = String::new();
        for part in &record.code {
            match part {
                Part::Literal(text) => code.push_str(text),
                Part::Child(id) => {
                    let index = record
                        .children
                        .iter()
                        .position(|child| child == id)
                        .ok_or_else(|| annotation::invalid("missing table FIELD child index"))?;
                    code.push_str(&format!("%<\\_FldIdx {index}>%"));
                }
                Part::Object(id) => code.push_str(&object_code(*id, targets, &mut objects)?),
                Part::Cells(reference) => {
                    if let Some(id) = reference.table_id {
                        code.push_str(&format!(
                            "Table({}).",
                            object_code(id, targets, &mut objects)?
                        ));
                    }
                    let range = reference.range;
                    code.push_str(&cell_address(range.top_row, range.left_column));
                    if range.top_row != range.bottom_row || range.left_column != range.right_column
                    {
                        code.push(':');
                        code.push_str(&cell_address(range.bottom_row, range.right_column));
                    }
                }
            }
        }
        fields.push(Field {
            handle: handles[&record.id],
            owner: owners[&record.id],
            evaluator_id: record.evaluator.clone(),
            code,
            format: record.format.clone(),
            child_fields: record.children.iter().map(|id| handles[id]).collect(),
            referenced_objects: objects,
            evaluation_option: record.evaluation_options,
            filing_option: record.filing_options,
            state: record.state,
            evaluation_status: record.evaluation_status,
            evaluation_error_code: record.error_code,
            evaluation_error_message: record.error_message.clone(),
            value: cache_in(&record.cache, targets)?,
            value_string: record.cache.display.clone(),
            value_string_length: record.cache.display.encode_utf16().count() as i32,
            child_values: record
                .auxiliary_values
                .iter()
                .map(|aux| {
                    Ok(FieldChildValue {
                        key: aux.key.clone(),
                        value: cache_in(&aux.cache, targets)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            ..Default::default()
        });
    }
    for (dxf, cpp) in [("FIELD", "AcDbField"), ("FIELDLIST", "AcDbFieldList")] {
        if !document.classes.contains(dxf) {
            let mut class = acadrust::classes::DxfClass::new(dxf, cpp);
            class.proxy_flags = acadrust::classes::ProxyFlags(1152);
            document.classes.add_or_update(class);
        }
    }
    document.register_fields(fields);
    Ok(handles[&graph.root])
}

fn cache_in(cache: &Cache, targets: &HashMap<u64, Handle>) -> Result<CellValue, Error> {
    let mut value = table::value_in(&cache.value)?;
    if let Value::Object(id) = cache.value {
        value.handle_value = Some(lookup(targets, id)?);
    }
    value.unit_type = table::unit_in(cache.unit);
    value.flags = cache.flags;
    value.format = cache.value_format.clone();
    value.formatted_value = cache.formatted_value.clone();
    Ok(value)
}

fn lookup(targets: &HashMap<u64, Handle>, id: u64) -> Result<Handle, Error> {
    targets
        .get(&id)
        .copied()
        .ok_or_else(|| annotation::invalid("missing CAD table FIELD target"))
}

fn object_code(
    id: u64,
    targets: &HashMap<u64, Handle>,
    objects: &mut Vec<Handle>,
) -> Result<String, Error> {
    let handle = lookup(targets, id)?;
    let index = objects
        .iter()
        .position(|target| *target == handle)
        .unwrap_or_else(|| {
            objects.push(handle);
            objects.len() - 1
        });
    Ok(format!("%<\\_ObjIdx {index}>%"))
}

fn cell_address(row: usize, mut col: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push(char::from(b'A' + (col % 26) as u8));
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    let column: String = letters.into_iter().rev().collect();
    format!("{column}{}", row + 1)
}
