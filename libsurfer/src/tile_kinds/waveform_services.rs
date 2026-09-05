//! Read-only dependencies of waveform rendering. No application or workspace access.

use crate::{
    SystemState,
    clock_highlighting::ClockHighlightType,
    config::{FocusHighlight, PrimaryMouseDrag, SurferConfig},
    time::{TimeFormat, TimeUnit},
    trace_style::TraceStyle,
    translation::TranslatorList,
};

pub(crate) struct WaveformReadServices<'a> {
    pub config: &'a SurferConfig,
    pub translators: &'a TranslatorList,
    pub translator_generation: u64,
    pub wanted_timeunit: TimeUnit,
    pub time_format: TimeFormat,
    pub trace_style: TraceStyle,
    pub show_default_timeline: bool,
    pub show_ticks: bool,
    pub focus_highlight: FocusHighlight,
    pub fill_high_values: bool,
    pub show_divider_text: bool,
    pub draw_vector_unknowns_as_line: bool,
    pub clock_highlight_type: ClockHighlightType,
    pub primary_button_drag_behavior: PrimaryMouseDrag,
    pub align_names_right: bool,
    pub show_tooltip: bool,
    pub transition_value: crate::config::TransitionValue,
    pub blacklisted_translators:
        &'a std::collections::HashSet<(crate::wave_container::VariableRef, String)>,
    pub variable_name_info_cache: &'a std::cell::RefCell<crate::system_state::VariableInfoCache>,
    pub wcp_capabilities: Option<&'a crate::WcpClientCapabilities>,
    #[cfg(feature = "performance_plot")]
    pub timing: &'a std::cell::RefCell<crate::benchmark::Timing>,
}

impl SystemState {
    pub(crate) fn waveform_services(&self) -> WaveformReadServices<'_> {
        WaveformReadServices {
            config: &self.user.config,
            translators: &self.translators,
            translator_generation: self.translator_generation,
            wanted_timeunit: self.user.wanted_timeunit,
            time_format: self.get_time_format(),
            trace_style: self.trace_style(),
            show_default_timeline: self.show_default_timeline(),
            show_ticks: self.show_ticks(),
            focus_highlight: self.focus_highlight(),
            fill_high_values: self.fill_high_values(),
            show_divider_text: self.show_divider_text(),
            draw_vector_unknowns_as_line: self.draw_vector_unknowns_as_line(),
            clock_highlight_type: self.clock_highlight_type(),
            primary_button_drag_behavior: self.primary_button_drag_behavior(),
            align_names_right: self.align_names_right(),
            show_tooltip: self.show_tooltip(),
            transition_value: self.transition_value(),
            blacklisted_translators: &self.user.blacklisted_translators,
            variable_name_info_cache: &self.variable_name_info_cache,
            wcp_capabilities: self
                .wcp_greeted_signal
                .load(std::sync::atomic::Ordering::Relaxed)
                .then_some(&self.wcp_client_capabilities),
            #[cfg(feature = "performance_plot")]
            timing: &self.timing,
        }
    }
}

impl WaveformReadServices<'_> {
    pub fn do_measure(&self, modifiers: &egui::Modifiers) -> bool {
        (self.primary_button_drag_behavior == PrimaryMouseDrag::Measure && !modifiers.shift)
            || (self.primary_button_drag_behavior == PrimaryMouseDrag::Cursor && modifiers.shift)
    }
}
