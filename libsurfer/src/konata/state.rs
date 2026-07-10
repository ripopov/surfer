use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use ftr_parser::types::{GeneratorId, StreamId};
use serde::{Deserialize, Serialize};

use crate::{source::SourceId, transaction_container::TransactionStreamRef};

use super::{KonataModel, KonataRowSet, KonataSearchHits, KonataViewport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KonataTileId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KonataModelSpec {
    #[serde(default)]
    pub source: SourceId,
    pub generator: TransactionStreamRef,
}

impl KonataModelSpec {
    #[must_use]
    pub fn references_source(&self, source: SourceId) -> bool {
        self.source == source
    }

    pub(crate) fn remap_sources(&mut self, source_map: &HashMap<SourceId, SourceId>) {
        if let Some(source) = source_map.get(&self.source) {
            self.source = *source;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KonataLaneMode {
    #[default]
    Merged,
    SplitFixed,
    SplitNatural,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KonataColorScheme {
    #[default]
    Auto,
    Unique,
    Thread,
    Orange,
    RoyalBlue,
    ColorBlindSafe,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KonataArrowStyle {
    #[default]
    Inside,
    LeftCurve,
    Hidden,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KonataAlignmentMode {
    #[default]
    ThreadRid,
    FetchId,
    Timestamp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KonataInstructionClassifier {
    #[default]
    Generic,
    X86Gem5,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KonataHslComponent {
    Value(f32),
    Auto(KonataAutoKeyword),
}

impl Default for KonataHslComponent {
    fn default() -> Self {
        Self::Auto(KonataAutoKeyword::Auto)
    }
}

impl KonataHslComponent {
    #[must_use]
    pub fn resolve(self, automatic: f32) -> f32 {
        match self {
            Self::Value(value) if value.is_finite() => value,
            Self::Value(_) | Self::Auto(_) => automatic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KonataAutoKeyword {
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KonataHslColor {
    pub hue: KonataHslComponent,
    pub saturation: KonataHslComponent,
    pub lightness: KonataHslComponent,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KonataCustomColorScheme {
    pub default: KonataHslColor,
    pub stages: BTreeMap<String, KonataHslColor>,
    pub lanes: BTreeMap<String, KonataHslColor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KonataViewConfig {
    pub splitter_px: f32,
    pub hide_flushed: bool,
    pub lane_mode: KonataLaneMode,
    pub color_scheme: KonataColorScheme,
    pub arrow_style: KonataArrowStyle,
    pub show_minimap: bool,
    pub synchronize_scroll: bool,
    /// Synchronization group. Legacy states with only `synchronize_scroll`
    /// participate in group zero.
    pub sync_group: Option<u64>,
    pub alignment_mode: KonataAlignmentMode,
    pub overlay: Option<KonataModelSpec>,
    /// Identifies the selected tile when several tiles share one model spec.
    /// `overlay` remains the durable source fallback for older state files.
    pub overlay_tile: Option<KonataTileId>,
    pub text_lod_px: f32,
    pub frame_lod_px: f32,
    pub color_lod_px: f32,
    pub arrow_lod_px: f32,
    pub zoom_step: f64,
    pub execution_stages: String,
    pub stall_stages: String,
    pub stall_case_sensitive: bool,
    pub instruction_classifier: KonataInstructionClassifier,
    pub include_estimated_flush_rates: bool,
    pub custom_stage_colors: BTreeMap<String, [u8; 3]>,
    pub custom_color_schemes: BTreeMap<String, KonataCustomColorScheme>,
    pub custom_color_scheme: String,
    pub custom_color_name: String,
    pub custom_color_rgb: [u8; 3],
    pub options_filter: String,
    /// User-provided pipeline clock in trace ticks. `None` keeps the view in
    /// trace-time mode.
    pub clock_period_ticks: Option<u64>,
    pub clock_origin_tick: i64,
    pub ruler_cycles: bool,
}

impl Default for KonataViewConfig {
    fn default() -> Self {
        Self {
            splitter_px: 310.0,
            hide_flushed: false,
            lane_mode: KonataLaneMode::Merged,
            color_scheme: KonataColorScheme::Auto,
            arrow_style: KonataArrowStyle::Inside,
            show_minimap: false,
            synchronize_scroll: false,
            sync_group: None,
            alignment_mode: KonataAlignmentMode::ThreadRid,
            overlay: None,
            overlay_tile: None,
            text_lod_px: 10.0,
            frame_lod_px: 4.0,
            color_lod_px: 1.0,
            arrow_lod_px: 4.0,
            zoom_step: 2.0_f64.sqrt(),
            execution_stages: "X,x,Ex,execute".to_string(),
            stall_stages: "f,stl".to_string(),
            stall_case_sensitive: true,
            instruction_classifier: KonataInstructionClassifier::Generic,
            include_estimated_flush_rates: false,
            custom_stage_colors: BTreeMap::new(),
            custom_color_schemes: BTreeMap::new(),
            custom_color_scheme: String::new(),
            custom_color_name: String::new(),
            custom_color_rgb: [90, 160, 220],
            options_filter: String::new(),
            clock_period_ticks: None,
            clock_origin_tick: 0,
            ruler_cycles: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KonataTileState {
    pub spec: KonataModelSpec,
    pub title: String,
    #[serde(default)]
    pub config: KonataViewConfig,
    #[serde(default)]
    pub viewport: KonataViewport,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KonataBookmark {
    pub tx_id: u64,
    pub tick: i64,
    pub px_per_tick: f64,
    pub row_height_px: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KonataModelKey {
    pub source: SourceId,
    pub stream: StreamId,
    pub parent_generator: GeneratorId,
    pub event_generator: GeneratorId,
    pub generation: u64,
}

#[derive(Debug)]
pub struct KonataModelEntry {
    pub key: KonataModelKey,
    result: RwLock<Option<Result<Arc<KonataModel>, Arc<str>>>>,
    complete: AtomicBool,
    revision: AtomicU64,
}

impl KonataModelEntry {
    #[must_use]
    pub fn new(key: KonataModelKey) -> Self {
        Self {
            key,
            result: RwLock::new(None),
            complete: AtomicBool::new(false),
            revision: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.is_ready()
    }

    #[must_use]
    pub fn model(&self) -> Option<Arc<KonataModel>> {
        self.result
            .read()
            .ok()
            .and_then(|result| result.as_ref()?.as_ref().ok().cloned())
    }

    #[must_use]
    pub fn error(&self) -> Option<Arc<str>> {
        self.result
            .read()
            .ok()
            .and_then(|result| result.as_ref()?.as_ref().err().cloned())
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn complete(&self, result: Result<Arc<KonataModel>, Arc<str>>) {
        if let Ok(mut slot) = self.result.write() {
            *slot = Some(result);
            self.revision.fetch_add(1, Ordering::AcqRel);
            self.complete.store(true, Ordering::Release);
        }
    }

    pub fn publish(&self, model: Arc<KonataModel>) {
        if !self.is_complete()
            && let Ok(mut slot) = self.result.write()
        {
            *slot = Some(Ok(model));
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }
}

#[derive(Debug)]
pub struct KonataRuntimeState {
    pub entry: Option<Arc<KonataModelEntry>>,
    pub requested_generation: Option<u64>,
    pub model_progress: f32,
    pub model_phase: String,
    pub cancel_token: Arc<AtomicBool>,
    pub revision: u64,
    pub hover_row: Option<usize>,
    pub hover_stage: Option<usize>,
    pub keyboard_row: Option<usize>,
    pub keyboard_stage_event: Option<u64>,
    pub keyboard_region_focused: bool,
    pub last_focused_tx: Option<u64>,
    pub suppress_focus_scroll: Option<u64>,
    pub canvas_size: egui::Vec2,
    pub viewport_motion: Option<KonataViewportMotion>,
    pub range_selection: Option<(u64, u64)>,
    pub range_drag_start: Option<u64>,
    pub reload_anchor: Option<KonataReloadAnchor>,
    pub producer_chain: Option<Arc<KonataRowSet>>,
    pub producer_chain_root: Option<usize>,
    pub producer_chain_revision: u64,
    pub producer_chain_cancel: Arc<AtomicBool>,
    pub pinned_tooltip: Option<(u64, Option<u64>)>,
    pub overlap_click_key: Option<(usize, i64)>,
    pub overlap_click_index: usize,
    pub dependency_walk_origin: Option<usize>,
    pub dependency_walk_target: Option<usize>,
    pub dependency_walk_forward: bool,
    pub dependency_walk_index: usize,
    pub overlay_emphasis: Option<KonataTileId>,
    pub find_open: bool,
    pub find_focus_requested: bool,
    pub find_query: String,
    pub find_valid_pattern: Option<String>,
    pub find_revision: u64,
    pub find_cancel: Arc<AtomicBool>,
    pub find_hits: Option<Arc<KonataSearchHits>>,
    pub find_error: Option<String>,
    pub find_active_row: Option<usize>,
    pub find_anchor_row: usize,
    pub find_searching: bool,
    pub find_partial_first: Option<usize>,
    pub find_partial_count: usize,
    pub find_processed: Arc<AtomicUsize>,
    pub find_total: usize,
}

impl Default for KonataRuntimeState {
    fn default() -> Self {
        Self {
            entry: None,
            requested_generation: None,
            model_progress: 0.0,
            model_phase: "Preparing pipeline projection".to_string(),
            cancel_token: Arc::new(AtomicBool::new(false)),
            revision: 0,
            hover_row: None,
            hover_stage: None,
            keyboard_row: None,
            keyboard_stage_event: None,
            keyboard_region_focused: false,
            last_focused_tx: None,
            suppress_focus_scroll: None,
            canvas_size: egui::Vec2::ZERO,
            viewport_motion: None,
            range_selection: None,
            range_drag_start: None,
            reload_anchor: None,
            producer_chain: None,
            producer_chain_root: None,
            producer_chain_revision: 0,
            producer_chain_cancel: Arc::new(false.into()),
            pinned_tooltip: None,
            overlap_click_key: None,
            overlap_click_index: 0,
            dependency_walk_origin: None,
            dependency_walk_target: None,
            dependency_walk_forward: false,
            dependency_walk_index: 0,
            overlay_emphasis: None,
            find_open: false,
            find_focus_requested: false,
            find_query: String::new(),
            find_valid_pattern: None,
            find_revision: 0,
            find_cancel: Arc::new(AtomicBool::new(false)),
            find_hits: None,
            find_error: None,
            find_active_row: None,
            find_anchor_row: 0,
            find_searching: false,
            find_partial_first: None,
            find_partial_count: 0,
            find_processed: Arc::new(AtomicUsize::new(0)),
            find_total: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KonataViewportMotion {
    pub start: KonataViewport,
    pub target: KonataViewport,
    pub elapsed: f32,
    pub duration: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct KonataReloadAnchor {
    pub tx_id: u64,
    pub fallback_tick: u64,
    pub left_offset: f64,
    pub row_offset: f64,
    pub px_per_tick: f64,
    pub row_height_px: f64,
}
