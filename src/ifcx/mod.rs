//! IFCX through the ifccad library. ifccad reads and writes the files; the
//! mapping between its drawing model and a [`Document`] lives here, so the two
//! libraries never depend on each other.
//!
//! The mapping covers the drawing tables, block
//! definitions, paper layouts with their viewports and plot settings, the
//! workspace state, and every IFCX entity kind. Stored dimension graphics use
//! native CAD picture blocks within a supported planar profile. Pictures
//! outside that profile and semantics IFCX cannot hold are reported as losses.

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::TAU;

use acadrust::entities::{
    Arc, Block, BlockEnd, Circle, Ellipse, EntityCommon, Insert, Line, LwPolyline, LwVertex, Point,
    Polyline3D, Solid, Text, TextHorizontalAlignment, TextVerticalAlignment,
};
use acadrust::tables::BlockRecord;
use acadrust::types::{Color, Handle, LineWeight, Matrix3, Transparency, Vector2, Vector3};
use acadrust::EntityType;
use ifccad::text::{
    validate_text_runs, FontRequest, TextHorizontalAlignment as Horizontal, TextLayout, TextRun,
    TextVerticalAlignment as Vertical,
};
use ifccad::{
    encode_ifccad_document, load_ifccad_bytes, IfccadBlockDefinition, IfccadBlockTransform,
    IfccadColor, IfccadDocument, IfccadEntity, IfccadEntityAppearance, IfccadEntityKind,
    IfccadFilledQuadrilateral, IfccadHeader, IfccadLinePatternGeneration, IfccadLinePatternId,
    IfccadMode, IfccadNativeEntity, IfccadPaperLayout, IfccadPlacement, IfccadPlanarPolylineWidths,
    IfccadSymbolSourceId, IfccadText, IfccadTextStyleId, IfccadUcsId, IfccadUcsSelection,
};

use crate::{Document, Error, Format, Loss, Placement};

mod dimension;
mod dimension_graphics;
mod dimension_kinds;
mod hatch;
mod layout;
mod tables;
mod text;

// Custom document properties that carry an IFCX file's identity through
// editing, so a drawing read from IFCX is written back as the same drawing.
const HEADER_ID: &str = "IFCX id";
const DATA_VERSION: &str = "IFCX data version";
const DRAWING_ID: &str = "IFCX drawing id";

/// An empty drawing: a model layout and the continuous line pattern (id 0)
/// that layers and entities refer to.
const TEMPLATE: &str = r#"{
  "header": {
    "id": "opencad-drawing",
    "ifcxVersion": "ifcx_alpha",
    "dataVersion": "0.1.0",
    "author": "OpenCAD",
    "timestamp": "1970-01-01T00:00:00Z"
  },
  "imports": [{ "uri": "urn:example:ifccad:0.3.0" }],
  "schemas": {},
  "data": [
    {
      "path": "/cad/d1",
      "children": {
        "linePattern0": "/cad/d1/linePattern/0",
        "model": "/cad/d1/layout/1"
      },
      "attributes": {
        "ifccad::drawing": {
          "lengthUnit": "mm",
          "nextBlockId": 1,
          "nextEntityId": 1,
          "nextLayerId": 1,
          "nextLayoutId": 2,
          "nextLinePatternId": 1,
          "nextSymbolSourceId": 1,
          "profileVersion": "0.3.0",
          "nextUcsId": 1,
          "nextModelWindowId": 1
        }
      }
    },
    {
      "path": "/cad/d1/linePattern/0",
      "attributes": { "ifccad::linePattern": { "name": "Continuous", "pattern": [] } }
    },
    {
      "path": "/cad/d1/layout/1",
      "attributes": { "ifccad::layout": { "kind": "Model", "tabIndex": 0 } }
    }
  ]
}"#;

/// INSUNITS codes 0-24 in order, as IFCX length-unit tokens.
const UNITS: [&str; 25] = [
    "unitless",
    "in",
    "ft",
    "mi",
    "mm",
    "cm",
    "m",
    "km",
    "microin",
    "mil",
    "yd",
    "angstrom",
    "nm",
    "um",
    "dm",
    "dam",
    "hm",
    "Gm",
    "au",
    "ly",
    "pc",
    "usSurveyFoot",
    "usSurveyInch",
    "usSurveyYard",
    "usSurveyMile",
];

/// IFCX file IO and conversion between [`IfccadDocument`] and [`Document`].
pub struct Ifcx;

/// The result of importing a logical IFCCAD drawing into the shared CAD model.
#[derive(Debug)]
pub struct IfccadImportOutcome {
    /// The converted CAD drawing.
    pub document: Document,
    /// Conversion diagnostics, including content loss and retained-picture numerical observations.
    pub losses: Vec<Loss>,
    /// Source IFCCAD entity IDs mapped to handles in `document`.
    ///
    /// Includes converted entities in Model, Paper and block definitions.
    /// Omitted entities have no entry. Generated CAD helpers (such as dimension
    /// picture primitives) have no separate IFCCAD entity ID and are not mapped.
    /// A block member is mapped once, not once per insert occurrence.
    pub entity_handles: BTreeMap<u64, Handle>,
}

/// The result of exporting the shared CAD model to a logical IFCCAD drawing.
#[derive(Debug)]
pub struct IfccadExportOutcome {
    /// The converted drawing, validated by ifccad.
    pub drawing: IfccadDocument,
    /// Conversion diagnostics, including content loss and retained-picture numerical observations.
    pub losses: Vec<Loss>,
    /// Source CAD handles mapped to entity IDs in `drawing`.
    ///
    /// Includes converted entities in Model, Paper and block definitions.
    /// Omitted entities and CAD helpers encoded as dimension-picture parts have
    /// no entry. A block member is mapped once, not per insert occurrence.
    pub entity_ids: BTreeMap<Handle, u64>,
}

impl Format for Ifcx {
    fn name(&self) -> &str {
        "IFCX"
    }

    fn extensions(&self) -> &[&str] {
        &["ifcx"]
    }

    fn read(&self, bytes: &[u8]) -> Result<(Document, Vec<Loss>), Error> {
        let drawing = load_ifccad_bytes(bytes, Default::default())?.into_document();
        let outcome = self.import_ifccad(&drawing)?;
        Ok((outcome.document, outcome.losses))
    }

    fn write(&self, document: &Document) -> Result<(Vec<u8>, Vec<Loss>), Error> {
        let outcome = self.export_ifccad(document)?;
        let encoded = encode_ifccad_document(&outcome.drawing)?;
        Ok((encoded.bytes().to_vec(), outcome.losses))
    }
}

impl Ifcx {
    /// Convert a logical IFCCAD drawing into a [`Document`] without a JSON bridge.
    ///
    /// Validates `drawing` before mapping it. Uses the same mapping, rounding and
    /// loss reporting as [`Format::read`]; this is not a lossless conversion.
    /// Only the supplied logical drawing is converted, not any enclosing IFCX
    /// source graph. Entity associations refer to this conversion's output.
    ///
    /// # Errors
    ///
    /// Returns an error if IFCCAD validation or CAD document construction fails.
    pub fn import_ifccad(&self, drawing: &IfccadDocument) -> Result<IfccadImportOutcome, Error> {
        ifccad::validate_ifccad_document(drawing)?;
        let mut losses = Losses::default();
        let mut document = Document::new();
        document.header.insertion_units = unit_code(&drawing.length_unit);
        let info = &mut document.summary_info;
        set(
            &mut info.custom_properties,
            HEADER_ID,
            drawing.header.id.clone(),
        );
        set(
            &mut info.custom_properties,
            DATA_VERSION,
            drawing.header.data_version.clone(),
        );
        set(
            &mut info.custom_properties,
            DRAWING_ID,
            drawing.drawing_id.to_string(),
        );
        if info.author.is_empty() {
            info.author = drawing.header.author.clone();
        }

        let mut names = Names::default();
        tables::read(drawing, &mut document, &mut names, &mut losses)?;

        // Every definition exists before any member is converted, so inserts
        // of blocks defined later in the file resolve.
        let mut members = Vec::new();
        for block in &drawing.blocks {
            let mut record = BlockRecord::new(block.name.clone());
            if record.is_model_space()
                || record.is_paper_space()
                || document.block_records.get(&block.name).is_some()
            {
                losses.note(format!("block {}", block.name));
                continue;
            }
            record.description = block.description.clone();
            record.flags.anonymous = block.anonymous;
            record.explodable = block.explodable;
            record.scale_uniformly = block.uniform_scaling;
            record.base_point = vector(block.base_point);
            record.units = unit_code(&block.insertion_unit);
            record.handle = document.allocate_handle();
            record.block_entity_handle = document.allocate_handle();
            record.block_end_handle = document.allocate_handle();
            let mut start = Block::new(block.name.clone(), record.base_point);
            start.description = block.description.clone();
            start.common.handle = record.block_entity_handle;
            start.common.owner_handle = record.handle;
            let mut end = BlockEnd::new();
            end.common.handle = record.block_end_handle;
            end.common.owner_handle = record.handle;
            names.blocks.insert(block.id, block.name.clone());
            members.push((record.handle, &block.entities));
            document.block_records.add(record)?;
            document.add_entity(EntityType::Block(start))?;
            document.add_entity(EntityType::BlockEnd(end))?;
        }
        dimension::read_styles(drawing, &mut document, &mut names, &mut losses)?;
        let papers = layout::read(drawing, &mut document, &mut losses)?;
        let mut sheets: Vec<&IfccadPaperLayout> = drawing.paper_layouts.iter().collect();
        sheets.sort_by_key(|paper| paper.tab_index);
        let sheets = papers
            .iter()
            .zip(sheets)
            .map(|(owner, paper)| (*owner, &paper.entities));
        let model = (Handle::NULL, &drawing.model.entities);
        let mut handles = HashMap::new();
        for (owner, entities) in members.into_iter().chain(sheets).chain([model]) {
            for entity in entities {
                if let Some(handle) =
                    add_in(&mut document, entity, owner, drawing, &names, &mut losses)?
                {
                    handles.insert(entity.id(), handle);
                }
            }
        }
        // Hatch boundaries refer to entities that may come after them.
        let scopes = drawing.blocks.iter().map(|block| &block.entities);
        let papers_entities = drawing.paper_layouts.iter().map(|paper| &paper.entities);
        for entity in scopes
            .chain(papers_entities)
            .chain([&drawing.model.entities])
            .flatten()
        {
            let Some(IfccadNativeEntity {
                id,
                kind: IfccadEntityKind::Hatch(hatch),
                ..
            }) = entity.as_native()
            else {
                continue;
            };
            let Some(EntityType::Hatch(converted)) = handles
                .get(id)
                .and_then(|handle| document.get_entity_mut(*handle))
            else {
                continue;
            };
            for (path, hatch_loop) in converted.paths.iter_mut().zip(&hatch.loops) {
                path.boundary_handles = hatch_loop
                    .source_entity_ids
                    .iter()
                    .filter_map(|id| handles.get(id).copied())
                    .collect();
            }
        }
        layout::finish(drawing, &mut document, &names, &papers, &handles)?;
        dimension::associations_in(drawing, &mut document, &handles, &mut losses);
        Ok(IfccadImportOutcome {
            document,
            losses: losses.into_vec(),
            entity_handles: handles.into_iter().collect(),
        })
    }

    /// Convert a [`Document`] into a validated logical IFCCAD drawing.
    ///
    /// Uses the same mapping, rounding and loss reporting as [`Format::write`],
    /// without encoding and reparsing the result. Entity associations refer to
    /// this conversion's output; IDs are not promised stable across conversions.
    /// Encode the result with [`ifccad::encode_ifccad_document`] when IFCX JSON
    /// bytes are needed.
    ///
    /// # Examples
    ///
    /// ```
    /// use opencaddocument::{Document, Ifcx};
    ///
    /// let exported = Ifcx.export_ifccad(&Document::new())?;
    /// let imported = Ifcx.import_ifccad(&exported.drawing)?;
    /// assert!(exported.losses.is_empty());
    /// assert!(imported.losses.is_empty());
    /// # Ok::<(), opencaddocument::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if conversion or validation of the resulting drawing fails.
    pub fn export_ifccad(&self, document: &Document) -> Result<IfccadExportOutcome, Error> {
        let mut drawing =
            load_ifccad_bytes(TEMPLATE.as_bytes(), Default::default())?.into_document();
        let info = &document.summary_info;
        let get = |name: &str| {
            info.custom_properties
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        drawing.header = IfccadHeader {
            id: get(HEADER_ID).unwrap_or_else(|| "opencad-drawing".to_string()),
            data_version: get(DATA_VERSION).unwrap_or_else(|| "0.1.0".to_string()),
            author: match info.author.as_str() {
                "" => "OpenCAD".to_string(),
                author => author.to_string(),
            },
            timestamp: rfc3339(now_seconds()),
        };
        drawing.drawing_id = get(DRAWING_ID).and_then(|id| id.parse().ok()).unwrap_or(1);
        drawing.length_unit = unit_name(document.header.insertion_units);
        let mut writer = Writer {
            document,
            drawing,
            losses: Losses::default(),
            layers: HashMap::new(),
            line_patterns: HashMap::new(),
            styles: HashMap::new(),
            style_handles: HashMap::new(),
            symbols: HashMap::new(),
            blocks: HashMap::new(),
            entity_ids: HashMap::new(),
            hatch_sources: HashMap::new(),
            viewport_clips: HashMap::new(),
            layout_ids: HashMap::new(),
            ucs_ids: HashMap::new(),
            dim_styles: HashMap::new(),
            dimension_sources: HashMap::new(),
        };

        tables::write(&mut writer)?;
        dimension::write_styles(&mut writer)?;

        for entity in document.model_space_entities() {
            if let Some(entity) = writer.entity(entity)? {
                writer.drawing.model.entities.push(entity);
            }
        }
        // Named blocks are kept even when nothing inserts them; anonymous ones
        // only exist for what refers to them.
        for record in document.block_records.iter() {
            if !record.is_anonymous() {
                writer.block(&record.name)?;
            }
        }
        layout::write(&mut writer)?;
        layout::link_clips(&mut writer);
        let ids = &writer.entity_ids;
        let sources = &writer.hatch_sources;
        let drawing = &mut writer.drawing;
        let scopes = drawing.blocks.iter_mut().map(|block| &mut block.entities);
        let papers = drawing
            .paper_layouts
            .iter_mut()
            .map(|paper| &mut paper.entities);
        for entities in scopes.chain(papers).chain([&mut drawing.model.entities]) {
            link_hatches(entities, ids, sources);
            dimension::link_associations(
                entities,
                ids,
                &writer.dimension_sources,
                &mut writer.losses,
            );
        }

        ifccad::validate_ifccad_document(&writer.drawing)?;
        Ok(IfccadExportOutcome {
            drawing: writer.drawing,
            losses: writer.losses.into_vec(),
            entity_ids: writer.entity_ids.into_iter().collect(),
        })
    }
}

/// Point each hatch loop at its boundary source entities, the ones that are
/// in the same scope and can bound a hatch.
fn link_hatches(
    entities: &mut [IfccadEntity],
    ids: &HashMap<Handle, u64>,
    sources: &HashMap<u64, Vec<Vec<Handle>>>,
) {
    let eligible: std::collections::HashSet<u64> = entities
        .iter()
        .filter_map(IfccadEntity::as_native)
        .filter(|entity| entity.kind.is_hatch_boundary_source())
        .map(|entity| entity.id)
        .collect();
    for entity in entities {
        let IfccadEntity::Native(IfccadNativeEntity {
            id,
            kind: IfccadEntityKind::Hatch(hatch),
            ..
        }) = entity
        else {
            continue;
        };
        let Some(handles) = sources.get(id) else {
            continue;
        };
        for (hatch_loop, handles) in hatch.loops.iter_mut().zip(handles) {
            hatch_loop.source_entity_ids = handles
                .iter()
                .filter_map(|handle| ids.get(handle).copied())
                .filter(|id| eligible.contains(id))
                .collect();
        }
    }
}

/// Names of what IFCX refers to by id.
#[derive(Default)]
struct Names {
    layers: HashMap<u64, String>,
    layer_handles: HashMap<u64, Handle>,
    /// Dimension styles: name, settings and per-kind overrides.
    dim_styles: HashMap<
        ifccad::IfccadDimensionStyleId,
        (
            String,
            ifccad::IfccadDimensionSettings,
            BTreeMap<ifccad::dimension::DimensionKind, ifccad::IfccadDimensionOverrides>,
        ),
    >,
    line_patterns: HashMap<IfccadLinePatternId, String>,
    styles: HashMap<IfccadTextStyleId, String>,
    style_handles: HashMap<IfccadTextStyleId, Handle>,
    symbols: HashMap<IfccadSymbolSourceId, Handle>,
    blocks: HashMap<u64, String>,
}

fn add_in(
    document: &mut Document,
    entity: &IfccadEntity,
    owner: Handle,
    drawing: &IfccadDocument,
    names: &Names,
    losses: &mut Losses,
) -> Result<Option<Handle>, Error> {
    let IfccadEntity::Native(entity) = entity else {
        losses.note("an entity ifccad keeps only as opaque data");
        return Ok(None);
    };
    let converted = match &entity.kind {
        IfccadEntityKind::Dimension(source) => {
            dimension::dimension_in(source, document, names, losses)
        }
        kind => entity_in(kind, names, losses),
    };
    let Some(mut converted) = converted else {
        losses.note(kind_name(&entity.kind));
        return Ok(None);
    };
    let common = converted.common_mut();
    common.owner_handle = owner;
    common.layer = names
        .layers
        .get(&entity.layer_id)
        .cloned()
        .unwrap_or_else(|| "0".to_string());
    appearance_in(&entity.appearance, names, common, losses);
    common.invisible = !entity.visible;
    common.linetype_scale = entity.line_pattern_scale;
    if let (IfccadEntityKind::Dimension(source), EntityType::Dimension(target)) =
        (&entity.kind, &mut converted)
    {
        if let Some(name) = dimension_graphics::import(
            source,
            entity.id,
            entity.layer_id,
            owner,
            drawing,
            document,
            names,
            losses,
        )? {
            target.base_mut().block_name = name;
        }
    }
    Ok(Some(document.add_entity(converted)?))
}

struct Writer<'a> {
    document: &'a Document,
    drawing: IfccadDocument,
    losses: Losses,
    layers: HashMap<String, u64>,
    /// Line pattern ids by lower-case linetype name.
    line_patterns: HashMap<String, IfccadLinePatternId>,
    styles: HashMap<String, IfccadTextStyleId>,
    style_handles: HashMap<Handle, IfccadTextStyleId>,
    /// Symbol sources by the handle of their shape-file style.
    symbols: HashMap<Handle, IfccadSymbolSourceId>,
    /// Block ids by lower-case name; `None` for blocks IFCX cannot hold.
    blocks: HashMap<String, Option<u64>>,
    entity_ids: HashMap<Handle, u64>,
    /// Boundary source handles of each hatch loop, by hatch id.
    hatch_sources: HashMap<u64, Vec<Vec<Handle>>>,
    /// Clip boundary of each clipped viewport, by viewport handle.
    viewport_clips: HashMap<Handle, Handle>,
    /// Paper layout ids by LAYOUT object handle.
    layout_ids: HashMap<Handle, u64>,
    ucs_ids: HashMap<String, IfccadUcsId>,
    /// Dimension style ids and settings by lower-case name.
    dim_styles: HashMap<
        String,
        (
            ifccad::IfccadDimensionStyleId,
            ifccad::IfccadDimensionSettings,
        ),
    >,
    /// Point associations of each associated dimension, by its handle.
    dimension_sources: HashMap<
        Handle,
        Vec<(
            ifccad::dimension::DimensionRole,
            Handle,
            ifccad::dimension::DimensionPointSelector,
        )>,
    >,
}

impl Writer<'_> {
    fn entity(&mut self, entity: &EntityType) -> Result<Option<IfccadEntity>, Error> {
        let Some(mut kind) = self.kind(entity)? else {
            self.losses.note(entity.as_entity().entity_type());
            return Ok(None);
        };
        if let (EntityType::Dimension(source), IfccadEntityKind::Dimension(target)) =
            (entity, &mut kind)
        {
            target.graphics = dimension_graphics::capture(self, source, &target.placement)?;
        }
        let common = entity.common();
        let layer_id = match self.layers.get(&common.layer.to_ascii_lowercase()) {
            Some(id) => *id,
            None => self.layers.get("0").copied().unwrap_or(0),
        };
        let id = self.drawing.id_counters.allocate_entity_id()?;
        self.entity_ids.insert(common.handle, id);
        if let EntityType::Hatch(hatch) = entity {
            let sources: Vec<_> = hatch
                .paths
                .iter()
                .map(|p| p.boundary_handles.clone())
                .collect();
            if sources.iter().any(|handles| !handles.is_empty()) {
                self.hatch_sources.insert(id, sources);
            }
        }
        Ok(Some(IfccadEntity::Native(IfccadNativeEntity {
            id,
            visible: !common.invisible,
            layer_id,
            appearance: appearance_out(common, &self.line_patterns, &mut self.losses),
            line_pattern_scale: common.linetype_scale,
            kind,
        })))
    }

    /// The id of block `name`, converting its definition the first time.
    fn block(&mut self, name: &str) -> Result<Option<u64>, Error> {
        let key = name.to_ascii_lowercase();
        if let Some(id) = self.blocks.get(&key) {
            return Ok(*id);
        }
        let document = self.document;
        let Some(record) = document.block_records.get(name) else {
            return Ok(None);
        };
        if record.is_layout()
            || record.is_model_space()
            || record.is_paper_space()
            || record.flags.is_xref
            || record.flags.is_xref_overlay
        {
            self.blocks.insert(key, None);
            return Ok(None);
        }
        // The id is known before the members convert, so a block that
        // (wrongly) inserts itself does not recurse forever.
        let id = self.drawing.id_counters.allocate_block_id()?;
        self.blocks.insert(key, Some(id));
        let mut entities = Vec::new();
        for entity in document.entities_in_block(&record.name) {
            if let Some(entity) = self.entity(entity)? {
                entities.push(entity);
            }
        }
        self.drawing.blocks.push(IfccadBlockDefinition {
            description: record.description.clone(),
            anonymous: record.is_anonymous(),
            explodable: record.explodable,
            uniform_scaling: record.scale_uniformly,
            bounds_quality: None,
            id,
            name: record.name.clone(),
            base_point: array(record.base_point),
            insertion_unit: unit_name(record.units),
            bounds: None,
            entities,
        });
        Ok(Some(id))
    }

    fn kind(&mut self, entity: &EntityType) -> Result<Option<IfccadEntityKind>, Error> {
        Ok(Some(match entity {
            EntityType::Line(line) => IfccadEntityKind::LineSegment {
                start: array(line.start),
                end: array(line.end),
                thickness: line.thickness,
                extrusion_direction: array(unit_direction(line.normal)?),
            },
            EntityType::Point(point) => IfccadEntityKind::Point {
                placement: point_placement(
                    point.location,
                    unit_direction(point.normal)?,
                    point.x_axis_angle,
                ),
                thickness: point.thickness,
            },
            EntityType::Circle(circle) => IfccadEntityKind::Circle {
                radius: circle.radius,
                placement: placement_out(circle.center, unit_direction(circle.normal)?, 0.0),
                thickness: circle.thickness,
            },
            EntityType::Arc(arc) => IfccadEntityKind::Arc {
                radius: arc.radius,
                start_parameter: arc.start_angle,
                sweep_parameter: match (arc.end_angle - arc.start_angle).rem_euclid(TAU) {
                    0.0 => TAU,
                    sweep => sweep,
                },
                placement: placement_out(arc.center, unit_direction(arc.normal)?, 0.0),
                thickness: arc.thickness,
            },
            EntityType::Ellipse(ellipse) => {
                let axis = to_ocs(ellipse.major_axis, ellipse.normal);
                let major = axis.x.hypot(axis.y);
                let minor = major * ellipse.minor_axis_ratio;
                if !(major > 0.0 && minor > 0.0 && minor <= major) {
                    return Ok(None);
                }
                let placement = placement_out(
                    to_ocs(ellipse.center, ellipse.normal),
                    ellipse.normal,
                    axis.y.atan2(axis.x),
                );
                match (ellipse.end_parameter - ellipse.start_parameter).rem_euclid(TAU) {
                    0.0 => IfccadEntityKind::Ellipse {
                        semi_major_radius: major,
                        semi_minor_radius: minor,
                        placement,
                    },
                    sweep => IfccadEntityKind::EllipseArc {
                        semi_major_radius: major,
                        semi_minor_radius: minor,
                        start_parameter: ellipse.start_parameter,
                        sweep_parameter: sweep,
                        placement,
                    },
                }
            }
            EntityType::LwPolyline(polyline) => {
                let vertex_widths = polyline
                    .vertices
                    .iter()
                    .any(|vertex| vertex.start_width != 0.0 || vertex.end_width != 0.0);
                IfccadEntityKind::PlanarPolyline {
                    widths: match vertex_widths {
                        true => IfccadPlanarPolylineWidths::PerVertex {
                            widths: polyline
                                .vertices
                                .iter()
                                .map(|vertex| [vertex.start_width, vertex.end_width])
                                .collect(),
                        },
                        false => IfccadPlanarPolylineWidths::Constant {
                            width: polyline.constant_width,
                        },
                    },
                    vertices: polyline
                        .vertices
                        .iter()
                        .map(|vertex| [vertex.location.x, vertex.location.y])
                        .collect(),
                    bulges: polyline
                        .vertices
                        .iter()
                        .map(|vertex| vertex.bulge)
                        .collect(),
                    closed: polyline.is_closed,
                    placement: placement_out(
                        Vector3::new(0.0, 0.0, polyline.elevation),
                        unit_direction(polyline.normal)?,
                        0.0,
                    ),
                    line_pattern_generation: generation(polyline.plinegen),
                    thickness: polyline.thickness,
                }
            }
            EntityType::Polyline2D(polyline) => {
                if polyline.flags.bits() & !129 != 0
                    || polyline.smooth_surface as i32 != 0
                    || polyline
                        .vertices
                        .iter()
                        .any(|v| v.flags.bits() != 0 || v.curve_tangent != 0. || v.location.z != 0.)
                {
                    self.losses
                        .note("classic POLYLINE fitted/mesh or nonplanar vertex detail");
                    return Ok(None);
                }
                self.losses.retained.push(Loss {handle:Some(polyline.common.handle),message:
                    "classic POLYLINE mapped to PlanarPolyline; effective vertex widths retained; original default/override presence is unavailable or unassessed in the codec model; CAD return uses LWPOLYLINE".into()});
                if polyline.vertices.iter().any(|v| v.id != 0) {
                    self.losses.retained.push(Loss {
                        handle: Some(polyline.common.handle),
                        message: "classic POLYLINE vertex identifiers are not retained".into(),
                    });
                }
                IfccadEntityKind::PlanarPolyline {
                    widths: IfccadPlanarPolylineWidths::PerVertex {
                        widths: polyline
                            .vertices
                            .iter()
                            .map(|v| [v.start_width, v.end_width])
                            .collect(),
                    },
                    vertices: polyline
                        .vertices
                        .iter()
                        .map(|v| [v.location.x, v.location.y])
                        .collect(),
                    bulges: polyline.vertices.iter().map(|v| v.bulge).collect(),
                    closed: polyline.is_closed(),
                    placement: placement_out(
                        Vector3::new(0., 0., polyline.elevation),
                        unit_direction(polyline.normal)?,
                        0.,
                    ),
                    thickness: polyline.thickness,
                    line_pattern_generation: generation(polyline.flags.bits() & 128 != 0),
                }
            }
            EntityType::Polyline3D(polyline) => {
                if polyline.flags.spline_fit {
                    self.losses.note("the spline fit of a 3D polyline");
                }
                // Spline frame control points (flag 16) are not on the curve.
                let vertices: Vec<_> = polyline
                    .vertices
                    .iter()
                    .filter(|vertex| vertex.flags & 16 == 0)
                    .map(|vertex| array(vertex.position))
                    .collect();
                if vertices.len() < 2 {
                    return Ok(None);
                }
                IfccadEntityKind::SpatialPolyline {
                    vertices,
                    closed: polyline.flags.closed,
                    line_pattern_generation: generation(polyline.flags.linetype_continuous),
                }
            }
            EntityType::Solid(solid) => {
                let corners = [
                    solid.first_corner,
                    solid.second_corner,
                    solid.third_corner,
                    solid.fourth_corner,
                ];
                let elevation = solid.first_corner.z;
                if corners.iter().any(|corner| corner.z != elevation) {
                    return Ok(None);
                }
                let quadrilateral = IfccadFilledQuadrilateral {
                    placement: placement_out(Vector3::new(0.0, 0.0, elevation), solid.normal, 0.0),
                    corners: corners.map(|corner| [corner.x, corner.y]),
                    thickness: solid.thickness,
                };
                match solid.is_trace {
                    true => IfccadEntityKind::Trace(quadrilateral),
                    false => IfccadEntityKind::Solid(quadrilateral),
                }
            }
            EntityType::Text(text) => {
                let Some(&style_id) = self.styles.get(&text.style.to_ascii_lowercase()) else {
                    return Ok(None);
                };
                let content = text::text_runs(&text.value);
                if !(text.height > 0.0 && text.width_factor > 0.0)
                    || validate_text_runs(&content).is_err()
                {
                    return Ok(None);
                }
                let start = text.insertion_point;
                let alignment = text.alignment_point.unwrap_or(start);
                type H = TextHorizontalAlignment;
                // The point the text hangs from, its rotation, the rotation of
                // its frame, and how it is laid out.
                let (origin, rotation, frame, layout) = match text.horizontal_alignment {
                    H::Aligned | H::Fit => {
                        let (dx, dy) = (alignment.x - start.x, alignment.y - start.y);
                        let length = dx.hypot(dy);
                        if length <= 0.0 {
                            return Ok(None);
                        }
                        let layout = match text.horizontal_alignment {
                            H::Aligned => TextLayout::Aligned {
                                length,
                                width_factor: text.width_factor,
                            },
                            _ => TextLayout::Fit {
                                length,
                                height: text.height,
                            },
                        };
                        (start, 0.0, dy.atan2(dx), layout)
                    }
                    H::Middle => (
                        alignment,
                        text.rotation,
                        0.0,
                        TextLayout::WholeTextMiddle {
                            height: text.height,
                            width_factor: text.width_factor,
                        },
                    ),
                    horizontal => {
                        let vertical = match text.vertical_alignment {
                            TextVerticalAlignment::Baseline => Vertical::Baseline,
                            TextVerticalAlignment::Bottom => Vertical::Bottom,
                            TextVerticalAlignment::Middle => Vertical::Middle,
                            TextVerticalAlignment::Top => Vertical::Top,
                        };
                        let horizontal = match horizontal {
                            H::Left => Horizontal::Left,
                            H::Center => Horizontal::Center,
                            _ => Horizontal::Right,
                        };
                        let origin = match (horizontal, vertical) {
                            (Horizontal::Left, Vertical::Baseline) => start,
                            _ => alignment,
                        };
                        let layout = TextLayout::Anchored {
                            horizontal,
                            vertical,
                            height: text.height,
                            width_factor: text.width_factor,
                        };
                        (origin, text.rotation, 0.0, layout)
                    }
                };
                IfccadEntityKind::Text(IfccadText {
                    style_id,
                    placement: placement_out(origin, text.normal, frame),
                    rotation,
                    backward: text.generation_flags & 2 != 0,
                    upside_down: text.generation_flags & 4 != 0,
                    layout,
                    oblique_angle: text.oblique_angle,
                    thickness: text.thickness,
                    content,
                })
            }
            EntityType::MText(mtext) => return Ok(text::mtext_out(self, mtext)),
            EntityType::Hatch(hatch) => return Ok(hatch::hatch_out(hatch, &mut self.losses)),
            EntityType::Viewport(viewport) => return Ok(layout::viewport_out(self, viewport)),
            EntityType::Dimension(dimension) => {
                return Ok(dimension::dimension_out(self, dimension))
            }
            EntityType::Insert(insert) => {
                if insert.is_array() {
                    return Ok(None);
                }
                let Some(definition_id) = self.block(&insert.block_name)? else {
                    return Ok(None);
                };
                if insert.has_attributes() {
                    self.losses.note("block attributes");
                }
                IfccadEntityKind::BlockInstance {
                    definition_id,
                    transform: IfccadBlockTransform {
                        placement: placement_out(insert.insert_point, insert.normal, 0.0),
                        rotation: insert.rotation,
                        scale: [insert.x_scale(), insert.y_scale(), insert.z_scale()],
                    },
                }
            }
            _ => return Ok(None),
        }))
    }
}

/// Content omissions and separately staged retained-picture observations.
#[derive(Default)]
struct Losses {
    content: BTreeMap<String, usize>,
    retained: Vec<Loss>,
}

impl Losses {
    fn note(&mut self, what: impl Into<String>) {
        *self.content.entry(what.into()).or_default() += 1;
    }

    fn has_content_loss(&self) -> bool {
        !self.content.is_empty()
    }
    fn content_snapshot(&self) -> BTreeMap<String, usize> {
        self.content.clone()
    }
    fn retained_note(&mut self, note: Loss) {
        self.retained.push(note);
    }
    fn merge(&mut self, other: Self) {
        for (what, count) in other.content {
            *self.content.entry(what).or_default() += count;
        }
        self.retained.extend(other.retained);
    }
    fn locate_new_content(&mut self, before: &BTreeMap<String, usize>, prefix: &str) {
        for (key, count) in self.content_snapshot() {
            let previous = before.get(&key).copied().unwrap_or(0);
            if count > previous {
                if previous == 0 {
                    self.content.remove(&key);
                } else {
                    self.content.insert(key.clone(), previous);
                }
                *self.content.entry(format!("{prefix}: {key}")).or_default() += count - previous;
            }
        }
    }

    fn into_vec(self) -> Vec<Loss> {
        self.content
            .into_iter()
            .map(|(what, count)| Loss {
                handle: None,
                message: match count {
                    1 => format!("{what}: not converted"),
                    n => format!("{what}: not converted ({n}×)"),
                },
            })
            .chain(self.retained)
            .collect()
    }
}

fn entity_in(kind: &IfccadEntityKind, names: &Names, losses: &mut Losses) -> Option<EntityType> {
    Some(match kind {
        IfccadEntityKind::LineSegment {
            start,
            end,
            thickness,
            extrusion_direction,
        } => {
            let mut line = Line::from_points(vector(*start), vector(*end));
            line.thickness = *thickness;
            line.normal = vector(*extrusion_direction);
            EntityType::Line(line)
        }
        IfccadEntityKind::Point {
            placement,
            thickness,
        } => {
            let mut point = Point::new();
            point.location = vector(placement.origin);
            let (_, normal, rotation) = placement_in(placement);
            point.normal = normal;
            point.x_axis_angle = rotation;
            point.thickness = *thickness;
            EntityType::Point(point)
        }
        IfccadEntityKind::Circle {
            radius,
            placement,
            thickness,
        } => {
            let (center, normal, _) = placement_in(placement);
            let mut circle = Circle::new();
            circle.center = center;
            circle.normal = normal;
            circle.radius = *radius;
            circle.thickness = *thickness;
            EntityType::Circle(circle)
        }
        IfccadEntityKind::Arc {
            radius,
            start_parameter,
            sweep_parameter,
            placement,
            thickness,
        } => {
            let (center, normal, rotation) = placement_in(placement);
            // A clockwise sweep is the same arc drawn from its other end.
            let start = start_parameter + rotation + sweep_parameter.min(0.0);
            let mut arc = Arc::new();
            arc.center = center;
            arc.normal = normal;
            arc.radius = *radius;
            arc.thickness = *thickness;
            arc.start_angle = start.rem_euclid(TAU);
            arc.end_angle = (start + sweep_parameter.abs()).rem_euclid(TAU);
            EntityType::Arc(arc)
        }
        IfccadEntityKind::Ellipse {
            semi_major_radius,
            semi_minor_radius,
            placement,
        }
        | IfccadEntityKind::EllipseArc {
            semi_major_radius,
            semi_minor_radius,
            placement,
            ..
        } => {
            if semi_minor_radius > semi_major_radius {
                return None;
            }
            let (_, normal, _) = placement_in(placement);
            let mut ellipse = Ellipse::new();
            ellipse.center = vector(placement.origin);
            ellipse.major_axis = vector(placement.x_axis) * *semi_major_radius;
            ellipse.minor_axis_ratio = semi_minor_radius / semi_major_radius;
            ellipse.normal = normal;
            if let IfccadEntityKind::EllipseArc {
                start_parameter,
                sweep_parameter,
                ..
            } = kind
            {
                // A clockwise sweep is the same arc drawn from its other end.
                let start = start_parameter + sweep_parameter.min(0.0);
                ellipse.start_parameter = start;
                ellipse.end_parameter = start + sweep_parameter.abs();
            }
            EntityType::Ellipse(ellipse)
        }
        IfccadEntityKind::PlanarPolyline {
            widths,
            vertices,
            bulges,
            closed,
            placement,
            line_pattern_generation,
            thickness,
        } => {
            let (origin, normal, rotation) = placement_in(placement);
            let mut polyline = LwPolyline::new();
            polyline.normal = normal;
            polyline.thickness = *thickness;
            polyline.elevation = origin.z;
            polyline.is_closed = *closed;
            polyline.plinegen = *line_pattern_generation == IfccadLinePatternGeneration::Continuous;
            for (index, [u, v]) in vertices.iter().enumerate() {
                let mut vertex = LwVertex::new(frame_to_ocs(*u, *v, origin, rotation));
                vertex.bulge = bulges.get(index).copied().unwrap_or(0.0);
                polyline.vertices.push(vertex);
            }
            match widths {
                IfccadPlanarPolylineWidths::Constant { width } => polyline.constant_width = *width,
                IfccadPlanarPolylineWidths::PerVertex { widths } => {
                    for (vertex, [start, end]) in polyline.vertices.iter_mut().zip(widths) {
                        vertex.start_width = *start;
                        vertex.end_width = *end;
                    }
                }
                IfccadPlanarPolylineWidths::Defaulted {
                    default_start_width,
                    default_end_width,
                    overrides,
                } => {
                    losses.retained.push(Loss{handle:None,message:"PlanarPolyline width inheritance materialized: effective widths retained, default/override dependencies lost".into()});
                    for (vertex, width) in polyline.vertices.iter_mut().zip(overrides) {
                        vertex.start_width = width.start.unwrap_or(*default_start_width);
                        vertex.end_width = width.end.unwrap_or(*default_end_width);
                    }
                }
            }
            EntityType::LwPolyline(polyline)
        }
        IfccadEntityKind::SpatialPolyline {
            vertices,
            closed,
            line_pattern_generation,
        } => {
            let mut polyline =
                Polyline3D::from_points(vertices.iter().map(|vertex| vector(*vertex)).collect());
            polyline.flags.closed = *closed;
            polyline.flags.linetype_continuous =
                *line_pattern_generation == IfccadLinePatternGeneration::Continuous;
            EntityType::Polyline3D(polyline)
        }
        IfccadEntityKind::Solid(quadrilateral) | IfccadEntityKind::Trace(quadrilateral) => {
            let (origin, normal, rotation) = placement_in(&quadrilateral.placement);
            let [first, second, third, fourth] = quadrilateral.corners.map(|[u, v]| {
                let corner = frame_to_ocs(u, v, origin, rotation);
                Vector3::new(corner.x, corner.y, origin.z)
            });
            let mut solid = Solid::new(first, second, third, fourth)
                .with_normal(normal)
                .with_thickness(quadrilateral.thickness);
            solid.is_trace = matches!(kind, IfccadEntityKind::Trace(_));
            EntityType::Solid(solid)
        }
        IfccadEntityKind::Text(text) => {
            let style = names.styles.get(&text.style_id)?;
            let (origin, normal, frame) = placement_in(&text.placement);
            let mut converted = Text::new();
            converted.value = text::text_value(&text.content);
            converted.style = style.clone();
            converted.normal = normal;
            converted.thickness = text.thickness;
            converted.oblique_angle = text.oblique_angle;
            converted.generation_flags =
                i16::from(text.backward) * 2 + i16::from(text.upside_down) * 4;
            converted.insertion_point = origin;
            // A frame turned in its plane is a text rotation; written frames
            // are square to the OCS, so read them back exactly that way.
            converted.rotation = text.rotation + frame;
            type H = TextHorizontalAlignment;
            type V = TextVerticalAlignment;
            match text.layout {
                TextLayout::Anchored {
                    horizontal,
                    vertical,
                    height,
                    width_factor,
                } => {
                    converted.height = height;
                    converted.width_factor = width_factor;
                    converted.horizontal_alignment = match horizontal {
                        Horizontal::Left => H::Left,
                        Horizontal::Center => H::Center,
                        Horizontal::Right => H::Right,
                    };
                    converted.vertical_alignment = match vertical {
                        Vertical::Baseline => V::Baseline,
                        Vertical::Bottom => V::Bottom,
                        Vertical::Middle => V::Middle,
                        Vertical::Top => V::Top,
                    };
                    if (horizontal, vertical) != (Horizontal::Left, Vertical::Baseline) {
                        converted.alignment_point = Some(origin);
                    }
                }
                TextLayout::WholeTextMiddle {
                    height,
                    width_factor,
                } => {
                    converted.height = height;
                    converted.width_factor = width_factor;
                    converted.horizontal_alignment = H::Middle;
                    converted.alignment_point = Some(origin);
                }
                TextLayout::Aligned { length, .. } | TextLayout::Fit { length, .. } => {
                    match text.layout {
                        TextLayout::Aligned { width_factor, .. } => {
                            converted.horizontal_alignment = H::Aligned;
                            converted.width_factor = width_factor;
                        }
                        TextLayout::Fit { height, .. } => {
                            converted.horizontal_alignment = H::Fit;
                            converted.height = height;
                        }
                        _ => unreachable!(),
                    }
                    let (sin, cos) = converted.rotation.sin_cos();
                    converted.alignment_point =
                        Some(origin + Vector3::new(length * cos, length * sin, 0.0));
                }
            }
            EntityType::Text(converted)
        }
        IfccadEntityKind::MText(mtext) => return text::mtext_in(mtext, names, losses),
        IfccadEntityKind::Hatch(hatch) => hatch::hatch_in(hatch),
        IfccadEntityKind::Viewport(viewport) => {
            if layout::has_property_overrides(viewport) {
                losses.note("viewport layer property overrides");
            }
            layout::viewport_in(viewport, names)
        }
        IfccadEntityKind::BlockInstance {
            definition_id,
            transform,
        } => {
            let name = names.blocks.get(definition_id)?;
            let (origin, normal, frame) = placement_in(&transform.placement);
            let mut insert = Insert::new(name.clone(), origin);
            insert.normal = normal;
            insert.rotation = transform.rotation + frame;
            let [x, y, z] = transform.scale;
            insert.set_x_scale(x);
            insert.set_y_scale(y);
            insert.set_z_scale(z);
            if [insert.x_scale(), insert.y_scale(), insert.z_scale()].map(f64::to_bits)
                != transform.scale.map(f64::to_bits)
            {
                losses.note("BlockInstance scale cannot be represented by CAD INSERT setters");
                return None;
            }
            EntityType::Insert(insert)
        }
        _ => return None,
    })
}

/// A frame with origin `origin` in the object coordinate system of `normal`,
/// turned by `rotation` about it.
fn placement_out(origin: Vector3, normal: Vector3, rotation: f64) -> IfccadPlacement {
    // Quantizing the OCS normal can cross an arbitrary-axis branch while
    // retaining the old OCS origin/angle, producing a different world frame.
    let (origin, x, y) = Placement::from_ocs(origin, normal, rotation).to_axes();
    IfccadPlacement {
        origin: array(origin),
        x_axis: array(x),
        y_axis: array(y),
    }
}

/// OCS origin, normal and rotation of an IFCX frame.
fn placement_in(placement: &IfccadPlacement) -> (Vector3, Vector3, f64) {
    Placement::from_axes(
        vector(placement.origin),
        vector(placement.x_axis),
        vector(placement.y_axis),
    )
    .to_ocs()
}

/// Frame coordinates `(u, v)` in the OCS of a frame at `origin` turned by
/// `rotation`.
fn frame_to_ocs(u: f64, v: f64, origin: Vector3, rotation: f64) -> Vector2 {
    let (x, y) = match rotation {
        0.0 => (u, v),
        _ => {
            let (sin, cos) = rotation.sin_cos();
            (u * cos - v * sin, u * sin + v * cos)
        }
    };
    Vector2::new(x + origin.x, y + origin.y)
}

/// A world point or direction in the object coordinate system of `normal`.
fn to_ocs(point: Vector3, normal: Vector3) -> Vector3 {
    let m = Matrix3::arbitrary_axis(normal).m;
    let axis = |i: usize| m[0][i] * point.x + m[1][i] * point.y + m[2][i] * point.z;
    Vector3::new(axis(0), axis(1), axis(2))
}

fn generation(continuous: bool) -> IfccadLinePatternGeneration {
    match continuous {
        true => IfccadLinePatternGeneration::Continuous,
        false => IfccadLinePatternGeneration::PerSegment,
    }
}

fn appearance_out(
    common: &EntityCommon,
    line_patterns: &HashMap<String, IfccadLinePatternId>,
    losses: &mut Losses,
) -> IfccadEntityAppearance {
    let color = match common.color {
        Color::ByLayer => IfccadMode::ByLayer,
        Color::ByBlock => IfccadMode::ByBlock,
        color => match color_out(color) {
            Some(color) => IfccadMode::Explicit(color),
            None => {
                losses.note("an entity colour");
                IfccadMode::ByLayer
            }
        },
    };
    let line_pattern = match common.linetype.to_ascii_lowercase().as_str() {
        "" | "bylayer" => IfccadMode::ByLayer,
        "byblock" => IfccadMode::ByBlock,
        name => match line_patterns.get(name) {
            Some(id) => IfccadMode::Explicit(*id),
            None => {
                losses.note("an entity linetype");
                IfccadMode::ByLayer
            }
        },
    };
    let line_weight = match common.line_weight {
        LineWeight::ByBlock => IfccadMode::ByBlock,
        weight => weight_out(weight).map_or(IfccadMode::ByLayer, IfccadMode::Explicit),
    };
    let opacity = match common.transparency {
        Transparency::ByLayer => IfccadMode::ByLayer,
        Transparency::ByBlock => IfccadMode::ByBlock,
        Transparency::Explicit(amount) => IfccadMode::Explicit(1.0 - f64::from(amount) / 255.0),
    };
    IfccadEntityAppearance {
        color,
        opacity,
        line_pattern,
        line_weight,
    }
}

fn appearance_in(
    appearance: &IfccadEntityAppearance,
    names: &Names,
    common: &mut EntityCommon,
    losses: &mut Losses,
) {
    common.color = match &appearance.color {
        IfccadMode::ByLayer => Color::ByLayer,
        IfccadMode::ByBlock => Color::ByBlock,
        IfccadMode::Explicit(color) => color_in(color),
    };
    common.linetype = match appearance.line_pattern {
        IfccadMode::ByLayer => "ByLayer".to_string(),
        IfccadMode::ByBlock => "ByBlock".to_string(),
        IfccadMode::Explicit(pattern) => match names.line_patterns.get(&pattern) {
            Some(name) => name.clone(),
            None => {
                losses.note("an entity line pattern");
                "ByLayer".to_string()
            }
        },
    };
    common.line_weight = match appearance.line_weight {
        IfccadMode::ByLayer => LineWeight::ByLayer,
        IfccadMode::ByBlock => LineWeight::ByBlock,
        IfccadMode::Explicit(millimetres) => weight_in(millimetres),
    };
    common.transparency = match appearance.opacity {
        IfccadMode::ByLayer => Transparency::ByLayer,
        IfccadMode::ByBlock => Transparency::ByBlock,
        IfccadMode::Explicit(opacity) => transparency_in(opacity),
    };
}

fn transparency_in(opacity: f64) -> Transparency {
    Transparency::Explicit(((1.0 - opacity.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

fn unit_code(name: &str) -> i16 {
    UNITS.iter().position(|unit| *unit == name).unwrap_or(0) as i16
}

fn unit_name(code: i16) -> String {
    UNITS.get(code as usize).unwrap_or(&"unitless").to_string()
}

/// ACI 7 is the foreground colour; other indices keep their index identity.
fn color_out(color: Color) -> Option<IfccadColor> {
    match color {
        Color::Index(7) => Some(IfccadColor::Foreground),
        Color::Index(index) => {
            let (r, g, b) = color.rgb()?;
            Some(IfccadColor::Concrete {
                rgb: [r, g, b],
                indexed: Some(("ACI".to_string(), u64::from(index))),
                named: None,
            })
        }
        Color::Rgb { r, g, b } => Some(IfccadColor::Concrete {
            rgb: [r, g, b],
            indexed: None,
            named: None,
        }),
        _ => None,
    }
}

fn color_in(color: &IfccadColor) -> Color {
    match color {
        IfccadColor::Foreground => Color::Index(7),
        IfccadColor::Concrete { rgb, indexed, .. } => {
            let [r, g, b] = *rgb;
            match indexed {
                Some((system, index))
                    if system.eq_ignore_ascii_case("ACI")
                        && (1..=255).contains(index)
                        && Color::Index(*index as u8).rgb() == Some((r, g, b)) =>
                {
                    Color::Index(*index as u8)
                }
                _ => Color::Rgb { r, g, b },
            }
        }
    }
}

/// A line weight in millimetres; `None` for the inherited and default weights.
fn weight_out(weight: LineWeight) -> Option<f64> {
    match weight {
        LineWeight::Value(hundredths) => Some(f64::from(hundredths) / 100.0),
        _ => None,
    }
}

fn weight_in(millimetres: f64) -> LineWeight {
    LineWeight::Value((millimetres * 100.0).round() as i16)
}

fn vector([x, y, z]: [f64; 3]) -> Vector3 {
    Vector3::new(x, y, z)
}

fn array(v: Vector3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

fn kind_name(kind: &IfccadEntityKind) -> &'static str {
    match kind {
        IfccadEntityKind::Solid(_) => "Solid",
        IfccadEntityKind::Trace(_) => "Trace",
        IfccadEntityKind::Dimension(_) => "Dimension",
        IfccadEntityKind::Hatch(_) => "Hatch",
        IfccadEntityKind::Text(_) => "Text",
        IfccadEntityKind::MText(_) => "MText",
        IfccadEntityKind::Viewport(_) => "Viewport",
        IfccadEntityKind::Ellipse { .. } => "Ellipse",
        IfccadEntityKind::EllipseArc { .. } => "EllipseArc",
        IfccadEntityKind::SpatialPolyline { .. } => "SpatialPolyline",
        IfccadEntityKind::BlockInstance { .. } => "BlockInstance",
        _ => "an entity",
    }
}

fn set(properties: &mut Vec<(String, String)>, name: &str, value: String) {
    match properties.iter_mut().find(|(key, _)| key == name) {
        Some(entry) => entry.1 = value,
        None => properties.push((name.to_string(), value)),
    }
}

fn now_seconds() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `seconds` since the Unix epoch as an RFC 3339 UTC timestamp.
fn rfc3339(seconds: u64) -> String {
    // Civil date from a day count (Howard Hinnant's algorithm).
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

fn unit_direction(v: Vector3) -> Result<Vector3, Error> {
    let m = v.x.abs().max(v.y.abs()).max(v.z.abs());
    if ![v.x, v.y, v.z].into_iter().all(f64::is_finite) || m == 0. {
        return Err("extrusion normal must be finite and nonzero".into());
    }
    let a = Vector3::new(v.x / m, v.y / m, v.z / m);
    let n = a.x.hypot(a.y).hypot(a.z);
    Ok(Vector3::new(a.x / n, a.y / n, a.z / n))
}
fn point_placement(origin: Vector3, normal: Vector3, angle: f64) -> IfccadPlacement {
    let mut p = placement_out(Vector3::ZERO, normal, angle);
    p.origin = array(origin);
    p
}

#[cfg(test)]
mod tests {
    use super::{rfc3339, Losses};

    #[test]
    fn retained_notes_do_not_become_content_and_content_counts_add() {
        let mut a = Losses::default();
        a.note("same");
        let mut b = Losses::default();
        b.note("same");
        b.retained_note(crate::Loss {
            handle: None,
            message: "retained".into(),
        });
        a.merge(b);
        assert!(a.has_content_loss());
        assert_eq!(a.content_snapshot()["same"], 2);
        let reports = a.into_vec();
        assert_eq!(reports[0].message, "same: not converted (2×)");
        assert_eq!(reports[1].message, "retained");
        let mut retained = Losses::default();
        retained.retained_note(crate::Loss {
            handle: None,
            message: "retained".into(),
        });
        assert!(!retained.has_content_loss());
        assert!(retained.content_snapshot().is_empty());
    }

    #[test]
    fn timestamps_are_utc_civil_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_793_000_000), "2026-10-26T07:33:20Z");
    }
}
