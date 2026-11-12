//! Closed named maps and tagged tuples for the V1 projection.
use super::{error, frame, Result, SelectionStorageErrorKind as Kind};
use crate::selection::*;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

macro_rules! named_map {
    ($vis:vis $name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(serde::Serialize)]
        $vis struct $name { $($field: $ty),* }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                struct Named;
                impl<'de> serde::de::Visitor<'de> for Named {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str(concat!("named map for ", stringify!($name)))
                    }
                    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut m: M)
                        -> std::result::Result<$name, M::Error> {
                        use serde::de::Error;
                        $(let mut $field: Option<$ty> = None;)*
                        while let Some(key) = m.next_key::<String>()? {
                            match key.as_str() {
                                $(stringify!($field) => {
                                    if $field.is_some() {
                                        return Err(M::Error::duplicate_field(stringify!($field)));
                                    }
                                    $field = Some(m.next_value()?);
                                })*
                                _ => return Err(M::Error::unknown_field(&key, &[$(stringify!($field)),*])),
                            }
                        }
                        Ok($name { $($field: $field.ok_or_else(||
                            M::Error::missing_field(stringify!($field)))?),* })
                    }
                }
                d.deserialize_any(Named)
            }
        }
    }
}

macro_rules! tagged {
    ($vis:vis $name:ident { $($tag:literal => $variant:ident ($($arg:ident: $ty:ty),*)),* $(,)? }) => {
        $vis enum $name { $($variant($($ty),*)),* }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                match self {
                    $(Self::$variant($($arg),*) => serde::Serialize::serialize(&($tag, $($arg,)*), s)),*
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                struct Tagged;
                impl<'de> serde::de::Visitor<'de> for Tagged {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str(concat!("tagged tuple for ", stringify!($name)))
                    }
                    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A)
                        -> std::result::Result<$name, A::Error> {
                        use serde::de::Error;
                        let tag: String = next(&mut a)?;
                        let value = match tag.as_str() {
                            $($tag => $name::$variant($(next::<$ty, _>(&mut a)?),*)),*,
                            _ => return Err(A::Error::unknown_variant(&tag, &[$($tag),*])),
                        };
                        if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                            return Err(A::Error::custom("extra element in tagged tuple"));
                        }
                        Ok(value)
                    }
                }
                d.deserialize_any(Tagged)
            }
        }
    }
}

fn next<'de, T: Deserialize<'de>, A: de::SeqAccess<'de>>(
    a: &mut A,
) -> std::result::Result<T, A::Error> {
    a.next_element()?
        .ok_or_else(|| de::Error::custom("missing tagged tuple element"))
}

macro_rules! text_enum {
    ($name:ident($model:ident) { $($tag:literal => $variant:ident),* $(,)? }) => {
        struct $name($model);

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str(match self.0 { $($model::$variant => $tag),* })
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let tag = String::deserialize(d)?;
                match tag.as_str() {
                    $($tag => Ok(Self($model::$variant))),*,
                    _ => Err(de::Error::unknown_variant(&tag, &[$($tag),*])),
                }
            }
        }
    }
}

mod payload;
use payload::Payload;

text_enum!(RecordKind(SelectionRecordKind) {
    "entity" => Entity, "layer" => Layer, "linetype" => LineType, "object" => Object,
});
text_enum!(Role(ContextRole) {
    "model_space" => ModelSpace, "mline_style_dictionary" => MLineStyleDictionary,
    "by_layer" => ByLayer, "by_block" => ByBlock,
});
text_enum!(EvidenceCode(CaptureEvidenceCode) {
    "missing_dependency" => MissingDependency, "excluded_raw_record" => ExcludedRawRecord,
    "excluded_graphics" => ExcludedGraphics, "excluded_derived_data" => ExcludedDerivedData,
    "resolved_source_default" => ResolvedSourceDefault, "unknown_units" => UnknownUnits,
});

named_map!(Key {
    kind: RecordKind,
    handle: u64
});
named_map!(Record {
    key: Key,
    payload: Payload
});
tagged!(Slot {
    "owner" => Owner(), "layer" => Layer(), "linetype" => LineType(),
    "mline_style" => MLineStyle(), "style_element_linetype" => StyleElementLineType(index: u32),
    "style_registration" => StyleRegistration(),
});
tagged!(Disposition {
    "included" => Included(key: Key), "external" => External(key: Key),
    "missing" => Missing(), "context" => Context(role: Role),
});
named_map!(Requirement {
    source: Key, slot: Slot, source_handle: Option<u64>, source_name: Option<String>,
    disposition: Disposition,
});
named_map!(Evidence {
    source: Option<Key>, slot: Option<Slot>, code: EvidenceCode, message: String,
});
named_map!(Context {
    insertion_units: i16,
    linetype_scale: u64
});
named_map!(Selection {
    namespace: String, roots: Vec<Key>, records: Vec<Record>, requirements: Vec<Requirement>,
    report: Vec<Evidence>, context: Context,
});

impl From<SelectionRecordKey> for Key {
    fn from(key: SelectionRecordKey) -> Self {
        Self {
            kind: RecordKind(key.kind),
            handle: key.handle.value(),
        }
    }
}
impl From<Key> for SelectionRecordKey {
    fn from(key: Key) -> Self {
        Self {
            kind: key.kind.0,
            handle: Handle::new(key.handle),
        }
    }
}
impl TryFrom<&SelectionSlot> for Slot {
    type Error = super::SelectionStorageError;
    fn try_from(s: &SelectionSlot) -> Result<Self> {
        Ok(match s {
            SelectionSlot::Owner => Self::Owner(),
            SelectionSlot::Layer => Self::Layer(),
            SelectionSlot::LineType => Self::LineType(),
            SelectionSlot::MLineStyle => Self::MLineStyle(),
            SelectionSlot::StyleElementLineType(i) => Self::StyleElementLineType(count32(*i)?),
            SelectionSlot::StyleRegistration => Self::StyleRegistration(),
        })
    }
}
impl From<Slot> for SelectionSlot {
    fn from(s: Slot) -> Self {
        match s {
            Slot::Owner() => Self::Owner,
            Slot::Layer() => Self::Layer,
            Slot::LineType() => Self::LineType,
            Slot::MLineStyle() => Self::MLineStyle,
            Slot::StyleElementLineType(i) => Self::StyleElementLineType(i as usize),
            Slot::StyleRegistration() => Self::StyleRegistration,
        }
    }
}
impl From<&RequirementDisposition> for Disposition {
    fn from(d: &RequirementDisposition) -> Self {
        match d {
            RequirementDisposition::Included(k) => Self::Included((*k).into()),
            RequirementDisposition::External(k) => Self::External((*k).into()),
            RequirementDisposition::Missing => Self::Missing(),
            RequirementDisposition::Context(r) => Self::Context(Role(*r)),
        }
    }
}
impl From<Disposition> for RequirementDisposition {
    fn from(d: Disposition) -> Self {
        match d {
            Disposition::Included(k) => Self::Included(k.into()),
            Disposition::External(k) => Self::External(k.into()),
            Disposition::Missing() => Self::Missing,
            Disposition::Context(r) => Self::Context(r.0),
        }
    }
}

fn count32(n: usize) -> Result<u32> {
    u32::try_from(n).map_err(|_| error(Kind::LimitExceeded, "selection count exceeds u32"))
}

// Bound the variable-size allocations in the DTO before building its vectors and
// cloning strings. The completed frame is also checked for its full wire budget.
struct Projection {
    array_items: usize,
    string_bytes: usize,
}
impl Projection {
    fn list<T, U>(
        &mut self,
        values: &[T],
        mut project: impl FnMut(&mut Self, &T) -> Result<U>,
    ) -> Result<Vec<U>> {
        if values.len() > frame::COUNT_LIMIT || values.len() > self.array_items {
            return Err(error(Kind::LimitExceeded, "selection exceeds array budget"));
        }
        self.array_items -= values.len();
        values.iter().map(|v| project(self, v)).collect()
    }

    fn text(&mut self, text: &str) -> Result<String> {
        if text.len() > frame::STRING_LIMIT || text.len() > self.string_bytes {
            return Err(error(Kind::LimitExceeded, "selection exceeds text budget"));
        }
        self.string_bytes -= text.len();
        Ok(text.to_owned())
    }

    fn optional_text(&mut self, text: &Option<String>) -> Result<Option<String>> {
        text.as_deref().map(|s| self.text(s)).transpose()
    }
}

pub(super) fn read_v1(body: &[u8]) -> Result<DocumentSelection> {
    let w: Selection =
        rmp_serde::from_slice(body).map_err(|e| error(Kind::InvalidBody, e.to_string()))?;
    Ok(DocumentSelection {
        namespace: w.namespace,
        roots: w.roots.into_iter().map(Into::into).collect(),
        records: w
            .records
            .into_iter()
            .map(|r| {
                Ok(SelectionRecord {
                    key: r.key.into(),
                    payload: r.payload.into_model()?,
                })
            })
            .collect::<Result<_>>()?,
        requirements: w
            .requirements
            .into_iter()
            .map(|r| SelectionRequirement {
                source: r.source.into(),
                slot: r.slot.into(),
                source_handle: r.source_handle.map(Handle::new),
                source_name: r.source_name,
                disposition: r.disposition.into(),
            })
            .collect(),
        report: w
            .report
            .into_iter()
            .map(|e| CaptureEvidence {
                source: e.source.map(Into::into),
                slot: e.slot.map(Into::into),
                code: e.code.0,
                message: e.message,
            })
            .collect(),
        context: SelectionContext {
            insertion_units: w.context.insertion_units,
            linetype_scale: f64::from_bits(w.context.linetype_scale),
        },
    })
}

pub(super) fn write_v1(s: &DocumentSelection) -> Result<Vec<u8>> {
    let mut p = Projection {
        array_items: frame::VALUE_LIMIT,
        string_bytes: frame::BODY_LIMIT,
    };
    let w = Selection {
        namespace: p.text(&s.namespace)?,
        roots: p.list(&s.roots, |_, k| Ok((*k).into()))?,
        records: p.list(&s.records, |p, r| {
            Ok(Record {
                key: r.key.into(),
                payload: Payload::from_model(p, &r.payload)?,
            })
        })?,
        requirements: p.list(&s.requirements, |p, r| {
            Ok(Requirement {
                source: r.source.into(),
                slot: Slot::try_from(&r.slot)?,
                source_handle: r.source_handle.map(|h| h.value()),
                source_name: p.optional_text(&r.source_name)?,
                disposition: (&r.disposition).into(),
            })
        })?,
        report: p.list(&s.report, |p, e| {
            Ok(Evidence {
                source: e.source.map(Into::into),
                slot: e.slot.as_ref().map(Slot::try_from).transpose()?,
                code: EvidenceCode(e.code),
                message: p.text(&e.message)?,
            })
        })?,
        context: Context {
            insertion_units: s.context.insertion_units,
            linetype_scale: s.context.linetype_scale.to_bits(),
        },
    };
    let mut out = frame::BoundedFrame::new();
    if let Err(e) = w.serialize(&mut rmp_serde::Serializer::new(&mut out).with_struct_map()) {
        return Err(error(
            if out.exceeded {
                Kind::LimitExceeded
            } else {
                Kind::InvalidSelection
            },
            e.to_string(),
        ));
    }
    out.finish()
}
