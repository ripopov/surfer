use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem,
};

use camino::Utf8PathBuf;

use crate::{
    CanvasState, StartupParams,
    clock_highlighting::ClockHighlightType,
    config::{
        ArrowKeyBindings, AutoLoad, FocusHighlight, PrimaryMouseDrag, SurferConfig, TransitionValue,
    },
    data_container::DataContainer,
    dialog::{OpenSiblingStateFileDialog, ReloadWaveformDialog},
    frame_buffer::FrameBufferSettings,
    hierarchy::{HierarchyStyle, ParameterDisplayLocation},
    message::Message,
    system_state::SystemState,
    time::{TimeStringFormatting, TimeUnit},
    trace_style::TraceStyle,
    transaction_container::TransactionContainer,
    variable_filter::VariableFilter,
    wave_container::{ScopeRef, VariableRef, WaveContainer},
    wave_data::TimeRange,
    wave_source::{LoadOptions, WaveFormat, WaveSource},
};
use egui::{
    Visuals,
    style::{Selection, WidgetVisuals, Widgets},
};
use epaint::{CornerRadius, Stroke};
use eyre::{Result, WrapErr as _};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use surfer_translation_types::Translator;
use surver::SurverFileInfo;
use tracing::{error, info, trace, warn};

// Keep runtime state and its wire fields in one declaration. The decoder
// inspects the original RON before choosing native or version-zero ownership.
macro_rules! user_state_fields {
    ($(#[$meta:meta])* pub struct UserState {
        $($(#[$attr:meta])* $visibility:vis $field:ident: $ty:ty,)*
    }) => {
        $(#[$meta])*
        #[derive(Serialize)]
        pub struct UserState { $($(#[$attr])* $visibility $field: $ty,)* }

        #[derive(Deserialize)]
        #[serde(default)]
        struct UserStateFields { $($(#[$attr])* $field: $ty,)* }
        impl Default for UserStateFields {
            fn default() -> Self {
                let state = UserState::default();
                Self { $($field: state.$field,)* }
            }
        }
        impl From<UserStateFields> for UserState {
            fn from(state: UserStateFields) -> Self { Self { $($field: state.$field,)* } }
        }
    };
}

user_state_fields! {
/// The parts of the program state that need to be serialized when loading/saving state
pub struct UserState {
    pub(crate) state_version: u32,
    #[serde(skip)]
    pub config: SurferConfig,

    /// Overrides for the config show_* fields.
    ///
    /// Defaults to `config.show_*` if not present
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

    pub(crate) waves: Option<crate::wave_data::WaveData>,

    pub(crate) workspace: crate::tiles::workspace::Workspace,

    pub(crate) previous_waves: Option<crate::wave_data::WaveData>,

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
    pub(crate) wanted_timeunit: TimeUnit,
    pub(crate) time_string_format: Option<TimeStringFormatting>,
    pub(crate) show_url_entry: bool,
    /// Show a confirmation dialog asking the user for confirmation
    /// that surfer should reload changed files from disk.
    #[serde(skip, default)]
    pub(crate) show_reload_suggestion: Option<ReloadWaveformDialog>,
    #[serde(skip, default)]
    pub(crate) show_open_sibling_state_file_suggestion: Option<OpenSiblingStateFileDialog>,
    /// Unused, but kept for backward compatibility with old state files.
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
    pub(crate) draw_vector_unknowns_as_line: Option<bool>,
    #[serde(default)]
    pub(crate) focus_highlight: Option<FocusHighlight>,

    // Path of last saved-to state file
    // Do not serialize as this causes a few issues and doesn't help:
    // - We need to set it on load of a state anyways since the file could have been renamed
    // - Bad interoperatility story between native and wasm builds
    // - Sequencing issue in serialization, due to us having to run that async
    #[serde(skip)]
    pub state_file: Option<Utf8PathBuf>,

    #[serde(default)]
    pub(crate) enable_time_offset: Option<bool>,
}

}

fn present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct StateOwnershipProbe {
    #[serde(deserialize_with = "present")]
    frame_buffer: Option<FrameBufferSettings>,
    show_annotation_list: bool,
    #[serde(alias = "show_marker_window")]
    show_cursor_window: bool,
    show_logs: bool,
    #[serde(deserialize_with = "present")]
    state_version: Option<u32>,
    #[serde(deserialize_with = "present")]
    workspace: Option<Box<ron::value::RawValue>>,
    waves: Option<Box<ron::value::RawValue>>,
    previous_waves: Option<Box<ron::value::RawValue>>,
}

impl<'de> Deserialize<'de> for UserState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let raw = Box::<ron::value::RawValue>::deserialize(deserializer)?;
        let mut probe: StateOwnershipProbe =
            crate::tiles::serde::decode(raw.get_ron()).map_err(D::Error::custom)?;
        let version = probe
            .state_version
            .unwrap_or(u32::from(probe.workspace.is_some()));
        if version > 1 {
            return Err(D::Error::custom(format!(
                "unsupported state version {version}"
            )));
        }
        if version == 1 && probe.workspace.is_none() {
            return Err(D::Error::custom("version-one state requires a workspace"));
        }
        if version == 0 && probe.workspace.is_some() {
            return Err(D::Error::custom(
                "version-zero state cannot contain a native workspace",
            ));
        }
        let fields: UserStateFields =
            crate::tiles::serde::decode(raw.get_ron()).map_err(D::Error::custom)?;
        let mut state = UserState::from(fields);
        if version == 0 {
            let previous = probe.waves.is_none();
            if let Some(old) = probe.waves.or(probe.previous_waves) {
                let old: crate::tiles::legacy::LegacyWaveformV0 =
                    crate::tiles::serde::decode(old.get_ron()).map_err(D::Error::custom)?;
                let mut runtime = crate::tiles::runtime::WorkspaceRuntime::default();
                let migrated = old.into_workspace(&mut runtime).map_err(D::Error::custom)?;
                state.workspace = migrated.workspace;
                probe.show_annotation_list |= migrated.annotation_list_visible;
                if previous {
                    state.previous_waves = Some(migrated.document);
                } else {
                    state.waves = Some(migrated.document);
                }
            }
        }
        if probe.state_version.is_none() {
            let mut runtime = crate::tiles::runtime::WorkspaceRuntime::default();
            runtime
                .install_workspace(
                    state.workspace.tiles().keys().copied(),
                    state.workspace.item_lists().keys().copied(),
                )
                .map_err(D::Error::custom)?;
            for (visible, kind, direction) in [
                (
                    probe.show_annotation_list,
                    "annotation_list",
                    crate::tiles::layout::Direction::Right,
                ),
                (
                    version == 0
                        && state.workspace.layout().tile_order().into_iter().any(|id| {
                            state
                                .workspace
                                .waveform_resources(id)
                                .is_some_and(|(_, view)| view.focused_transaction.is_some())
                        }),
                    "transaction_details",
                    crate::tiles::layout::Direction::Right,
                ),
                (
                    probe.show_logs,
                    "logs",
                    crate::tiles::layout::Direction::Down,
                ),
                (
                    probe.show_cursor_window,
                    "markers",
                    crate::tiles::layout::Direction::Right,
                ),
            ] {
                if visible {
                    state
                        .workspace
                        .apply_command(
                            &mut runtime,
                            crate::tiles::commands::WorkspaceCommand::OpenTile {
                                kind: kind.into(),
                                placement: crate::tiles::layout::Placement::Edge(direction),
                                focus: false,
                            },
                        )
                        .map_err(D::Error::custom)?;
                }
            }
        }
        if let Some(settings) = probe.frame_buffer
            && settings != FrameBufferSettings::default()
            && !state
                .workspace
                .tiles()
                .values()
                .any(|entry| matches!(entry.kind, crate::tiles::kind::TileKind::FrameBuffer(_)))
        {
            let mut runtime = crate::tiles::runtime::WorkspaceRuntime::default();
            runtime
                .install_workspace(
                    state.workspace.tiles().keys().copied(),
                    state.workspace.item_lists().keys().copied(),
                )
                .map_err(D::Error::custom)?;
            let before = state
                .workspace
                .tiles()
                .keys()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            state
                .workspace
                .apply_command(
                    &mut runtime,
                    crate::tiles::commands::WorkspaceCommand::CreateTile {
                        kind: "frame_buffer".into(),
                        placement: crate::tiles::layout::Placement::Edge(
                            crate::tiles::layout::Direction::Right,
                        ),
                        focus: false,
                    },
                )
                .map_err(D::Error::custom)?;
            let id = *state
                .workspace
                .tiles()
                .keys()
                .find(|id| !before.contains(id))
                .unwrap();
            state
                .workspace
                .apply_tile_message(
                    id,
                    crate::tiles::kind::TileMessage::FrameBuffer(
                        crate::tile_kinds::frame_buffer::FrameBufferMessage::State(Box::new(
                            crate::tile_kinds::frame_buffer::FrameBufferState {
                                settings,
                                ..Default::default()
                            },
                        )),
                    ),
                    None,
                )
                .map_err(D::Error::custom)?;
        }
        state.state_version = 1;
        Ok(state)
    }
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
            state_version: 1,
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
            fill_high_values: None,
            primary_button_drag_behavior: None,
            arrow_key_bindings: None,
            clock_highlight_type: None,
            hierarchy_style: None,
            autoload_sibling_state_files: None,
            autoreload_files: None,
            enable_time_offset: None,
            waves: None,
            workspace: Default::default(),
            previous_waves: None,
            count: None,
            blacklisted_translators: HashSet::new(),
            show_about: false,
            show_keys: false,
            show_gestures: false,
            show_quick_start: false,
            show_license: false,
            show_performance: false,
            wanted_timeunit: TimeUnit::None,
            time_string_format: None,
            show_url_entry: false,
            show_reload_suggestion: None,
            show_open_sibling_state_file_suggestion: None,
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
            toolbar_group_enabled: HashMap::new(),
            toolbar_group_rows: Vec::new(),
            draw_vector_unknowns_as_line: None,
            focus_highlight: None,
        }
    }
}

impl UserState {
    pub(crate) fn waveform_read(&self) -> Option<crate::wave_data::WaveformRead<'_>> {
        let id = self
            .workspace
            .resolve_waveform(crate::tiles::TileTarget::Focused)?;
        self.waveform_read_at(id)
    }
    pub(crate) fn waveform_read_at(
        &self,
        id: crate::tiles::TileId,
    ) -> Option<crate::wave_data::WaveformRead<'_>> {
        let (list, view) = self.workspace.waveform_resources(id)?;
        Some(crate::wave_data::WaveformRead {
            document: self.waves.as_ref()?,
            items: list,
            view,
            tile_id: id,
        })
    }
    pub(crate) fn waveform_edit(&mut self) -> Option<crate::wave_data::WaveformEdit<'_>> {
        let id = self
            .workspace
            .resolve_waveform(crate::tiles::TileTarget::Focused)?;
        self.waveform_edit_at(id)
    }
    pub(crate) fn waveform_edit_at(
        &mut self,
        id: crate::tiles::TileId,
    ) -> Option<crate::wave_data::WaveformEdit<'_>> {
        self.workspace.waveform_edit(id, self.waves.as_mut()?)
    }
}

impl SystemState {
    pub fn with_params(mut self, args: StartupParams) -> Self {
        self.user.previous_waves = self.user.waves;
        self.user.waves = None;

        // we turn the waveform argument and any startup command file into batch commands
        self.batch_messages = VecDeque::new();

        match args.waves {
            Some(WaveSource::Url(url)) => {
                self.add_batch_message(Message::LoadWaveformFileFromUrl(url, LoadOptions::KeepAll));
            }
            Some(WaveSource::File(file)) => {
                self.add_batch_message(Message::LoadFile(file, LoadOptions::KeepAll));
            }
            Some(WaveSource::Data) => error!("Attempted to load data at startup"),
            Some(WaveSource::Cxxrtl(url)) => {
                self.add_batch_message(Message::SetupCxxrtl(url));
            }
            Some(WaveSource::DragAndDrop(_)) => {
                error!("Attempted to load from drag and drop at startup (how?)");
            }
            None => {}
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
        let Some(waves) = self.user.waves.as_mut() else {
            return vec![];
        };

        let wave_cont = waves.inner.as_waves().unwrap();

        let children = wave_cont.child_scopes(scope);
        let mut variables = wave_cont
            .variables_in_scope(scope)
            .iter()
            .sorted_by(|a, b| numeric_sort::cmp(&a.name, &b.name))
            .cloned()
            .collect_vec();

        if recursive && let Ok(children) = children {
            for child in children {
                variables.append(&mut self.get_scope(&child, true));
            }
        }

        variables
    }

    pub(crate) fn on_waves_loaded(
        &mut self,
        filename: WaveSource,
        format: WaveFormat,
        new_waves: WaveContainer,
        transactions: Option<TransactionContainer>,
        load_options: LoadOptions,
    ) {
        for translator in self.translators.all_translators() {
            translator.set_wave_source(Some(filename.into_translation_type()));
        }
        let inner = match transactions {
            Some(transactions) => DataContainer::Combined {
                waves: new_waves,
                transactions,
            },
            None => DataContainer::Waves(new_waves),
        };
        self.install_document(inner, filename, format, load_options);
    }

    pub(crate) fn on_transaction_streams_loaded(
        &mut self,
        filename: WaveSource,
        format: WaveFormat,
        new_ftr: TransactionContainer,
        load_options: LoadOptions,
    ) {
        self.install_document(
            DataContainer::Transactions(new_ftr),
            filename,
            format,
            load_options,
        );
        self.user.config.theme.alt_frequency = 0;
    }

    fn install_document(
        &mut self,
        inner: DataContainer,
        source: WaveSource,
        format: WaveFormat,
        options: LoadOptions,
    ) {
        use crate::tiles::{commands::WorkspaceCommand, layout::Placement};
        if let Err(error) = self.workspace_runtime.document_changed() {
            error!("Document replacement rejected: {error}");
            return;
        }
        if self.user.workspace.tiles().is_empty() && !self.workspace_runtime.workspace_initialized()
        {
            if let Err(error) = self.user.workspace.apply_command(
                &mut self.workspace_runtime,
                WorkspaceCommand::CreateTile {
                    kind: "waveform".into(),
                    placement: Placement::Root,
                    focus: true,
                },
            ) {
                error!("Initial waveform creation failed: {error}");
                return;
            }
            self.user
                .workspace
                .set_default_name_type(self.user.config.default_variable_name_type);
        }
        let previous = self
            .user
            .waves
            .take()
            .or_else(|| self.user.previous_waves.take());
        let clear = options == LoadOptions::Clear;
        let mut document = crate::wave_data::WaveData {
            inner,
            source: source.clone(),
            format,
            active_scope: None,
            cursor: None,
            markers: HashMap::new(),
            display_variable_indices: self.show_variable_indices(),
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: HashMap::new(),
            cached_time_range: TimeRange::default(),
        };
        if let Some(previous) = previous {
            document.cache_generation = previous.cache_generation.saturating_add(1);
            if !clear {
                document.old_max_timestamp = previous.max_timestamp();
                document.cursor = previous.cursor;
                document.markers = previous.markers;
                document.active_scope = previous.active_scope.filter(|scope| match scope {
                    crate::wave_data::ScopeType::WaveScope(scope) => document
                        .inner
                        .as_waves()
                        .is_some_and(|waves| waves.scope_exists(scope)),
                    crate::wave_data::ScopeType::StreamScope(scope) => document
                        .inner
                        .as_transactions()
                        .is_some_and(|transactions| transactions.stream_scope_exists(scope)),
                });
            }
        }
        document.refresh_time_range(self.enable_time_offset());
        let loads = self.user.workspace.attach_document(
            &mut document,
            &self.translators,
            clear,
            options == LoadOptions::KeepAll,
        );
        self.user.workspace.update_viewports(&mut document);
        self.user.workspace.ensure_annotation_groups();
        self.user.wanted_timeunit = document.inner.metadata().timescale.unit;
        let title = document.window_title();
        self.user.waves = Some(document);
        self.undo_stack.clear();
        self.redo_stack.clear();
        for load in loads {
            self.load_variables(load);
        }
        self.invalidate_draw_commands();
        self.record_file_history(&source);
        if let Some(context) = &self.context {
            context.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            #[cfg(target_arch = "wasm32")]
            if let Some(document) = web_sys::window().and_then(|window| window.document()) {
                document.set_title(&title);
            }
        }
        if source.sibling_state_file().is_some() {
            self.update(Message::SuggestOpenSiblingStateFile);
        }
    }

    fn record_file_history(&mut self, source: &WaveSource) {
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

    pub(crate) fn load_state(
        &mut self,
        mut loaded_state: Box<UserState>,
        path: Option<Utf8PathBuf>,
    ) {
        if let Err(error) = self.workspace_runtime.install_workspace(
            loaded_state.workspace.tiles().keys().copied(),
            loaded_state.workspace.item_lists().keys().copied(),
        ) {
            error!("State replacement rejected: {error}");
            return;
        }
        // first swap everything, fix special cases afterwards
        mem::swap(&mut self.user, &mut loaded_state);

        // swap back waves for inner, source, format since we want to keep the file
        // fix up all wave references from paths if a wave is loaded
        mem::swap(&mut loaded_state.waves, &mut self.user.waves);
        let loads = if let Some(document) = self.user.waves.as_mut() {
            if let Some(saved) = loaded_state.waves.take() {
                document.active_scope = saved.active_scope;
                document.cursor = saved.cursor;
                document.markers = saved.markers;
            }
            self.user
                .workspace
                .attach_document(document, &self.translators, false, true)
        } else {
            Vec::new()
        };
        for load in loads {
            self.load_variables(load);
        }
        self.undo_stack.clear();
        self.redo_stack.clear();

        if let Some(ctx) = &self.context {
            egui::DragAndDrop::clear_payload(ctx);
        }

        // Keep saved document settings with the pending workspace until a file loads.
        self.user.previous_waves = if self.user.waves.is_none() {
            loaded_state
                .waves
                .take()
                .or_else(|| self.user.previous_waves.take())
        } else {
            None
        };
        self.user.count = None;

        // use just loaded path since path is not part of the export as it might have changed anyways
        self.user.state_file = path;

        let enable_time_offset = self.enable_time_offset();
        self.invalidate_draw_commands();
        if let Some(waves) = &mut self.user.waves {
            waves.refresh_time_range(enable_time_offset);
            self.user.workspace.update_viewports(waves);
        }
    }

    /// Returns true if the waveform and all requested signals have been loaded.
    /// Used for testing to make sure the GUI is at its final state before taking a
    /// snapshot.
    pub fn waves_fully_loaded(&self) -> bool {
        self.can_start_batch_command()
            && self.pending_document.is_none()
            && self
                .user
                .waves
                .as_ref()
                .is_some_and(|w| w.inner.is_fully_loaded())
    }

    /// Returns true if no analog caches are currently being built
    pub fn analog_caches_ready(&self) -> bool {
        self.user
            .waves
            .as_ref()
            .is_none_or(|w| w.inflight_caches.is_empty())
    }

    /// Snapshot one list's content for history. Shared marker times are not
    /// part of it; marker edits record their own value (§9).
    pub(crate) fn current_canvas_state(
        list: crate::tiles::ItemListId,
        items: &crate::item_list::ItemList,
        message: String,
    ) -> CanvasState {
        CanvasState {
            message,
            list,
            items_tree: items.items_tree.clone(),
            displayed_items: items.displayed_items.clone(),
            graphics: items.graphics.clone(),
            default_variable_name_type: items.default_variable_name_type,
            annotations: items.annotations.clone(),
            annotation_group: items.annotation_groups.clone(),
            annotation_counter: items.annotation_counter,
        }
    }

    /// Restore the recorded list, regardless of current focus. Surviving views
    /// retain navigation and repair only references invalidated by the edit.
    pub(crate) fn restore_canvas_state(
        &mut self,
        previous: CanvasState,
    ) -> Result<CanvasState, Box<CanvasState>> {
        let inverse = self.user.workspace.restore_items(previous)?;
        self.invalidate_draw_commands();
        Ok(inverse)
    }

    /// Record the target waveform's list before an item edit. Shared marker
    /// times are not part of the record, so unrelated marker changes survive undo.
    pub(crate) fn save_current_canvas(&mut self, message: String) {
        if let Some(waves) = self.user.waveform_read() {
            self.record_canvas_edit(SystemState::current_canvas_state(
                self.user.workspace.tiles()[&waves.tile_id]
                    .kind
                    .waveform_list()
                    .unwrap(),
                waves.items,
                message,
            ));
        }
    }

    /// Record a pre-edit snapshot only after a checked edit succeeds.
    pub(crate) fn record_canvas_edit(&mut self, before: CanvasState) {
        self.record_edit(crate::tiles::history::UndoRecord::Items(Box::new(before)));
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

#[cfg(test)]
mod empty_workspace_tests {
    use super::*;
    #[test]
    fn explicit_empty_layout_is_preserved_but_rejected_commands_do_not_initialize_it() {
        use crate::{
            Message,
            tiles::{TileId, commands::WorkspaceCommand},
        };
        for explicit in [true, false] {
            let mut state = SystemState::new_default_config().unwrap();
            if explicit {
                state
                    .update(Message::Workspace(WorkspaceCommand::SetLayout(None)))
                    .unwrap();
            } else {
                assert!(
                    state
                        .update(Message::Workspace(WorkspaceCommand::CloseTile(TileId(99))))
                        .is_none()
                );
            }
            state.install_document(
                DataContainer::Empty,
                WaveSource::Data,
                WaveFormat::Vcd,
                LoadOptions::KeepAll,
            );
            assert_eq!(state.user.workspace.tiles().is_empty(), explicit);
        }
    }

    #[test]
    fn reload_and_saved_empty_layout_do_not_recreate_a_closed_waveform() {
        let mut state = SystemState::new_default_config().unwrap();
        let install = |state: &mut SystemState| {
            state.install_document(
                DataContainer::Empty,
                WaveSource::Data,
                WaveFormat::Vcd,
                LoadOptions::KeepAll,
            )
        };
        install(&mut state);
        assert_eq!(state.user.workspace.tiles().len(), 1);
        let id = state.user.workspace.layout().focused().unwrap();
        state
            .update(crate::Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::CloseTile(id),
            ))
            .unwrap();
        install(&mut state);
        assert!(state.user.workspace.tiles().is_empty());
        let saved = state.encode_state().unwrap();
        let restored: UserState = crate::tiles::serde::decode(&saved).unwrap();
        let mut fresh = SystemState::new_default_config().unwrap();
        fresh.load_state(Box::new(restored), None);
        install(&mut fresh);
        assert!(fresh.user.workspace.tiles().is_empty());
    }
}
