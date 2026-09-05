//! Versioned payload envelopes. Unknown payloads must never pass through
//! `ron::Value`, which loses enum variant names.

use std::collections::BTreeMap;

use ::serde::{Deserialize, Serialize, de::DeserializeOwned};
use ron::value::RawValue;

use crate::{
    annotation::Annotation,
    annotation_list::AnnotationGroup,
    displayed_item::{DisplayedItem, DisplayedItemRef},
    displayed_item_tree::DisplayedItemTree,
    graphics::{Graphic, GraphicId},
    item_list::ItemList,
    variable_name_type::VariableNameType,
};

pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PARSE_DEPTH: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("state exceeds the {MAX_STATE_BYTES}-byte input limit")]
    TooLarge,
    #[error(transparent)]
    Ron(#[from] ron::error::SpannedError),
}

/// Bound input before parsing, including before buffering a raw kind payload.
pub fn decode<T: DeserializeOwned>(input: &str) -> Result<T, DecodeError> {
    decode_bytes(input.as_bytes())
}

pub fn decode_bytes<T: DeserializeOwned>(input: &[u8]) -> Result<T, DecodeError> {
    if input.len() > MAX_STATE_BYTES {
        return Err(DecodeError::TooLarge);
    }
    Ok(ron::Options::default()
        .with_recursion_limit(MAX_PARSE_DEPTH)
        .from_bytes(input)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileFile {
    pub title: Option<String>,
    pub kind: String,
    pub kind_version: u32,
    pub payload: Box<RawValue>,
}

impl TileFile {
    pub fn encode<T: Serialize>(
        title: Option<String>,
        kind: &str,
        kind_version: u32,
        payload: &T,
    ) -> Result<Self, ron::Error> {
        Ok(Self {
            title,
            kind: kind.into(),
            kind_version,
            payload: RawValue::from_rust(payload)?,
        })
    }

    /// Call only after registry dispatch has recognized the kind and version.
    /// Errors from known payloads are load errors, never unknown placeholders.
    pub fn decode_payload<T: DeserializeOwned>(&self) -> Result<T, DecodeError> {
        decode(self.payload.get_ron())
    }
}

/// Serialized list content has no layout caches or tile navigation.
#[derive(Serialize, Deserialize)]
pub struct ItemListFile {
    pub items_tree: DisplayedItemTree,
    pub displayed_items: BTreeMap<DisplayedItemRef, DisplayedItem>,
    pub ref_counter: usize,
    pub default_variable_name_type: VariableNameType,
    pub annotations: Vec<Annotation>,
    pub annotation_groups: Vec<AnnotationGroup>,
    pub annotation_counter: i32,
    pub graphics: BTreeMap<GraphicId, Graphic>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid item-list tree, item references, or identity counter")]
pub struct ItemListError;

impl ItemListFile {
    pub fn validate(&self) -> Result<(), ItemListError> {
        let mut seen = std::collections::BTreeSet::new();
        let mut previous: Option<(u8, bool)> = None;
        for node in self.items_tree.iter() {
            let Some(item) = self.displayed_items.get(&node.item_ref) else {
                return Err(ItemListError);
            };
            if !seen.insert(node.item_ref) || node.item_ref.0 == 0 {
                return Err(ItemListError);
            }
            match previous {
                None if node.level != 0 => return Err(ItemListError),
                Some((level, group))
                    if node.level > level
                        && (!group || level.checked_add(1) != Some(node.level)) =>
                {
                    return Err(ItemListError);
                }
                _ => {}
            }
            previous = Some((node.level, matches!(item, DisplayedItem::Group(_))));
        }
        if seen.len() != self.displayed_items.len()
            || self.ref_counter == usize::MAX
            || seen.last().is_some_and(|id| id.0 > self.ref_counter)
        {
            return Err(ItemListError);
        }
        Ok(())
    }
}

impl From<&ItemList> for ItemListFile {
    fn from(list: &ItemList) -> Self {
        Self {
            items_tree: list.items_tree.clone(),
            displayed_items: list
                .displayed_items
                .iter()
                .map(|(id, item)| (*id, item.clone()))
                .collect(),
            ref_counter: list.display_item_ref_counter,
            default_variable_name_type: list.default_variable_name_type,
            annotations: list.annotations.clone(),
            annotation_groups: list.annotation_groups.clone(),
            annotation_counter: list.annotation_counter,
            graphics: list
                .graphics
                .iter()
                .map(|(id, graphic)| (id.clone(), graphic.clone()))
                .collect(),
        }
    }
}

impl From<ItemListFile> for ItemList {
    fn from(file: ItemListFile) -> Self {
        Self {
            items_tree: file.items_tree,
            displayed_items: file.displayed_items.into_iter().collect(),
            display_item_ref_counter: file.ref_counter,
            default_variable_name_type: file.default_variable_name_type,
            annotations: file.annotations,
            annotation_groups: file.annotation_groups,
            annotation_counter: file.annotation_counter,
            graphics: file.graphics.into_iter().collect(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    enum Nested {
        Unit,
        Tuple(u32, Box<Nested>),
        Record { children: Vec<Nested> },
    }

    #[test]
    fn unknown_kind_and_future_version_keep_nested_enum_payloads() {
        // This fixture is decoded without knowing its kind or version. A future
        // reader can still recover the exact typed enum after an older build saves.
        let source = include_str!("fixtures/future-tile.ron");
        let mut envelope: TileFile = decode(source).unwrap();
        let original_payload = envelope.payload.get_ron().to_owned();
        envelope.title = Some("Renamed future tile".into());
        let serialized = ron::to_string(&envelope).unwrap();
        let restored: TileFile = decode(&serialized).unwrap();
        assert_eq!(restored.kind, "future.pipeline");
        assert_eq!(restored.kind_version, 17);
        assert_eq!(restored.title.as_deref(), Some("Renamed future tile"));
        assert_eq!(restored.payload.get_ron(), original_payload);
        assert_eq!(
            restored.decode_payload::<Nested>().unwrap(),
            Nested::Record {
                children: vec![
                    Nested::Tuple(42, Box::new(Nested::Unit)),
                    Nested::Record { children: vec![] }
                ],
            }
        );
    }

    #[test]
    fn typed_known_payload_errors_are_not_converted_to_unknown() {
        let file = TileFile::encode(
            None,
            "example",
            1,
            &Nested::Tuple(1, Box::new(Nested::Unit)),
        )
        .unwrap();
        assert!(file.decode_payload::<Vec<String>>().is_err());
        assert_eq!(
            file.decode_payload::<Nested>().unwrap(),
            Nested::Tuple(1, Box::new(Nested::Unit))
        );
    }

    #[test]
    fn list_file_does_not_store_runtime_geometry() {
        let list = ItemList {
            layout_cache: std::cell::RefCell::new(crate::item_list::ItemLayoutCache {
                signature: Some(7),
                total_height: 90.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        let serialized = ron::to_string(&ItemListFile::from(&list)).unwrap();
        assert!(!serialized.contains("drawing_infos"));
        assert!(!serialized.contains("total_height"));
        let restored = ItemList::from(decode::<ItemListFile>(&serialized).unwrap());
        assert!(restored.layout_cache.borrow().signature.is_none());
        assert_eq!(restored.layout_cache.borrow().total_height, 0.0);
    }

    #[test]
    fn list_validation_rejects_broken_references_nesting_and_counters() {
        use crate::displayed_item::DisplayedDivider;
        let divider = DisplayedItem::Divider(DisplayedDivider {
            color: None,
            background_color: None,
            name: None,
        });
        let make = |nodes: &[(usize, u8)]| {
            let nodes = nodes
                .iter()
                .map(|(id, level)| {
                    format!("(item_ref:({id}),level:{level},unfolded:true,selected:false)")
                })
                .collect::<Vec<_>>()
                .join(",");
            let mut file = ItemListFile::from(&ItemList::default());
            file.items_tree = decode(&format!("(items:[{nodes}])")).unwrap();
            file.displayed_items
                .insert(DisplayedItemRef(1), divider.clone());
            file.displayed_items
                .insert(DisplayedItemRef(2), divider.clone());
            file.ref_counter = 2;
            file
        };
        make(&[(1, 0), (2, 0)]).validate().unwrap();
        for nodes in [
            vec![(1, 0), (1, 0)],
            vec![(1, 0), (3, 0)],
            vec![(1, 1), (2, 0)],
            vec![(1, 0), (2, 1)],
            vec![(1, 0)],
        ] {
            assert!(make(&nodes).validate().is_err());
        }
        for counter in [0, 1, usize::MAX] {
            let mut file = make(&[(1, 0), (2, 0)]);
            file.ref_counter = counter;
            assert!(file.validate().is_err());
        }
    }

    #[test]
    fn parser_limits_apply_even_to_unknown_raw_payloads() {
        let nested = format!(
            "{}0{}",
            "[".repeat(MAX_PARSE_DEPTH + 1),
            "]".repeat(MAX_PARSE_DEPTH + 1)
        );
        let input = format!("(title:None,kind:\"future\",kind_version:1,payload:{nested})");
        assert!(decode::<TileFile>(&input).is_err());
    }
}
