use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem,
    path::PathBuf,
};

use crate::{
    CanvasState, StartupParams,
    annotation_list::{AnnotationGroup, DEFAULT_GROUP_NAME},
    clock_highlighting::ClockHighlightType,
    config::{ArrowKeyBindings, AutoLoad, PrimaryMouseDrag, SurferConfig, TransitionValue},
    data_container::DataContainer,
    dialog::{OpenSiblingStateFileDialog, ReloadWaveformDialog, SignalAnalysisWizardDialog},
    displayed_item_tree::{DisplayedItemTree, VisibleItemIndex},
    frame_buffer::FrameBufferSettings,
    hierarchy::{HierarchyStyle, ParameterDisplayLocation},
    message::Message,
    source::{SourceId, SourceStore, format_time_domain},
    system_state::SystemState,
    table::{TableTileId, TableTileState},
    tiles::SurferTileTree,
    time::{TimeStringFormatting, TimeUnit},
    trace_style::TraceStyle,
    transaction_container::TransactionContainer,
    variable_filter::VariableFilter,
    viewport::Viewport,
    wave_container::{ScopeRef, VariableRef, WaveContainer},
    wave_data::WaveData,
    wave_source::{LoadIntent, LoadOptions, WaveFormat, WaveSource},
};
use egui::{
    Visuals,
    style::{Selection, WidgetVisuals, Widgets},
};
use epaint::{CornerRadius, Stroke};
use eyre::{Result, WrapErr as _};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use surfer_translation_types::{Translator, VariableType};
use surver::SurverFileInfo;
use tracing::{error, info, trace, warn};

#[derive(Clone, Copy)]
enum ScopeVariableSelection {
    All,
    VcdEventOnly,
}

/// The parts of the program state that need to be serialized when loading/saving state
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct UserState {
    #[serde(skip)]
    pub config: SurferConfig,

    /// Overrides for the config show_* fields. Defaults to `config.show_*` if not present
    pub(crate) show_hierarchy: Option<bool>,
    pub(crate) show_menu: Option<bool>,
    pub(crate) show_ticks: Option<bool>,
    pub(crate) show_toolbar: Option<bool>,
    pub(crate) show_tooltip: Option<bool>,
    pub(crate) show_scope_tooltip: Option<bool>,
    pub(crate) show_default_timeline: Option<bool>,
    pub(crate) show_overview: Option<bool>,
    pub(crate) show_statusbar: Option<bool>,
    pub(crate) align_names_right: Option<bool>,
    pub(crate) show_variable_indices: Option<bool>,
    pub(crate) show_variable_direction: Option<bool>,
    pub(crate) show_empty_scopes: Option<bool>,
    pub(crate) show_hierarchy_icons: Option<bool>,
    pub(crate) show_parameters_in_scopes: Option<bool>,
    #[serde(default)]
    pub(crate) parameter_display_location: Option<ParameterDisplayLocation>,
    #[serde(default)]
    pub(crate) highlight_focused: Option<bool>,
    #[serde(default)]
    pub(crate) fill_high_values: Option<bool>,
    #[serde(default)]
    pub(crate) primary_button_drag_behavior: Option<PrimaryMouseDrag>,
    #[serde(default)]
    pub(crate) arrow_key_bindings: Option<ArrowKeyBindings>,
    #[serde(default)]
    pub(crate) clock_highlight_type: Option<ClockHighlightType>,
    #[serde(default)]
    pub(crate) hierarchy_style: Option<HierarchyStyle>,
    #[serde(default)]
    pub(crate) autoload_sibling_state_files: Option<AutoLoad>,
    #[serde(default)]
    pub(crate) autoreload_files: Option<AutoLoad>,

    pub(crate) waves: Option<WaveData>,
    pub(crate) drag_started: bool,
    pub(crate) drag_source_idx: Option<VisibleItemIndex>,
    pub(crate) drag_target_idx: Option<crate::displayed_item_tree::TargetPosition>,

    pub(crate) previous_waves: Option<WaveData>,
    #[serde(skip, default)]
    pub(crate) pending_state_restore: Option<PendingStateRestore>,

    /// Count argument for movements
    pub(crate) count: Option<String>,

    // Vector of translators which have failed at the `translates` function for a variable.
    pub(crate) blacklisted_translators: HashSet<(VariableRef, String)>,

    pub(crate) show_about: bool,
    pub(crate) show_keys: bool,
    pub(crate) show_gestures: bool,
    pub(crate) show_quick_start: bool,
    pub(crate) show_license: bool,
    pub(crate) show_performance: bool,
    pub(crate) show_logs: bool,
    pub(crate) show_cursor_window: bool,
    #[serde(default)]
    pub(crate) frame_buffer: FrameBufferSettings,
    pub(crate) wanted_timeunit: TimeUnit,
    pub(crate) time_string_format: Option<TimeStringFormatting>,
    pub(crate) show_url_entry: bool,
    /// Show a confirmation dialog asking the user for confirmation
    /// that surfer should reload changed files from disk.
    #[serde(skip, default)]
    pub(crate) show_reload_suggestion: Option<ReloadWaveformDialog>,
    #[serde(skip, default)]
    pub(crate) show_open_sibling_state_file_suggestion: Option<OpenSiblingStateFileDialog>,
    #[serde(skip, default)]
    pub(crate) show_signal_analysis_wizard: Option<SignalAnalysisWizardDialog>,
    #[serde(skip, default)]
    pub(crate) signal_analysis_wizard_edit_target: Option<TableTileId>,
    pub(crate) variable_name_filter_focused: bool,
    pub(crate) variable_filter: VariableFilter,
    //Sidepanel width
    pub(crate) sidepanel_width: Option<f32>,
    /// UI zoom factor if set by the user
    pub(crate) ui_zoom_factor: Option<f32>,
    #[serde(default)]
    pub(crate) animation_enabled: Option<bool>,
    #[serde(skip, default)]
    pub(crate) use_dinotrace_style: Option<bool>,
    #[serde(default)]
    pub(crate) trace_style: Option<TraceStyle>,
    #[serde(skip, default)]
    pub(crate) show_server_file_window: bool,
    #[serde(skip, default)]
    pub(crate) selected_server_file_index: Option<usize>,
    #[serde(skip, default)]
    pub(crate) surver_file_infos: Option<Vec<SurverFileInfo>>,
    #[serde(skip, default)]
    pub(crate) surver_url: Option<String>,
    #[serde(default)]
    pub(crate) transition_value: Option<TransitionValue>,
    #[serde(default)]
    pub(crate) toolbar_group_enabled: HashMap<String, Option<bool>>,
    #[serde(default)]
    pub(crate) toolbar_group_rows: Vec<Vec<String>>,
    #[serde(default)]
    pub(crate) tile_tree: SurferTileTree,
    #[serde(default)]
    pub(crate) table_tiles: HashMap<TableTileId, TableTileState>,
    /// Show raw `.events` generators in the hierarchy sidebar instead of
    /// folding them into their parent generator's presentation
    #[serde(default)]
    pub(crate) show_raw_event_generators: bool,

    // Path of last saved-to state file
    // Do not serialize as this causes a few issues and doesn't help:
    // - We need to set it on load of a state anyways since the file could have been renamed
    // - Bad interoperatility story between native and wasm builds
    // - Sequencing issue in serialization, due to us having to run that async
    #[serde(skip)]
    pub state_file: Option<PathBuf>,

    pub(crate) show_annotation_list: bool,
}

pub(crate) struct PendingStateRestore {
    waves: WaveData,
    sources: Vec<(SourceId, WaveSource)>,
}

// Impl needed since for loading we need to put State into a Message
// Snip out the actual contents to not completely spam the terminal
impl std::fmt::Debug for UserState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedState {{ <snipped> }}")
    }
}

impl UserState {
    pub fn new(force_default_config: bool) -> Result<UserState> {
        let config = SurferConfig::new(force_default_config)
            .with_context(|| "Failed to load config file")?;

        Ok(UserState {
            config,
            ..Default::default()
        })
    }
}

impl Default for UserState {
    fn default() -> Self {
        Self {
            config: SurferConfig::default(),
            show_hierarchy: None,
            show_menu: None,
            show_ticks: None,
            show_toolbar: None,
            show_tooltip: None,
            show_scope_tooltip: None,
            show_default_timeline: None,
            show_overview: None,
            show_statusbar: None,
            align_names_right: None,
            show_variable_indices: None,
            show_variable_direction: None,
            show_empty_scopes: None,
            show_hierarchy_icons: None,
            show_parameters_in_scopes: None,
            parameter_display_location: None,
            highlight_focused: None,
            fill_high_values: None,
            primary_button_drag_behavior: None,
            arrow_key_bindings: None,
            clock_highlight_type: None,
            hierarchy_style: None,
            autoload_sibling_state_files: None,
            autoreload_files: None,
            waves: None,
            drag_started: false,
            drag_source_idx: None,
            drag_target_idx: None,
            previous_waves: None,
            pending_state_restore: None,
            count: None,
            blacklisted_translators: HashSet::new(),
            show_about: false,
            show_keys: false,
            show_gestures: false,
            show_quick_start: false,
            show_license: false,
            show_performance: false,
            show_logs: false,
            show_cursor_window: false,
            frame_buffer: FrameBufferSettings::default(),
            wanted_timeunit: TimeUnit::None,
            time_string_format: None,
            show_url_entry: false,
            show_reload_suggestion: None,
            show_open_sibling_state_file_suggestion: None,
            show_signal_analysis_wizard: None,
            signal_analysis_wizard_edit_target: None,
            variable_name_filter_focused: false,
            variable_filter: VariableFilter::new(),
            sidepanel_width: None,
            ui_zoom_factor: None,
            state_file: None,
            animation_enabled: None,
            use_dinotrace_style: None,
            trace_style: None,
            selected_server_file_index: None,
            show_server_file_window: false,
            surver_file_infos: None,
            surver_url: None,
            transition_value: None,
            show_annotation_list: false,
            toolbar_group_enabled: HashMap::new(),
            toolbar_group_rows: Vec::new(),
            tile_tree: SurferTileTree::default(),
            table_tiles: HashMap::new(),
            show_raw_event_generators: false,
        }
    }
}

impl SystemState {
    pub fn with_params(mut self, args: StartupParams) -> Self {
        self.user.previous_waves = self.user.waves;
        self.user.waves = None;

        // we turn the waveform argument and any startup command file into batch commands
        self.batch_messages = VecDeque::new();

        for (idx, source) in args
            .waves
            .into_iter()
            .chain(args.additional_waves)
            .enumerate()
        {
            if idx == 0 {
                match source {
                    WaveSource::Url(url) => {
                        self.add_batch_message(Message::LoadUrlWithIntent(
                            url,
                            LoadIntent::ReplaceSession,
                        ));
                    }
                    WaveSource::File(file) => {
                        self.add_batch_message(Message::LoadFileWithIntent(
                            file,
                            crate::wave_source::LoadIntent::ReplaceSession,
                        ));
                    }
                    WaveSource::Data => error!("Attempted to load data at startup"),
                    WaveSource::Cxxrtl(url) => {
                        self.add_batch_message(Message::SetupCxxrtl(url));
                    }
                    WaveSource::DragAndDrop(_) => {
                        error!("Attempted to load from drag and drop at startup (how?)");
                    }
                }
            } else {
                match source {
                    WaveSource::File(file) => {
                        self.add_batch_message(Message::LoadFileWithIntent(
                            file,
                            crate::wave_source::LoadIntent::AddSource,
                        ));
                    }
                    WaveSource::Url(url) => {
                        self.add_batch_message(Message::LoadUrlWithIntent(
                            url,
                            LoadIntent::AddSource,
                        ));
                    }
                    WaveSource::Data => error!("Attempted to add data at startup"),
                    WaveSource::Cxxrtl(_) => error!("Attempted to add CXXRTL source at startup"),
                    WaveSource::DragAndDrop(_) => {
                        error!("Attempted to add drag-and-drop source at startup")
                    }
                }
            }
        }

        if let Some(port) = args.wcp_initiate {
            let addr = format!("127.0.0.1:{port}");
            self.add_batch_message(Message::StartWcpServer {
                address: Some(addr),
                initiate: true,
            });
        }

        self.add_batch_commands(args.startup_commands);

        self
    }

    pub fn wcp(&mut self) {
        self.handle_wcp_commands();
    }

    pub(crate) fn get_scope(&mut self, scope: &ScopeRef, recursive: bool) -> Vec<VariableRef> {
        self.get_scope_from_source(WaveData::primary_source_id(), scope.clone(), recursive)
    }

    pub(crate) fn get_scope_from_source(
        &mut self,
        source: SourceId,
        scope: ScopeRef,
        recursive: bool,
    ) -> Vec<VariableRef> {
        self.collect_scope_variables(source, &scope, recursive, ScopeVariableSelection::All)
    }

    pub(crate) fn get_scope_vcd_events_from_source(
        &mut self,
        source: SourceId,
        scope: ScopeRef,
        recursive: bool,
    ) -> Vec<VariableRef> {
        self.collect_scope_variables(
            source,
            &scope,
            recursive,
            ScopeVariableSelection::VcdEventOnly,
        )
    }

    fn collect_scope_variables(
        &mut self,
        source: SourceId,
        scope: &ScopeRef,
        recursive: bool,
        selection: ScopeVariableSelection,
    ) -> Vec<VariableRef> {
        let Some(waves) = self.user.waves.as_ref() else {
            return vec![];
        };

        let Some(wave_cont) = waves.waves_for_source(source) else {
            warn!("Cannot collect variables from {source}: waveform source not found");
            return vec![];
        };

        let children = wave_cont.child_scopes(scope);
        let mut variables = wave_cont
            .variables_in_scope(scope)
            .iter()
            .sorted_by(|a, b| numeric_sort::cmp(&a.name, &b.name))
            .filter_map(|var| match selection {
                ScopeVariableSelection::All => Some(var.clone()),
                ScopeVariableSelection::VcdEventOnly => match wave_cont.variable_meta(var) {
                    Ok(meta) => {
                        (meta.variable_type == Some(VariableType::VCDEvent)).then_some(var.clone())
                    }
                    Err(error) => {
                        warn!(
                            "Failed metadata lookup for variable {:?} in scope {scope}: {error:#}",
                            var
                        );
                        None
                    }
                },
            })
            .collect_vec();

        if recursive && let Ok(children) = children {
            for child in children {
                variables.append(&mut self.collect_scope_variables(source, &child, true, selection));
            }
        }

        variables
    }

    pub(crate) fn on_waves_loaded(
        &mut self,
        filename: WaveSource,
        format: WaveFormat,
        new_waves: Box<WaveContainer>,
        load_options: LoadOptions,
    ) {
        let filename_for_title = filename.clone();
        info!("{format} file loaded");
        let viewport = Viewport::new();
        let viewports = [viewport].to_vec();

        for translator in self.translators.all_translators() {
            translator.set_wave_source(Some(filename.into_translation_type()));
        }

        let ((new_wave, load_commands), is_reload) =
            if load_options != LoadOptions::Clear && self.user.waves.is_some() {
                (
                    self.user.waves.take().unwrap().update_with_waves(
                        new_waves,
                        filename,
                        format,
                        &self.translators,
                        load_options == LoadOptions::KeepAll,
                    ),
                    true,
                )
            } else if let Some(old) = self.user.previous_waves.take() {
                (
                    old.update_with_waves(
                        new_waves,
                        filename,
                        format,
                        &self.translators,
                        load_options == LoadOptions::KeepAll,
                    ),
                    true,
                )
            } else {
                (
                    (
                        WaveData {
                            inner: DataContainer::Waves(*new_waves),
                            source: filename,
                            format,
                            primary_source_label: None,
                            sources: SourceStore::default(),
                            active_scope_source: SourceId::default(),
                            active_scope: None,
                            items_tree: DisplayedItemTree::default(),
                            displayed_items: HashMap::new(),
                            viewports,
                            cursor: None,
                            markers: HashMap::new(),
                            annotations: Vec::new(),
                            selected_annotation: None,
                            annotation_counter: 0,
                            last_active_viewport_idx: 0,
                            annotation_menu_pos: None,
                            annotation_menu_time: None,
                            focused_item: None,
                            focused_transaction: (None, None),
                            default_variable_name_type: self.user.config.default_variable_name_type,
                            display_variable_indices: self.show_variable_indices(),
                            scroll_offset: 0.,
                            drawing_infos: vec![],
                            top_item_draw_offset: 0.,
                            total_height: 0.,
                            display_item_ref_counter: 0,
                            old_num_timestamps: None,
                            graphics: HashMap::new(),
                            cache_generation: 0,
                            inflight_caches: HashMap::new(),
                            annotation_groups: vec![],
                            annotation_list_visible: false,
                        },
                        None,
                    ),
                    false,
                )
            };

        if let Some(cmd) = load_commands {
            self.load_variables(cmd);
        }
        self.invalidate_draw_commands();

        self.user.waves = Some(new_wave);
        // Update window title with waveform name
        let title = match &filename_for_title {
            WaveSource::File(path) => {
                if let Some(name) = path.file_name() {
                    format!("Surfer - {name}")
                } else {
                    "Surfer".to_string()
                }
            }
            WaveSource::Url(url) => format!("Surfer - {url}"),
            _ => "Surfer".to_string(),
        };
        if let Some(ctx) = self.context.as_ref() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        self.record_file_history(&filename_for_title);

        if !is_reload && let Some(waves) = &mut self.user.waves {
            // Set time unit
            self.user.wanted_timeunit = waves.inner.metadata().timescale.unit;

            let ungrouped = AnnotationGroup {
                name: String::from(DEFAULT_GROUP_NAME),
                cycle_counter: 0,
                annotations: Vec::new(),
            };

            waves.annotation_groups.push(ungrouped);

            self.user.tile_tree = SurferTileTree::default();
            // Possibly open state file load dialog
            if waves.source.sibling_state_file().is_some() {
                self.update(Message::SuggestOpenSiblingStateFile);
            }
        }
    }

    pub(crate) fn on_transaction_streams_loaded(
        &mut self,
        filename: WaveSource,
        format: WaveFormat,
        new_ftr: TransactionContainer,
        load_options: LoadOptions,
    ) {
        info!("Transaction streams are loaded.");
        self.record_file_history(&filename);

        let viewport = Viewport::new();
        let viewports = [viewport].to_vec();

        let (new_transaction_streams, is_reload) =
            if load_options != LoadOptions::Clear && self.user.waves.is_some() {
                let old = self.user.waves.take().unwrap();
                (
                    old.update_with_transactions(new_ftr, filename, format, &self.translators),
                    true,
                )
            } else if let Some(old) = self.user.previous_waves.take() {
                (
                    old.update_with_transactions(new_ftr, filename, format, &self.translators),
                    true,
                )
            } else {
                (
                    WaveData {
                        inner: DataContainer::Transactions(new_ftr),
                        source: filename,
                        format,
                        primary_source_label: None,
                        sources: SourceStore::default(),
                        active_scope_source: SourceId::default(),
                        active_scope: None,
                        items_tree: DisplayedItemTree::default(),
                        displayed_items: HashMap::new(),
                        viewports,
                        cursor: None,
                        markers: HashMap::new(),
                        annotations: Vec::new(),
                        selected_annotation: None,
                        annotation_counter: 1,
                        last_active_viewport_idx: 0,
                        annotation_menu_pos: None,
                        annotation_menu_time: None,
                        focused_item: None,
                        focused_transaction: (None, None),
                        default_variable_name_type: self.user.config.default_variable_name_type,
                        display_variable_indices: self.show_variable_indices(),
                        scroll_offset: 0.,
                        drawing_infos: vec![],
                        top_item_draw_offset: 0.,
                        total_height: 0.,
                        display_item_ref_counter: 0,
                        old_num_timestamps: None,
                        graphics: HashMap::new(),
                        cache_generation: 0,
                        inflight_caches: HashMap::new(),
                        annotation_groups: vec![],
                        annotation_list_visible: false,
                    },
                    false,
                )
            };

        self.invalidate_draw_commands();

        self.user.config.theme.alt_frequency = 0;
        self.user.wanted_timeunit = new_transaction_streams.inner.metadata().timescale.unit;
        self.user.waves = Some(new_transaction_streams);

        if !is_reload && let Some(waves) = &mut self.user.waves {
            self.user.tile_tree = SurferTileTree::default();
            if waves.source.sibling_state_file().is_some() {
                self.update(Message::SuggestOpenSiblingStateFile);
            }
        }
    }

    pub(crate) fn on_transaction_streams_loaded_with_intent(
        &mut self,
        filename: WaveSource,
        format: WaveFormat,
        new_ftr: TransactionContainer,
        intent: crate::wave_source::LoadIntent,
    ) {
        match intent {
            crate::wave_source::LoadIntent::ReplaceSession => {
                self.on_transaction_streams_loaded(filename, format, new_ftr, LoadOptions::Clear);
            }
            crate::wave_source::LoadIntent::ReloadSource {
                source,
                keep_unavailable,
            } => {
                self.record_file_history(&filename);
                let source_label = filename.to_string();
                let candidate_container = DataContainer::Transactions(new_ftr);
                let candidate_domain =
                    crate::source::TimeDomain::from_container(&candidate_container);
                let Some(waves) = self.user.waves.as_mut() else {
                    self.update(Message::Error(eyre::eyre!(
                        "Cannot reload {source_label}. No session is loaded"
                    )));
                    return;
                };
                let existing_domain = waves.sources.session_time_domain.clone();
                match waves.replace_source(
                    source,
                    filename.clone(),
                    format,
                    candidate_container,
                    keep_unavailable,
                    &self.translators,
                ) {
                    Ok(Some(cmd)) => self.load_variables_for_source(source, cmd),
                    Ok(None) => {
                        info!("Reloaded transaction source {source_label} as {source}");
                        self.user.config.theme.alt_frequency = 0;
                        self.invalidate_draw_commands();
                    }
                    Err(err) => {
                        let details = match (existing_domain, candidate_domain) {
                            (Some(existing), Some(candidate)) => format!(
                                "\nExisting session: {}\nCandidate source: {}",
                                format_time_domain(&existing),
                                format_time_domain(&candidate)
                            ),
                            _ => String::new(),
                        };
                        self.update(Message::Error(eyre::eyre!(
                            "Cannot reload {source_label}. {err}{details}"
                        )));
                    }
                }
            }
            crate::wave_source::LoadIntent::AddSource => {
                self.record_file_history(&filename);
                if self.user.waves.is_none() {
                    self.on_transaction_streams_loaded(
                        filename,
                        format,
                        new_ftr,
                        LoadOptions::Clear,
                    );
                    return;
                }

                let source_label = filename.to_string();
                let Some(waves) = self.user.waves.as_mut() else {
                    return;
                };
                let candidate_container = DataContainer::Transactions(new_ftr);
                let candidate_domain =
                    crate::source::TimeDomain::from_container(&candidate_container);
                waves.refresh_session_time_domain();
                let existing_domain = waves.sources.session_time_domain.clone();

                match waves.add_loaded_source(filename.clone(), format, candidate_container) {
                    Ok(id) => {
                        info!("Added transaction source {source_label} as {id}");
                        self.user.config.theme.alt_frequency = 0;
                        self.invalidate_draw_commands();
                    }
                    Err(err) => {
                        let details = match (existing_domain, candidate_domain) {
                            (Some(existing), Some(candidate)) => format!(
                                "\nExisting session: {}\nCandidate source: {}",
                                format_time_domain(&existing),
                                format_time_domain(&candidate)
                            ),
                            _ => String::new(),
                        };
                        self.update(Message::Error(eyre::eyre!(
                            "Cannot add {source_label}. {err}{details}"
                        )));
                    }
                }
            }
        }
    }

    pub(crate) fn record_file_history(&mut self, source: &WaveSource) {
        if let Some(path) = source.path() {
            self.file_history.add(path);
        }
    }

    #[cfg(test)]
    pub(crate) fn handle_async_messages(&mut self) {
        let mut msgs = vec![];
        loop {
            match self.channels.msg_receiver.try_recv() {
                Ok(msg) => msgs.push(msg),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    trace!("Message sender disconnected");
                    break;
                }
            }
        }

        while let Some(msg) = msgs.pop() {
            self.update(msg);
        }
    }

    pub(crate) fn push_async_messages(&mut self, msgs: &mut Vec<Message>) {
        loop {
            match self.channels.msg_receiver.try_recv() {
                Ok(msg) => msgs.push(msg),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    trace!("Message sender disconnected");
                    break;
                }
            }
        }
    }

    pub fn get_visuals(&self) -> Visuals {
        let widget_style = WidgetVisuals {
            bg_fill: self.user.config.theme.secondary_ui_color.background,
            fg_stroke: Stroke {
                color: self.user.config.theme.secondary_ui_color.foreground,
                width: 1.0,
            },
            weak_bg_fill: self.user.config.theme.secondary_ui_color.background,
            bg_stroke: Stroke {
                color: self.user.config.theme.border_color,
                width: 1.0,
            },
            corner_radius: CornerRadius::same(2),
            expansion: 0.0,
        };

        Visuals {
            override_text_color: Some(self.user.config.theme.foreground),
            extreme_bg_color: self.user.config.theme.secondary_ui_color.background,
            panel_fill: self.user.config.theme.secondary_ui_color.background,
            window_fill: self.user.config.theme.primary_ui_color.background,
            window_stroke: Stroke {
                width: 1.0,
                color: self.user.config.theme.border_color,
            },
            selection: Selection {
                bg_fill: self.user.config.theme.selected_elements_colors.background,
                stroke: Stroke {
                    color: self.user.config.theme.selected_elements_colors.foreground,
                    width: 1.0,
                },
            },
            widgets: Widgets {
                noninteractive: widget_style,
                inactive: widget_style,
                hovered: widget_style,
                active: widget_style,
                open: widget_style,
            },
            ..Visuals::dark()
        }
    }

    pub(crate) fn load_state(&mut self, mut loaded_state: Box<UserState>, path: Option<PathBuf>) {
        if self.user.waves.is_none() && loaded_state.waves.is_some() {
            if let Err(err) = self.start_pending_state_restore(loaded_state, path) {
                self.update(Message::Error(err));
            }
            return;
        }

        // first swap everything, fix special cases afterwards
        mem::swap(&mut self.user, &mut loaded_state);

        // swap back waves for inner, source, format since we want to keep the file
        // fix up all wave references from paths if a wave is loaded
        mem::swap(&mut loaded_state.waves, &mut self.user.waves);
        if let Some(new_waves) = loaded_state.waves.take() {
            let source_map = self
                .user
                .waves
                .as_ref()
                .map(|waves| Self::source_id_map_for_loaded_state(waves, &new_waves))
                .unwrap_or_default();
            self.apply_loaded_wave_state(new_waves, &source_map);
        }

        self.finish_state_load(path);
    }

    fn start_pending_state_restore(
        &mut self,
        mut loaded_state: Box<UserState>,
        path: Option<PathBuf>,
    ) -> Result<()> {
        let Some(saved_waves) = loaded_state.waves.take() else {
            return Ok(());
        };
        let sources = Self::state_source_entries(&saved_waves);
        let load_messages = Self::state_restore_load_messages(&sources)?;

        mem::swap(&mut self.user, &mut loaded_state);
        self.user.waves = None;
        self.user.pending_state_restore = Some(PendingStateRestore {
            waves: saved_waves,
            sources,
        });
        self.finish_state_load(path);
        self.add_batch_messages(load_messages);
        Ok(())
    }

    pub(crate) fn try_apply_pending_state_restore(&mut self) {
        let Some(restore) = self.user.pending_state_restore.as_ref() else {
            return;
        };
        if !self.waves_fully_loaded() {
            return;
        }
        let Some(waves) = self.user.waves.as_ref() else {
            return;
        };
        if waves.source_count() != restore.sources.len() {
            return;
        }

        let source_map = restore
            .sources
            .iter()
            .map(|(saved_source, _)| *saved_source)
            .zip(
                Self::state_source_entries(waves)
                    .into_iter()
                    .map(|(source, _)| source),
            )
            .collect::<HashMap<_, _>>();
        let restore = self
            .user
            .pending_state_restore
            .take()
            .expect("checked above");
        self.apply_loaded_wave_state(restore.waves, &source_map);
    }

    fn apply_loaded_wave_state(
        &mut self,
        mut new_waves: WaveData,
        source_map: &HashMap<SourceId, SourceId>,
    ) {
        new_waves.remap_source_ids(source_map);
        self.remap_table_sources(source_map);

        let Some(mut waves) = self.user.waves.take() else {
            return;
        };

        mem::swap(
            &mut waves.active_scope_source,
            &mut new_waves.active_scope_source,
        );
        mem::swap(&mut waves.active_scope, &mut new_waves.active_scope);
        let items = std::mem::take(&mut new_waves.displayed_items);
        let items_tree = std::mem::take(&mut new_waves.items_tree);
        let load_commands = waves.restore_items_from_state(items, items_tree, &self.translators);

        mem::swap(&mut waves.viewports, &mut new_waves.viewports);
        mem::swap(&mut waves.cursor, &mut new_waves.cursor);
        mem::swap(&mut waves.markers, &mut new_waves.markers);
        mem::swap(&mut waves.focused_item, &mut new_waves.focused_item);
        mem::swap(
            &mut waves.focused_transaction,
            &mut new_waves.focused_transaction,
        );
        mem::swap(
            &mut waves.primary_source_label,
            &mut new_waves.primary_source_label,
        );

        mem::swap(&mut waves.annotations, &mut new_waves.annotations);
        mem::swap(
            &mut waves.annotation_groups,
            &mut new_waves.annotation_groups,
        );
        mem::swap(
            &mut waves.annotation_list_visible,
            &mut new_waves.annotation_list_visible,
        );
        mem::swap(
            &mut waves.annotation_counter,
            &mut new_waves.annotation_counter,
        );
        mem::swap(
            &mut waves.selected_annotation,
            &mut new_waves.selected_annotation,
        );
        waves.default_variable_name_type = new_waves.default_variable_name_type;
        waves.display_variable_indices = new_waves.display_variable_indices;
        waves.scroll_offset = new_waves.scroll_offset;
        waves.last_active_viewport_idx = new_waves.last_active_viewport_idx;

        self.user.waves = Some(waves);
        for (source, cmd) in load_commands {
            if source == WaveData::primary_source_id() {
                self.load_variables(cmd);
            } else {
                self.load_variables_for_source(source, cmd);
            }
        }
    }

    fn finish_state_load(&mut self, path: Option<PathBuf>) {
        self.user.drag_started = false;
        self.user.drag_source_idx = None;
        self.user.drag_target_idx = None;

        self.user.previous_waves = None;
        self.user.count = None;

        self.user.state_file = path;

        self.invalidate_draw_commands();
        if let Some(waves) = &mut self.user.waves {
            waves.update_viewports();
        }
    }

    fn state_source_entries(waves: &WaveData) -> Vec<(SourceId, WaveSource)> {
        std::iter::once((WaveData::primary_source_id(), waves.source.clone()))
            .chain(
                waves
                    .sources
                    .sources
                    .iter()
                    .map(|source| (source.id, source.source.clone())),
            )
            .collect()
    }

    fn state_restore_load_messages(sources: &[(SourceId, WaveSource)]) -> Result<Vec<Message>> {
        sources
            .iter()
            .enumerate()
            .map(|(idx, (_, source))| {
                let intent = if idx == 0 {
                    LoadIntent::ReplaceSession
                } else {
                    LoadIntent::AddSource
                };
                source
                    .path()
                    .cloned()
                    .map(|path| Message::LoadFileWithIntent(path, intent))
                    .ok_or_else(|| {
                        eyre::eyre!(
                            "Cannot restore state source {source}. Only file-backed sources are supported"
                        )
                    })
            })
            .try_collect()
    }

    fn source_id_map_for_loaded_state(
        current_waves: &WaveData,
        saved_waves: &WaveData,
    ) -> HashMap<SourceId, SourceId> {
        let current_sources = Self::state_source_entries(current_waves);
        let mut used_current = HashSet::new();

        Self::state_source_entries(saved_waves)
            .into_iter()
            .map(|(saved_id, saved_source)| {
                let current_id = current_sources
                    .iter()
                    .enumerate()
                    .find_map(|(idx, (current_id, current_source))| {
                        (!used_current.contains(&idx) && *current_source == saved_source)
                            .then_some((idx, *current_id))
                    })
                    .map_or(saved_id, |(idx, current_id)| {
                        used_current.insert(idx);
                        current_id
                    });
                (saved_id, current_id)
            })
            .collect()
    }

    fn remap_table_sources(&mut self, source_map: &HashMap<SourceId, SourceId>) {
        for tile in self.user.table_tiles.values_mut() {
            tile.spec.remap_sources(source_map);
        }
    }

    /// Returns true if the waveform and all requested signals have been loaded.
    /// Used for testing to make sure the GUI is at its final state before taking a
    /// snapshot.
    pub fn waves_fully_loaded(&self) -> bool {
        self.user.waves.as_ref().is_some_and(|w| {
            w.inner.is_fully_loaded()
                && w.sources.sources.iter().all(|source| {
                    source.inner.is_fully_loaded()
                        && matches!(source.load_state, crate::source::SourceLoadState::Loaded)
                })
        })
    }

    /// Returns true if no analog caches are currently being built
    pub fn analog_caches_ready(&self) -> bool {
        self.user.waves.as_ref().is_none_or(|w| {
            w.inflight_caches.is_empty()
                && w.sources
                    .sources
                    .iter()
                    .all(|source| source.inflight_caches.is_empty())
        })
    }

    /// Returns the current canvas state
    pub(crate) fn current_canvas_state(waves: &WaveData, message: String) -> CanvasState {
        CanvasState {
            message,
            focused_item: waves.focused_item,
            focused_transaction: waves.focused_transaction.clone(),
            items_tree: waves.items_tree.clone(),
            displayed_items: waves.displayed_items.clone(),
            markers: waves.markers.clone(),
            annotations: waves.annotations.clone(),
            annotation_group: waves.annotation_groups.clone(),
            annotation_list: waves.annotation_list_visible,
            selected_annotation: waves.selected_annotation,
            annotation_counter: waves.annotation_counter,
        }
    }

    /// Push the current canvas state to the undo stack
    pub(crate) fn save_current_canvas(&mut self, message: String) {
        if let Some(waves) = &self.user.waves {
            self.undo_stack
                .push(SystemState::current_canvas_state(waves, message));

            if self.undo_stack.len() > self.user.config.undo_stack_size {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start_wcp_server(&mut self, address: Option<String>, initiate: bool) {
        use wcp::wcp_server::WcpServer;

        use crate::wcp;

        if self.wcp_server_thread.as_ref().is_some()
            || self
                .wcp_running_signal
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            warn!("WCP HTTP server is already running");
            return;
        }
        // TODO: Consider an unbounded channel?
        let (wcp_s2c_sender, wcp_s2c_receiver) = tokio::sync::mpsc::channel(100);
        let (wcp_c2s_sender, wcp_c2s_receiver) = tokio::sync::mpsc::channel(100);

        self.channels.wcp_c2s_receiver = Some(wcp_c2s_receiver);
        self.channels.wcp_s2c_sender = Some(wcp_s2c_sender);
        let stop_signal_copy = self.wcp_stop_signal.clone();
        stop_signal_copy.store(false, std::sync::atomic::Ordering::Relaxed);
        let running_signal_copy = self.wcp_running_signal.clone();
        running_signal_copy.store(true, std::sync::atomic::Ordering::Relaxed);
        let greeted_signal_copy = self.wcp_greeted_signal.clone();
        greeted_signal_copy.store(true, std::sync::atomic::Ordering::Relaxed);

        let ctx = self.context.clone();
        let address = address.unwrap_or(self.user.config.wcp.address.clone());
        self.wcp_server_address = Some(address.clone());
        self.wcp_server_thread = Some(tokio::spawn(async move {
            let server = WcpServer::new(
                address,
                initiate,
                wcp_c2s_sender,
                wcp_s2c_receiver,
                stop_signal_copy,
                running_signal_copy,
                greeted_signal_copy,
                ctx,
            )
            .await;
            match server {
                Ok(mut server) => server.run().await,
                Err(m) => {
                    error!("Could not start WCP server. {m:?}");
                }
            }
        }));
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn stop_wcp_server(&mut self) {
        // stop wcp server if there is one running

        if self.wcp_server_address.is_some() && self.wcp_server_thread.is_some() {
            // signal the server to stop
            self.wcp_stop_signal
                .store(true, std::sync::atomic::Ordering::Relaxed);

            self.wcp_server_thread = None;
            self.wcp_server_address = None;
            self.channels.wcp_s2c_sender = None;
            self.channels.wcp_c2s_receiver = None;
            info!("Stopped WCP server");
        }
    }
}
