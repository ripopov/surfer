use std::collections::HashMap;

use eyre::{Result, WrapErr as _};
use num::bigint::ToBigInt as _;
use num::{BigInt, One, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
use surfer_translation_types::{TranslationPreference, Translator, VariableValue};
use tracing::{error, info, warn};

use crate::data_container::DataContainer;
use crate::displayed_item::{
    DisplayedDivider, DisplayedFieldRef, DisplayedGroup, DisplayedItem, DisplayedItemRef,
    DisplayedStream, DisplayedTimeLine, DisplayedVariable,
};
use crate::displayed_item_tree::{DisplayedItemTree, ItemIndex, TargetPosition, VisibleItemIndex};
use crate::transaction_container::{StreamScopeRef, TransactionStreamRef};
use crate::translation::{DynTranslator, TranslatorList, VariableInfoExt};
use crate::variable_name_type::VariableNameType;
use crate::viewport::Viewport;
use crate::wave_container::{
    AnalogCacheKey, ScopeRef, ScopeRefExt as _, VariableMeta, VariableRef, VariableRefExt,
    WaveContainer,
};
use crate::wave_source::{WaveFormat, WaveSource};
use crate::wellen::LoadSignalsCmd;
use ftr_parser::types::StreamId;
use itertools::Itertools;
use std::fmt::Formatter;
use std::ops::Not;

pub const PER_SCROLL_EVENT: f32 = 50.0;
pub const SCROLL_EVENTS_PER_PAGE: f32 = 20.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScopeType {
    WaveScope(ScopeRef),
    StreamScope(StreamScopeRef),
}

impl std::fmt::Display for ScopeType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeType::WaveScope(w) => w.fmt(f),
            ScopeType::StreamScope(s) => s.fmt(f),
        }
    }
}

/// The shared loaded document. This has no item list, view navigation or UI cache.
#[derive(Serialize, Deserialize)]
pub struct WaveData {
    #[serde(skip, default = "DataContainer::__new_empty")]
    pub inner: DataContainer,
    pub source: WaveSource,
    pub format: WaveFormat,
    pub active_scope: Option<ScopeType>,
    pub cursor: Option<BigInt>,
    pub markers: HashMap<u8, BigInt>,
    pub display_variable_indices: bool,
    #[serde(skip)]
    pub old_max_timestamp: Option<BigInt>,
    #[serde(skip)]
    pub cache_generation: u64,
    #[serde(skip)]
    pub inflight_caches:
        HashMap<AnalogCacheKey, std::sync::Arc<crate::analog_signal_cache::AnalogCacheEntry>>,
    #[serde(skip)]
    pub cached_time_range: TimeRange,
}

/// Immutable inputs for the resolved waveform tile.
#[derive(Clone, Copy)]
pub(crate) struct WaveformRead<'a> {
    pub document: &'a WaveData,
    pub items: &'a crate::item_list::ItemList,
    pub view: &'a crate::tile_kinds::waveform::WaveformView,
    pub tile_id: crate::tiles::TileId,
}
impl std::ops::Deref for WaveformRead<'_> {
    type Target = WaveData;
    fn deref(&self) -> &Self::Target {
        self.document
    }
}

/// Checked editing access to one waveform and its shared list. Resources stay
/// in their owners throughout the operation; peers borrow the same list.
pub(crate) struct WaveformEdit<'a> {
    pub document: &'a mut WaveData,
    pub items: &'a mut crate::item_list::ItemList,
    pub view: &'a mut crate::tile_kinds::waveform::WaveformView,
    pub peers: Vec<&'a mut crate::tile_kinds::waveform::WaveformView>,
}

impl std::ops::Deref for WaveformEdit<'_> {
    type Target = WaveData;
    fn deref(&self) -> &Self::Target {
        self.document
    }
}
impl std::ops::DerefMut for WaveformEdit<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.document
    }
}

#[derive(Debug, Clone)]
pub struct TimeRange {
    pub start: BigInt,
    pub end: BigInt,
}

impl TimeRange {
    #[must_use]
    pub fn length(&self) -> BigInt {
        &self.end - &self.start
    }
}

impl Default for TimeRange {
    fn default() -> Self {
        Self {
            start: BigInt::zero(),
            end: BigInt::one(),
        }
    }
}

pub(crate) fn select_preferred_translator(
    var: &VariableMeta,
    translators: &TranslatorList,
) -> String {
    let mut preferred: Vec<_> = translators
        .all_translators()
        .iter()
        .filter_map(|t| match t.translates(var) {
            Ok(TranslationPreference::Prefer) => Some(t.name()),
            Ok(TranslationPreference::Yes) => None,
            Ok(TranslationPreference::No) => None,
            Err(e) => {
                error!(
                    "Failed to check if {} translates {}\n{e:#?}",
                    t.name(),
                    var.var.full_path_string_no_index()
                );
                None
            }
        })
        .collect();
    if preferred.len() > 1 {
        // For a single bit that has other preferred translators in addition to "Bit", like enum,
        // we would like to select the other one.
        if var.num_bits == Some(1) {
            let bit = "Bit".to_string();
            preferred.retain(|x| x != &bit);
        } else {
            // Remove Signed from the list of preferred translators. This can happen for some SystemVerilog files which reports enums as signed.
            let signed = "Signed".to_string();
            preferred.retain(|x| x != &signed);
            if preferred.len() > 1 {
                // Remove Enum from the list of preferred translators. Probably there is an external translator that wants this.
                // TODO: Make it possible to detect external translators and use that there.
                let enum_translator = "Enum".to_string();
                preferred.retain(|x| x != &enum_translator);
            }
        }
        if preferred.len() > 1 {
            warn!(
                "More than one preferred translator for variable {} in scope {}: {}",
                var.var.name,
                var.var.path.full_name(),
                preferred.join(", ")
            );
            preferred.sort();
        }
    }
    // make sure we always pick the same translator, at least
    preferred
        .pop()
        .unwrap_or_else(|| translators.default.clone())
}

pub fn variable_translator<'a, F>(
    translator: Option<&String>,
    field: &[String],
    translators: &'a TranslatorList,
    meta: F,
) -> &'a DynTranslator
where
    F: FnOnce() -> Result<VariableMeta>,
{
    let translator_name = translator.cloned().unwrap_or_else(|| {
        if field.is_empty() {
            meta().as_ref().map_or_else(
                |e| {
                    warn!("{e:#?}");
                    translators.default.clone()
                },
                |meta| select_preferred_translator(meta, translators).clone(),
            )
        } else {
            translators.default.clone()
        }
    });

    (translators.get_translator(&translator_name)) as _
}

/// Reattach displayed items to a (re)loaded container, dropping or keeping
/// unavailable variables and pruning their rows from the tree.
pub(crate) fn update_displayed_items(
    waves: &WaveContainer,
    items: &HashMap<DisplayedItemRef, DisplayedItem>,
    keep_unavailable: bool,
    translators: &TranslatorList,
    items_tree: &mut DisplayedItemTree,
) -> HashMap<DisplayedItemRef, DisplayedItem> {
    items
        .iter()
        .filter_map(|(&id, i)| {
            let new = match i {
                // keep without a change
                DisplayedItem::Divider(_)
                | DisplayedItem::Marker(_)
                | DisplayedItem::TimeLine(_)
                | DisplayedItem::Stream(_)
                | DisplayedItem::Group(_) => Some((id, i.clone())),
                DisplayedItem::Variable(s) => s.update(waves, keep_unavailable).map(|r| (id, r)),
                DisplayedItem::Placeholder(p) => match waves.update_variable_ref(&p.variable_ref) {
                    None => {
                        if keep_unavailable {
                            Some((id, DisplayedItem::Placeholder(p.clone())))
                        } else {
                            None
                        }
                    }
                    Some(new_variable_ref) => {
                        let Ok(meta) = waves
                            .variable_meta(&new_variable_ref)
                            .context("When updating")
                            .map_err(|e| error!("{e:#?}"))
                        else {
                            return Some((id, DisplayedItem::Placeholder(p.clone())));
                        };
                        let translator =
                            variable_translator(p.format.as_ref(), &[], translators, || {
                                Ok(meta.clone())
                            });
                        let info = translator.variable_info(&meta).unwrap();
                        Some((
                            id,
                            DisplayedItem::Variable(
                                p.clone().into_variable(info, new_variable_ref),
                            ),
                        ))
                    }
                },
            };

            // remove element from item_tree if we are about to remove it from the displayed_items
            // we only remove variables or placeholders, so we don't have to think about traversing
            if new.is_none() {
                let removed = items_tree.drain_recursive_if(|n| n.item_ref == id);
                assert!(
                    removed.len() <= 1,
                    "more elements removed then should be possible"
                );
            }

            new
        })
        .collect()
}

impl WaveformEdit<'_> {
    pub(crate) fn update_metadata(&mut self, translators: &TranslatorList) {
        for di in self.items.displayed_items.values_mut() {
            let DisplayedItem::Variable(displayed_variable) = di else {
                continue;
            };

            let meta = self
                .document
                .inner
                .as_waves()
                .unwrap()
                .variable_meta(&displayed_variable.variable_ref.clone())
                .unwrap();
            let translator =
                variable_translator(displayed_variable.get_format(&[]), &[], translators, || {
                    Ok(meta.clone())
                });
            let info = translator.variable_info(&meta).ok();

            match info {
                Some(info) => displayed_variable
                    .field_formats
                    .retain(|ff| info.has_subpath(&ff.field)),
                _ => displayed_variable.field_formats.clear(),
            }

            displayed_variable.downgrade_type_limits_if_unsupported(translator, &meta);
        }
    }
    pub(crate) fn load_waves(&mut self) -> Option<LoadSignalsCmd> {
        let variables = self
            .items
            .displayed_items
            .values()
            .filter_map(|item| match item {
                DisplayedItem::Variable(r) => Some(&r.variable_ref),
                _ => None,
            });
        self.document
            .inner
            .as_waves_mut()
            .unwrap()
            .load_variables(variables)
            .expect("internal error: failed to load variables")
    }
    pub(crate) fn reattach(
        &mut self,
        translators: &TranslatorList,
        keep_unavailable: bool,
    ) -> Option<LoadSignalsCmd> {
        let container = self.document.inner.as_waves()?;
        let focus = self.view.focus_snapshot(self.items);
        let peer_focus = self
            .peers
            .iter()
            .map(|view| view.focus_snapshot(self.items))
            .collect::<Vec<_>>();
        self.items.displayed_items = update_displayed_items(
            container,
            &self.items.displayed_items,
            keep_unavailable,
            translators,
            &mut self.items.items_tree,
        );
        self.view.reconcile_item_focus(self.items, focus);
        for (view, focus) in self.peers.iter_mut().zip(peer_focus) {
            view.reconcile_item_focus(self.items, focus);
        }
        self.update_metadata(translators);
        self.load_waves()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_variables(
        &mut self,
        translators: &TranslatorList,
        variables: Vec<VariableRef>,
        target_position: Option<TargetPosition>,
        update_display_names: bool,
        ignore_failures: bool,
        variable_name_type: Option<VariableNameType>,
        move_focus: bool,
    ) -> (Option<LoadSignalsCmd>, Vec<DisplayedItemRef>) {
        let mut indices = vec![];
        // load variables from waveform
        let res = match self
            .document
            .inner
            .as_waves_mut()
            .unwrap()
            .load_variables(variables.iter())
        {
            Err(e) => {
                error!("{e:#?}");
                return (None, indices);
            }
            Ok(res) => res,
        };

        // initialize translator and add display item
        let mut target_position = target_position
            .or_else(|| {
                self.items
                    .insert_position(self.view.focused_index(self.items))
            })
            .unwrap_or(self.items.end_insert_position());
        for variable in variables {
            let Ok(meta) = self
                .document
                .inner
                .as_waves()
                .unwrap()
                .variable_meta(&variable)
                .context("When adding variable")
                .map_err(|e| error!("{e:#?}"))
            else {
                if ignore_failures {
                    continue;
                }
                return (res, indices);
            };

            let translator = variable_translator(None, &[], translators, || Ok(meta.clone()));
            let info = translator.variable_info(&meta).unwrap();

            let new_variable = DisplayedItem::Variable(DisplayedVariable {
                variable_ref: variable.clone(),
                info,
                color: None,
                background_color: None,
                display_name: variable.name.clone(),
                display_name_type: variable_name_type
                    .unwrap_or(self.items.default_variable_name_type),
                manual_name: None,
                format: None,
                field_formats: vec![],
                height_scaling_factor: None,
                analog: None,
                unfolded_fields: ahash::AHashSet::new(),
            });

            let id = match self.insert_item(new_variable, Some(target_position), false) {
                Ok(id) => id,
                Err(error) => {
                    error!(%error, "variable insertion rejected");
                    return (res, indices);
                }
            };
            if move_focus && self.view.focused_item.is_some() {
                self.view.focused_item = Some(id);
            }
            indices.push(id);
            target_position = TargetPosition {
                before: ItemIndex(target_position.before.0 + 1),
                level: target_position.level,
            }
        }

        if update_display_names {
            self.items.compute_variable_display_names(
                &self.document.inner,
                self.document.display_variable_indices,
            );
        }
        (res, indices)
    }

    pub fn remove_displayed_items(&mut self, ids: &[DisplayedItemRef]) {
        let focused = self.view.focus_snapshot(self.items);
        let peers = self
            .peers
            .iter()
            .map(|view| view.focus_snapshot(self.items))
            .collect::<Vec<_>>();
        if !self.items.remove_items(ids) {
            return;
        }
        self.view.reconcile_item_focus(self.items, focused);
        self.view.reconcile_annotations(self.items);
        self.view.invalidate_draw_cache();
        for (view, previous) in self.peers.iter_mut().zip(peers) {
            view.reconcile_item_focus(self.items, previous);
            view.reconcile_annotations(self.items);
            view.invalidate_draw_cache();
        }
    }

    pub fn add_divider(
        &mut self,
        name: Option<String>,
        vidx: Option<VisibleItemIndex>,
    ) -> Result<DisplayedItemRef, crate::item_list::ItemEditError> {
        let position = vidx
            .map(|index| {
                self.items
                    .insert_position(Some(index))
                    .ok_or(crate::item_list::ItemEditError::Placement)
            })
            .transpose()?;
        self.insert_item(
            DisplayedItem::Divider(DisplayedDivider {
                color: None,
                background_color: None,
                name,
            }),
            position,
            true,
        )
    }

    pub fn add_timeline(
        &mut self,
        vidx: Option<VisibleItemIndex>,
    ) -> Result<DisplayedItemRef, crate::item_list::ItemEditError> {
        let position = vidx
            .map(|index| {
                self.items
                    .insert_position(Some(index))
                    .ok_or(crate::item_list::ItemEditError::Placement)
            })
            .transpose()?;
        self.insert_item(
            DisplayedItem::TimeLine(DisplayedTimeLine {
                color: None,
                background_color: None,
                name: None,
            }),
            position,
            true,
        )
    }

    pub fn add_group(
        &mut self,
        name: String,
        target_position: Option<TargetPosition>,
    ) -> Result<DisplayedItemRef, crate::item_list::ItemEditError> {
        self.insert_item(
            DisplayedItem::Group(DisplayedGroup {
                name,
                color: None,
                background_color: None,
                content: vec![],
                is_open: false,
            }),
            target_position,
            true,
        )
    }

    pub fn add_generator(
        &mut self,
        gen_ref: TransactionStreamRef,
    ) -> Result<(), crate::item_list::ItemEditError> {
        let Some(gen_id) = gen_ref.gen_id else {
            return Ok(());
        };
        let Some(transactions) = self.document.inner.as_transactions_mut() else {
            return Ok(());
        };
        let is_empty = {
            let Some(generator) = transactions.get_generator(gen_id) else {
                return Ok(());
            };
            generator.transactions.is_empty()
        };
        if is_empty && !transactions.is_native() {
            info!("(Generator {gen_id}) Loading transactions into memory!");
            match transactions
                .inner
                .load_stream_into_memory(gen_ref.stream_id)
            {
                Ok(()) => info!("(Generator {gen_id}) Finished loading transactions!"),
                Err(_) => return Ok(()),
            }
        }

        let row_count = transactions
            .track_index(&gen_ref)
            .map_or(1, |index| index.row_count().max(1));

        let new_gen = DisplayedItem::Stream(DisplayedStream {
            display_name: gen_ref.name.clone(),
            transaction_stream_ref: gen_ref,
            color: None,
            background_color: None,
            manual_name: None,
            rows: row_count,
        });

        self.insert_item(new_gen, None, true)?;
        Ok(())
    }

    pub fn add_stream(
        &mut self,
        stream_ref: TransactionStreamRef,
    ) -> Result<(), crate::item_list::ItemEditError> {
        if self
            .document
            .inner
            .as_transactions_mut()
            .unwrap()
            .get_stream(stream_ref.stream_id)
            .unwrap()
            .transactions_loaded
            .not()
        {
            info!("(Stream) Loading transactions into memory!");
            match self
                .document
                .inner
                .as_transactions_mut()
                .unwrap()
                .inner
                .load_stream_into_memory(stream_ref.stream_id)
            {
                Ok(()) => info!(
                    "(Stream {}) Finished loading transactions!",
                    stream_ref.stream_id
                ),
                Err(_) => return Ok(()),
            }
        }

        let row_count = self
            .document
            .inner
            .as_transactions()
            .unwrap()
            .track_index(&stream_ref)
            .map_or(1, |index| index.row_count().max(1));
        let new_stream = DisplayedItem::Stream(DisplayedStream {
            display_name: stream_ref.name.clone(),
            transaction_stream_ref: stream_ref,
            color: None,
            background_color: None,
            manual_name: None,
            rows: row_count,
        });

        self.insert_item(new_stream, None, true)?;
        Ok(())
    }

    pub fn add_all_streams(&mut self) -> Result<(), crate::item_list::ItemEditError> {
        let mut streams: Vec<(StreamId, String)> = vec![];
        for stream in self.document.inner.as_transactions().unwrap().get_streams() {
            streams.push((stream.id, stream.name.clone()));
        }

        for (id, name) in streams
            .into_iter()
            .sorted_by(|a, b| numeric_sort::cmp(&a.1, &b.1))
        {
            self.add_stream(TransactionStreamRef::new_stream(id, name))?;
        }
        Ok(())
    }

    pub(crate) fn insert_item(
        &mut self,
        new_item: DisplayedItem,
        target_position: Option<TargetPosition>,
        move_focus: bool,
    ) -> Result<DisplayedItemRef, crate::item_list::ItemEditError> {
        let target_position = target_position
            .or_else(|| {
                self.items
                    .insert_position(self.view.focused_index(self.items))
            })
            .unwrap_or_else(|| self.items.end_insert_position());

        let item_ref = self.items.insert_item(new_item, target_position)?;

        if move_focus && self.view.focused_item.is_some() {
            self.view.focused_item = Some(item_ref);
        }
        self.items.items_tree.xselect_all_visible(false);
        self.view.invalidate_draw_cache();
        for peer in &self.peers {
            peer.invalidate_draw_cache();
        }
        Ok(item_ref)
    }

    pub fn remove_placeholders(&mut self) {
        let removed_refs = self.items.items_tree.drain_recursive_if(|node| {
            matches!(
                self.items.displayed_items.get(&node.item_ref),
                Some(DisplayedItem::Placeholder(_))
            )
        });
        for removed_ref in removed_refs {
            self.items.displayed_items.remove(&removed_ref);
        }
    }
}

impl WaveData {
    /// Returns the maximum timestamp in the current waves.
    ///
    /// For now, this adjusts the maximum timestamp as returned by wave
    /// sources if it has 0 time. This is done to avoid having
    /// to consider what happens with the viewport.
    #[must_use]
    pub fn max_timestamp(&self) -> Option<BigInt> {
        self.inner
            .max_timestamp()
            .filter(|r| !r.is_zero())
            .and_then(|r| r.to_bigint())
    }

    /// Returns the maximum timestamp in the current waves.
    ///
    /// This is like `max_timestamp` but will always return at least 1.
    #[must_use]
    pub fn safe_max_timestamp(&self) -> BigInt {
        self.max_timestamp().unwrap_or_else(BigInt::one)
    }

    /// Returns the cached time range (start offset and end timestamp).
    #[must_use]
    pub(crate) fn time_range(&self) -> &TimeRange {
        &self.cached_time_range
    }

    /// Updates the cached time range based on current config
    pub fn refresh_time_range(&mut self, enable_time_offset: bool) {
        let start = if enable_time_offset {
            self.inner
                .min_timestamp()
                .map_or_else(BigInt::zero, |ts| ts.to_bigint().unwrap())
        } else {
            BigInt::zero()
        };
        self.cached_time_range = TimeRange {
            start,
            end: self.safe_max_timestamp(),
        };
    }

    /// Spawn async worker to build analog cache.
    ///
    /// Worker holds Arc clone.
    pub fn build_analog_cache_async(
        &self,
        entry: std::sync::Arc<crate::analog_signal_cache::AnalogCacheEntry>,
        variable_ref: &VariableRef,
        translator: crate::translation::AnyTranslator,
        sender: &std::sync::mpsc::Sender<crate::message::Message>,
    ) -> Option<()> {
        let wave_container = self.inner.as_waves()?;
        let meta = wave_container.variable_meta(variable_ref).ok()?.clone();

        let max_timestamp = self.max_timestamp()?.to_u64()?;

        let accessor = wave_container.signal_accessor(entry.cache_key.0).ok()?;

        let sender_clone = sender.clone();
        crate::async_util::perform_work(move || {
            let result = crate::analog_signal_cache::AnalogSignalCache::build(
                accessor,
                &translator,
                &meta,
                max_timestamp,
                None,
            );

            let msg = match result {
                Some(cache) => crate::message::Message::AnalogCacheBuilt {
                    entry: entry.clone(),
                    result: Ok(cache),
                },
                None => crate::message::Message::AnalogCacheBuilt {
                    entry: entry.clone(),
                    result: Err("Failed to build analog cache".into()),
                },
            };

            crate::OUTSTANDING_TRANSACTIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = sender_clone.send(msg);

            if let Some(ctx) = crate::EGUI_CONTEXT.read().unwrap().as_ref() {
                ctx.request_repaint();
            }
        });

        Some(())
    }

    pub fn set_active_scope(&mut self, scope: Option<ScopeType>) -> Option<()> {
        if let Some(scope) = scope {
            let scope = if let ScopeType::StreamScope(StreamScopeRef::Empty(name)) = scope {
                let inner = self.inner.as_transactions()?;
                ScopeType::StreamScope(StreamScopeRef::new_stream_from_name(inner, name))
            } else {
                scope
            };

            if self.inner.scope_exists(&scope) {
                self.active_scope = Some(scope);
            } else {
                warn!("Setting active scope to {scope} which does not exist");
            }
        } else {
            // Set to top-level scope
            self.active_scope = None;
        }
        Some(())
    }

    /// The window/tab title to show when this waveform is loaded, e.g. "foo.vcd - Surfer".
    #[must_use]
    pub(crate) fn window_title(&self) -> String {
        let name = self.source.title_name().or_else(|| {
            let scope_names = self
                .inner
                .root_scopes()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!scope_names.is_empty()).then(|| scope_names.join(", "))
        });
        crate::wave_source::format_window_title(name.as_deref())
    }

    #[inline]
    #[must_use]
    pub fn numbered_marker_time(&self, idx: u8) -> &BigInt {
        self.markers.get(&idx).unwrap()
    }

    #[inline]
    #[must_use]
    pub fn numbered_marker_location(&self, idx: u8, viewport: &Viewport, view_width: f32) -> f32 {
        let range = self.time_range();
        viewport.pixel_from_time(self.numbered_marker_time(idx), view_width, range)
    }

    #[must_use]
    pub fn viewport_all(&self) -> Viewport {
        Viewport::new()
    }
}

impl crate::item_list::ItemList {
    #[must_use]
    pub fn variable_translator<'a>(
        &self,
        document: &WaveData,
        field: &DisplayedFieldRef,
        translators: &'a TranslatorList,
    ) -> &'a DynTranslator {
        let Some(DisplayedItem::Variable(displayed_variable)) =
            self.displayed_items.get(&field.item)
        else {
            panic!("asking for translator for a non DisplayItem::Variable item")
        };

        variable_translator(
            displayed_variable.get_format(&field.field),
            &field.field,
            translators,
            || {
                document
                    .inner
                    .as_waves()
                    .unwrap()
                    .variable_meta(&displayed_variable.variable_ref)
            },
        )
    }

    #[must_use]
    pub fn variable_translator_with_meta<'a>(
        &self,
        field: &DisplayedFieldRef,
        translators: &'a TranslatorList,
        meta: &VariableMeta,
    ) -> &'a DynTranslator {
        let Some(DisplayedItem::Variable(displayed_variable)) =
            self.displayed_items.get(&field.item)
        else {
            panic!("asking for translator for a non DisplayItem::Variable item")
        };

        variable_translator(
            displayed_variable.get_format(&field.field),
            &field.field,
            translators,
            || Ok(meta.clone()),
        )
    }
}

impl crate::tile_kinds::waveform::WaveformView {
    pub fn index_for_ref_or_focus(
        &self,
        items: &crate::item_list::ItemList,
        item_ref: Option<DisplayedItemRef>,
    ) -> Option<ItemIndex> {
        if let Some(item_ref) = item_ref {
            items
                .items_tree
                .iter()
                .enumerate()
                .find_map(|(idx, node)| (node.item_ref == item_ref).then_some(ItemIndex(idx)))
        } else if let Some(focused_item) = self.focused_index(items) {
            items
                .items_tree
                .get_visible_extra(focused_item)
                .map(|info| info.idx)
        } else {
            None
        }
    }
    pub fn cursor_at_transition(
        &self,
        document: &WaveData,
        items: &crate::item_list::ItemList,
        cursor: Option<&BigInt>,
        next: bool,
        variable: Option<VisibleItemIndex>,
        skip_zero: bool,
    ) -> Option<BigInt> {
        let mut position = cursor.cloned();
        if let Some(vidx) = variable.or(self.focused_index(items))
            && let Some(cursor) = &position
            && let Some(DisplayedItem::Variable(variable)) = &items
                .items_tree
                .get_visible(vidx)
                .and_then(|node| items.displayed_items.get(&node.item_ref))
            && let Some(wave_container) = document.inner.as_waves()
            && let Ok(Some(res)) = wave_container.query_variable(
                &variable.variable_ref,
                &cursor.to_biguint().unwrap_or_default(),
            )
        {
            if next {
                if let Some(ref time) = res.next {
                    let stime = time.to_bigint();
                    if stime.is_some() {
                        position.clone_from(&stime);
                    }
                } else {
                    // No next transition, go to end
                    if let Some(end_time) = document.max_timestamp() {
                        position = Some(end_time);
                    } else {
                        warn!(
                            "Set cursor at transition: No timestamp count even though waveforms should be loaded"
                        );
                    }
                }
            } else if let Some(stime) = res.current.unwrap().0.to_bigint() {
                let bigone = BigInt::from(1);
                // Check if we are on a transition
                if stime == *cursor && *cursor >= bigone {
                    // If so, subtract cursor position by one
                    if let Ok(Some(newres)) = document.inner.as_waves().unwrap().query_variable(
                        &variable.variable_ref,
                        &(cursor - bigone).to_biguint().unwrap_or_default(),
                    ) && let Some(current) = newres.current
                    {
                        let newstime = current.0.to_bigint();
                        if newstime.is_some() {
                            position.clone_from(&newstime);
                        }
                    }
                } else {
                    position = Some(stime);
                }
            }

            // if zero edges should be skipped
            if skip_zero {
                // check if the next transition is 0, if so and requested, go to
                // next positive transition
                if let Some(time) = &position {
                    let next_value = document.inner.as_waves().unwrap().query_variable(
                        &variable.variable_ref,
                        &time.to_biguint().unwrap_or_default(),
                    );
                    if next_value.is_ok_and(|r| {
                        r.is_some_and(|r| {
                            r.current.is_some_and(|v| match v.1 {
                                VariableValue::BigUint(v) => v.is_zero(),
                                VariableValue::String(_) => false,
                            })
                        })
                    }) {
                        position = self.cursor_at_transition(
                            document,
                            items,
                            position.as_ref(),
                            next,
                            Some(vidx),
                            false,
                        );
                    }
                }
            }
        }
        position
    }
}

impl WaveformEdit<'_> {
    pub fn index_for_ref_or_focus(&self, item_ref: Option<DisplayedItemRef>) -> Option<ItemIndex> {
        self.view.index_for_ref_or_focus(self.items, item_ref)
    }
    pub fn cursor_at_transition(
        &self,
        cursor: Option<&BigInt>,
        next: bool,
        variable: Option<VisibleItemIndex>,
        skip_zero: bool,
    ) -> Option<BigInt> {
        self.view
            .cursor_at_transition(self.document, self.items, cursor, next, variable, skip_zero)
    }
    pub fn go_to_cursor_if_not_in_view(&mut self) -> bool {
        if let Some(cursor) = self.document.cursor.clone() {
            let range = self.time_range().clone();
            self.view
                .viewport
                .go_to_cursor_if_not_in_view(&cursor, &range)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_drawing_info::{DividerDrawingInfo, ItemDrawingInfo};

    fn items_with_rows() -> crate::item_list::ItemList {
        crate::item_list::ItemList {
            layout_cache: std::cell::RefCell::new(crate::item_list::ItemLayoutCache {
                infos: vec![
                    ItemDrawingInfo::Divider(DividerDrawingInfo {
                        vidx: VisibleItemIndex(0),
                        top: 120.0,
                        bottom: 140.0,
                    }),
                    ItemDrawingInfo::Divider(DividerDrawingInfo {
                        vidx: VisibleItemIndex(1),
                        top: 140.0,
                        bottom: 160.0,
                    }),
                ],
                signature: None,
                total_height: 40.0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn get_item_at_y_finds_correct_item() {
        let items = items_with_rows();

        assert_eq!(items.get_item_at_y(125.0), Some(VisibleItemIndex(0)));
        assert_eq!(items.get_item_at_y(145.0), Some(VisibleItemIndex(1)));
        assert_eq!(items.get_item_at_y(165.0), None);
    }

    #[test]
    fn get_item_at_y_is_not_shifted_by_scroll_offset() {
        let items = items_with_rows();
        let view = crate::tile_kinds::waveform::WaveformView {
            scroll_offset: 80.0,
            ..crate::viewport::Viewport::new().into()
        };
        assert_eq!(view.get_top_item(&items), 0);
        assert_eq!(items.get_item_at_y(125.0), Some(VisibleItemIndex(0)));
        assert_eq!(items.get_item_at_y(145.0), Some(VisibleItemIndex(1)));
    }

    #[test]
    fn visible_drawing_infos_returns_overlapping_rows() {
        let items = items_with_rows();

        let visible = items
            .visible_drawing_infos(140.0, 150.0)
            .iter()
            .map(ItemDrawingInfo::vidx)
            .collect::<Vec<_>>();

        assert_eq!(visible, vec![VisibleItemIndex(0), VisibleItemIndex(1)]);
    }

    #[test]
    fn visible_drawing_infos_includes_boundary_rows() {
        let items = items_with_rows();

        let visible = items
            .visible_drawing_infos(140.0, 140.0)
            .iter()
            .map(ItemDrawingInfo::vidx)
            .collect::<Vec<_>>();

        assert_eq!(visible, vec![VisibleItemIndex(0), VisibleItemIndex(1)]);
    }
}
