//! Drawing-owned data sources and standalone DATATABLE records.
use super::*;
use acadrust::objects::{
    ClassObject, ClassObjectData, DataLink, DataLinkCustomData, DataTable, DataTableCellType,
    DataTableColumn, DataTableValue, Dictionary, ObjectType,
};
use ifccad::{
    IfccadDataTable, IfccadDataTableColumn, IfccadDataTableColumnType as ColumnType,
    IfccadDataTableOwner, IfccadDataTableSettings, IfccadTableDataLink,
    IfccadTableDataLinkSettings, IfccadTableDataLinkTarget, IfccadTableValue as Value,
};

pub(super) fn export(writer: &mut Writer<'_>) -> Result<(), Error> {
    let mut objects: Vec<_> = writer
        .document
        .objects
        .iter()
        .filter_map(|(handle, object)| {
            let ObjectType::ClassObject(object) = object else {
                return None;
            };
            matches!(
                object.data,
                ClassObjectData::DataLink(_) | ClassObjectData::DataTable(_)
            )
            .then_some((*handle, object))
        })
        .collect();
    objects.sort_by_key(|(handle, _)| *handle);
    for (handle, object) in objects {
        match &object.data {
            ClassObjectData::DataLink(data) => {
                let id = writer.drawing.id_counters.allocate_table_data_link_id()?;
                let name = dictionary_entry(writer.document, object.owner, handle)
                    .map(|(name, _)| name.to_owned())
                    .unwrap_or_else(|| format!("Link {}", id.0));
                let date = [
                    data.year,
                    data.month,
                    data.day,
                    data.hour,
                    data.minute,
                    data.second,
                    data.millisecond,
                ];
                let settings = IfccadTableDataLinkSettings {
                    name,
                    adapter: data.data_adapter.clone(),
                    source: data.connection_string.clone(),
                    source_selection: None,
                    description: nonempty(&data.description),
                    tooltip: nonempty(&data.tooltip),
                    options: data.option,
                    update_options: data.update_option,
                    path_option: data.path_option,
                    status_flags: data.status_flags,
                    flags: data.flags,
                    last_update: date.iter().any(|value| *value != 0).then_some(date),
                    update_status: nonempty(&data.update_status),
                    custom_data: table::custom_out(&data.data_map)?,
                    custom_targets: data
                        .custom_data
                        .iter()
                        .map(|value| IfccadTableDataLinkTarget {
                            object_id: value.target.value(),
                            value: value.value.clone(),
                        })
                        .collect(),
                };
                writer.table_data_links.insert(handle, id);
                writer
                    .drawing
                    .table_data_links
                    .push(IfccadTableDataLink { id, settings });
            }
            ClassObjectData::DataTable(data) => {
                let id = writer.drawing.id_counters.allocate_data_table_id()?;
                let row_count = usize::try_from(data.row_count)
                    .map_err(|_| annotation::invalid("negative DATATABLE row count"))?;
                let columns = data
                    .columns
                    .iter()
                    .map(column_out)
                    .collect::<Result<_, _>>()?;
                let owner = owner_out(writer.document, object);
                writer.drawing.data_tables.push(IfccadDataTable {
                    id,
                    settings: IfccadDataTableSettings {
                        name: data.name.clone(),
                        flags: data.flags,
                        row_count,
                        columns,
                        owner,
                    },
                });
            }
            _ => unreachable!("filtered class objects"),
        }
    }
    Ok(())
}

pub(super) fn bind_export(writer: &mut Writer<'_>) {
    let ids = &writer.entity_ids;
    writer.drawing.data_tables.retain_mut(|table| {
        let result = (|| -> Result<(), Error> {
            if let Some(id) = table
                .settings
                .owner
                .as_mut()
                .and_then(|owner| owner.entity_id.as_mut())
            {
                *id = ids
                    .get(&Handle::from(*id))
                    .copied()
                    .ok_or_else(|| annotation::invalid("unresolved DATATABLE owner"))?;
            }
            for value in table
                .settings
                .columns
                .iter_mut()
                .flat_map(|column| &mut column.values)
            {
                table::bind_value(value, ids)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            writer
                .losses
                .note(format!("DATATABLE {}: {error}", table.settings.name));
            return false;
        }
        true
    });
    for link in &mut writer.drawing.table_data_links {
        link.settings.custom_targets.retain_mut(|target| {
            if let Some(id) = ids.get(&Handle::from(target.object_id)) {
                target.object_id = *id;
                true
            } else {
                writer.losses.note(format!(
                    "DATALINK {}: missing custom target",
                    link.settings.name
                ));
                false
            }
        });
        link.settings.custom_data.retain_mut(|value| {
            if let Err(error) = table::bind_value(&mut value.value, ids) {
                writer.losses.note(format!(
                    "DATALINK {} data {}: {error}",
                    link.settings.name, value.name
                ));
                return false;
            }
            true
        });
    }
}

pub(super) fn allocate(drawing: &IfccadDocument, document: &mut Document, names: &mut Names) {
    names.table_data_links.extend(
        drawing
            .table_data_links
            .iter()
            .map(|link| (link.id, document.allocate_handle())),
    );
}

pub(super) fn import(
    drawing: &IfccadDocument,
    document: &mut Document,
    names: &Names,
    handles: &HashMap<u64, Handle>,
    losses: &mut Losses,
) -> Result<(), Error> {
    if !drawing.table_data_links.is_empty() {
        let owner = document.ensure_subdictionary("ACAD_DATALINK");
        for link in &drawing.table_data_links {
            let settings = &link.settings;
            let handle = names.table_data_links[&link.id];
            let [year, month, day, hour, minute, second, millisecond] =
                settings.last_update.unwrap_or_default();
            let mut data_map = table::custom_in(&settings.custom_data)?;
            data_map.retain_mut(|value| {
                if let Some(id) = value.value.handle_value {
                    value.value.handle_value = handles.get(&id.value()).copied();
                    if value.value.handle_value.is_none() {
                        losses.note(format!(
                            "DATALINK {} data {}: missing CAD target",
                            settings.name, value.name
                        ));
                        return false;
                    }
                }
                true
            });
            let custom_data = settings
                .custom_targets
                .iter()
                .filter_map(|value| {
                    let Some(target) = handles.get(&value.object_id).copied() else {
                        losses.note(format!(
                            "DATALINK {}: missing CAD custom target",
                            settings.name
                        ));
                        return None;
                    };
                    Some(DataLinkCustomData {
                        target,
                        value: value.value.clone(),
                    })
                })
                .collect();
            let data = DataLink {
                data_adapter: settings.adapter.clone(),
                description: settings.description.clone().unwrap_or_default(),
                tooltip: settings.tooltip.clone().unwrap_or_default(),
                connection_string: settings.source_selection.as_ref().map_or_else(
                    || settings.source.clone(),
                    |selection| format!("{}!{selection}", settings.source),
                ),
                option: settings.options,
                update_option: settings.update_options,
                flags: settings.flags,
                year,
                month,
                day,
                hour,
                minute,
                second,
                millisecond,
                path_option: settings.path_option,
                status_flags: settings.status_flags,
                update_status: settings.update_status.clone().unwrap_or_default(),
                custom_data,
                data_map,
                hard_owner: owner,
            };
            install(
                document,
                handle,
                owner,
                &settings.name,
                false,
                ClassObjectData::DataLink(data),
            );
        }
    }
    for source in &drawing.data_tables {
        let settings = &source.settings;
        let columns = settings
            .columns
            .iter()
            .map(|column| column_in(column, handles))
            .collect::<Result<Vec<_>, _>>();
        let columns = match columns {
            Ok(columns) => columns,
            Err(error) => {
                losses.note(format!("DATATABLE {}: {error}", settings.name));
                continue;
            }
        };
        let row_count = i32::try_from(settings.row_count)
            .map_err(|_| annotation::invalid("DATATABLE row count exceeds CAD limit"))?;
        let (owner, key, hard_owned) = if let Some(owner) = &settings.owner {
            let handle = owner_in(document, owner, handles)?;
            (handle, owner.key.clone(), owner.hard_owned)
        } else {
            (
                document.ensure_named_objects_dictionary(),
                format!("DATATABLE{}", source.id.0),
                false,
            )
        };
        let handle = document.allocate_handle();
        install(
            document,
            handle,
            owner,
            &key,
            hard_owned,
            ClassObjectData::DataTable(DataTable {
                flags: settings.flags,
                name: settings.name.clone(),
                row_count,
                columns,
            }),
        );
    }
    Ok(())
}

fn install(
    document: &mut Document,
    handle: Handle,
    owner: Handle,
    key: &str,
    hard_owned: bool,
    data: ClassObjectData,
) {
    if let Some(ObjectType::Dictionary(dictionary)) = document.objects.get_mut(&owner) {
        dictionary.insert(key, handle);
        if hard_owned
            && !dictionary
                .hard_owner_entries
                .iter()
                .any(|entry| entry == key)
        {
            dictionary.hard_owner_entries.push(key.into());
        }
    }
    let dxf_name = data.dxf_name();
    if !document.classes.contains(dxf_name) {
        let cpp = if dxf_name == "DATALINK" {
            "AcDbDataLink"
        } else {
            "AcDbDataTable"
        };
        document
            .classes
            .add_or_update(acadrust::classes::DxfClass::new(dxf_name, cpp));
    }
    document.objects.insert(
        handle,
        ObjectType::ClassObject(ClassObject {
            handle,
            owner,
            reactors: vec![owner],
            data,
            ..Default::default()
        }),
    );
}

fn dictionary_entry(document: &Document, owner: Handle, child: Handle) -> Option<(&str, bool)> {
    let ObjectType::Dictionary(dict) = document.objects.get(&owner)? else {
        return None;
    };
    let (key, _) = dict.entries.iter().find(|(_, target)| *target == child)?;
    Some((
        key,
        dict.hard_owner || dict.hard_owner_entries.contains(key),
    ))
}

fn owner_out(document: &Document, object: &ClassObject) -> Option<IfccadDataTableOwner> {
    let (key, hard_owned) = dictionary_entry(document, object.owner, object.handle)?;
    let mut current = object.owner;
    let mut path = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let entity_id = loop {
        if current == document.header.named_objects_dict_handle {
            break None;
        }
        if !seen.insert(current) {
            return None;
        }
        let Some(ObjectType::Dictionary(dict)) = document.objects.get(&current) else {
            return None;
        };
        if document.get_entity(dict.owner).is_some()
            && document.extension_dictionary_handle(dict.owner) == Some(current)
        {
            break Some(dict.owner.value());
        }
        let (name, _) = dictionary_entry(document, dict.owner, current)?;
        path.push(name.to_owned());
        current = dict.owner;
    };
    path.reverse();
    Some(IfccadDataTableOwner {
        entity_id,
        dictionary_path: path,
        key: key.into(),
        hard_owned,
    })
}

fn owner_in(
    document: &mut Document,
    owner: &IfccadDataTableOwner,
    handles: &HashMap<u64, Handle>,
) -> Result<Handle, Error> {
    let mut parent = if let Some(id) = owner.entity_id {
        let handle = handles
            .get(&id)
            .copied()
            .ok_or_else(|| annotation::invalid("missing DATATABLE owner"))?;
        document.ensure_extension_dictionary(handle)
    } else {
        document.ensure_named_objects_dictionary()
    };
    for key in &owner.dictionary_path {
        let existing = match document.objects.get(&parent) {
            Some(ObjectType::Dictionary(dict)) => dict.get(key),
            _ => None,
        };
        if let Some(handle) = existing {
            if !matches!(
                document.objects.get(&handle),
                Some(ObjectType::Dictionary(_))
            ) {
                return Err(annotation::invalid(
                    "DATATABLE owner path collides with existing object",
                ));
            }
            parent = handle;
            continue;
        }
        let handle = document.allocate_handle();
        let mut child = Dictionary::new();
        child.handle = handle;
        child.owner = parent;
        if let Some(ObjectType::Dictionary(dict)) = document.objects.get_mut(&parent) {
            dict.insert(key, handle);
        }
        document
            .objects
            .insert(handle, ObjectType::Dictionary(child));
        parent = handle;
    }
    Ok(parent)
}

fn column_out(source: &DataTableColumn) -> Result<IfccadDataTableColumn, Error> {
    use DataTableCellType as Cad;
    let kind = source
        .cell_type()
        .ok_or_else(|| annotation::invalid("unknown DATATABLE column type"))?;
    let value_type = match kind {
        Cad::Integer => ColumnType::Integer,
        Cad::Double => ColumnType::Real,
        Cad::Text => ColumnType::Text,
        Cad::Point => ColumnType::Point,
        Cad::ObjectId => ColumnType::Object,
        Cad::HardOwnerId => ColumnType::HardOwner,
        Cad::SoftOwnerId => ColumnType::SoftOwner,
        Cad::HardPointerId => ColumnType::HardPointer,
        Cad::SoftPointerId => ColumnType::SoftPointer,
        Cad::Bool => ColumnType::Boolean,
        Cad::Vector => ColumnType::Vector,
    };
    let values = source
        .rows
        .iter()
        .map(|value| match kind {
            Cad::Integer => Value::Integer(i64::from(value.integer)),
            Cad::Double => Value::Real(value.real),
            Cad::Text => Value::Text(value.text.clone()),
            Cad::Point | Cad::Vector => Value::Point3D(array(value.point)),
            Cad::Bool => Value::Boolean(value.integer != 0),
            _ => Value::Object(value.handle.value()),
        })
        .collect();
    Ok(IfccadDataTableColumn {
        name: source.name.clone(),
        value_type,
        values,
    })
}

fn column_in(
    source: &IfccadDataTableColumn,
    handles: &HashMap<u64, Handle>,
) -> Result<DataTableColumn, Error> {
    use DataTableCellType as Cad;
    let kind = match source.value_type {
        ColumnType::Integer => Cad::Integer,
        ColumnType::Real => Cad::Double,
        ColumnType::Text => Cad::Text,
        ColumnType::Point => Cad::Point,
        ColumnType::Object => Cad::ObjectId,
        ColumnType::HardOwner => Cad::HardOwnerId,
        ColumnType::SoftOwner => Cad::SoftOwnerId,
        ColumnType::HardPointer => Cad::HardPointerId,
        ColumnType::SoftPointer => Cad::SoftPointerId,
        ColumnType::Boolean => Cad::Bool,
        ColumnType::Vector => Cad::Vector,
    };
    let rows = source
        .values
        .iter()
        .map(|value| {
            let mut row = DataTableValue::default();
            match value {
                Value::Integer(value) => {
                    row.integer = i32::try_from(*value)
                        .map_err(|_| annotation::invalid("DATATABLE integer exceeds CAD range"))?
                }
                Value::Real(value) => row.real = *value,
                Value::Text(value) => row.text = value.clone(),
                Value::Point3D(value) => row.point = vector(*value),
                Value::Boolean(value) => row.integer = i32::from(*value),
                Value::Object(id) => {
                    row.handle = handles
                        .get(id)
                        .copied()
                        .ok_or_else(|| annotation::invalid("missing CAD DATATABLE cell target"))?
                }
                _ => return Err(annotation::invalid("invalid DATATABLE cell type")),
            }
            Ok(row)
        })
        .collect::<Result<_, Error>>()?;
    Ok(DataTableColumn {
        value_type: kind as i32,
        name: source.name.clone(),
        rows,
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}
