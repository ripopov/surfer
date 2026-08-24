use ecolor::Color32;
use egui::{CornerRadius, DragValue, Pos2, Rect, Sense, Stroke};
use serde::{Deserialize, Serialize};
use surfer_translation_types::VariableValue;

use crate::translation::ycbcr_to_rgb;
use crate::wave_container::{ScopeRef, ScopeRefExt, VariableRef, VariableRefExt, WaveContainer};
use crate::{Message, system_state::SystemState};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub(crate) struct FrameBufferSettings {
    pub pixels_per_row: usize,
    pub square_pixels: bool,
    #[serde(flatten)]
    pub color_settings: PixelColorSettings,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameBufferColorMode {
    #[default]
    Grayscale,
    Rgb,
    YCbCr,
}

impl std::fmt::Display for FrameBufferColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.string())
    }
}

impl FrameBufferColorMode {
    fn string(&self) -> &'static str {
        match self {
            FrameBufferColorMode::Grayscale => "Grayscale",
            FrameBufferColorMode::Rgb => "RGB",
            FrameBufferColorMode::YCbCr => "YCbCr",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct PixelColorSettings {
    #[serde(default)]
    pub color_mode: FrameBufferColorMode,
    pub grayscale_bits: u8,
    pub r_bits: u8,
    pub g_bits: u8,
    pub b_bits: u8,
    #[serde(default = "default_y_bits")]
    pub y_bits: u8,
    #[serde(default = "default_cb_bits")]
    pub cb_bits: u8,
    #[serde(default = "default_cr_bits")]
    pub cr_bits: u8,
}

fn default_y_bits() -> u8 {
    8
}

fn default_cb_bits() -> u8 {
    8
}

fn default_cr_bits() -> u8 {
    8
}

impl Default for PixelColorSettings {
    fn default() -> Self {
        Self {
            color_mode: FrameBufferColorMode::Grayscale,
            grayscale_bits: 1,
            r_bits: 3,
            g_bits: 3,
            b_bits: 2,
            y_bits: default_y_bits(),
            cb_bits: default_cb_bits(),
            cr_bits: default_cr_bits(),
        }
    }
}

impl Default for FrameBufferSettings {
    fn default() -> Self {
        Self {
            pixels_per_row: 16,
            square_pixels: true,
            color_settings: PixelColorSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArrayLevel {
    pub min_index: i64,
    pub max_index: i64,
    pub first_index: i64,
    pub last_index: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameBufferContentCacheKey {
    pub content: FrameBufferContent,
    pub cursor_position: num::BigUint,
}

#[derive(Debug, Clone)]
pub(crate) struct FrameBufferArrayCache {
    pub key: FrameBufferContentCacheKey,
    pub cached_value: Option<std::sync::Arc<[bool]>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameBufferPixelCacheKey {
    pub array_key: FrameBufferContentCacheKey,
    pub settings: PixelColorSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct FrameBufferPixelCache {
    pub key: FrameBufferPixelCacheKey,
    pub pixel_colors: std::sync::Arc<[Color32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameBufferContent {
    Array {
        scope_ref: ScopeRef,
        /// One range-selector per level of array nesting.
        /// The last level always applies to variables.
        levels: Vec<ArrayLevel>,
    },
    Variable(VariableRef),
}

impl SystemState {
    pub(crate) fn draw_frame_buffer_window(
        &mut self,
        ctx: &egui::Context,
        msgs: &mut Vec<Message>,
    ) {
        let mut open = true;
        egui::Window::new("Frame Buffer")
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                let frame_buffer_value = self.selected_variable_for_frame_buffer();
                let Some((bits, array_cache_key, variable_name)) = frame_buffer_value.as_ref()
                else {
                    ui.label("Place the cursor.");
                    return;
                };

                let color_settings_key = {
                    let settings = &mut self.user.frame_buffer;
                    let color_settings = &mut settings.color_settings;

                    ui.checkbox(&mut settings.square_pixels, "Square pixels");

                    ui.horizontal(|ui| {
                        ui.label("Color mode");
                        egui::ComboBox::from_id_salt("frame_buffer_color_mode")
                            .selected_text(color_settings.color_mode.string())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut color_settings.color_mode,
                                    FrameBufferColorMode::Grayscale,
                                    FrameBufferColorMode::Grayscale.string(),
                                );
                                ui.selectable_value(
                                    &mut color_settings.color_mode,
                                    FrameBufferColorMode::Rgb,
                                    FrameBufferColorMode::Rgb.string(),
                                );
                                ui.selectable_value(
                                    &mut color_settings.color_mode,
                                    FrameBufferColorMode::YCbCr,
                                    FrameBufferColorMode::YCbCr.string(),
                                );
                            });
                    });

                    match color_settings.color_mode {
                        FrameBufferColorMode::Grayscale => {
                            ui.horizontal(|ui| {
                                ui.label("Grayscale bits");
                                ui.add(
                                    DragValue::new(&mut color_settings.grayscale_bits).range(1..=8),
                                );
                            });
                        }
                        FrameBufferColorMode::Rgb => {
                            ui.horizontal(|ui| {
                                ui.label("R bits");
                                ui.add(DragValue::new(&mut color_settings.r_bits).range(0..=8));
                                ui.label("G bits");
                                ui.add(DragValue::new(&mut color_settings.g_bits).range(0..=8));
                                ui.label("B bits");
                                ui.add(DragValue::new(&mut color_settings.b_bits).range(0..=8));
                            });
                        }
                        FrameBufferColorMode::YCbCr => {
                            ui.horizontal(|ui| {
                                ui.label("Y bits");
                                ui.add(DragValue::new(&mut color_settings.y_bits).range(0..=8));
                                ui.label("Cb bits");
                                ui.add(DragValue::new(&mut color_settings.cb_bits).range(0..=8));
                                ui.label("Cr bits");
                                ui.add(DragValue::new(&mut color_settings.cr_bits).range(0..=8));
                            });
                        }
                    }

                    color_settings.clone()
                };

                ui.separator();

                if bits.is_empty() {
                    ui.label("No bits available");
                    return;
                }

                let pixel_cache_key = FrameBufferPixelCacheKey {
                    array_key: array_cache_key.clone(),
                    settings: color_settings_key.clone(),
                };

                let pixel_colors = if let Some(cache) = self
                    .frame_buffer_pixel_cache
                    .as_ref()
                    .filter(|cache| cache.key == pixel_cache_key)
                {
                    cache.pixel_colors.clone()
                } else {
                    let decoded = match color_settings_key.color_mode {
                        FrameBufferColorMode::Rgb => {
                            let r_bits = color_settings_key.r_bits as usize;
                            let g_bits = color_settings_key.g_bits as usize;
                            let b_bits = color_settings_key.b_bits as usize;
                            let bits_per_pixel = r_bits + g_bits + b_bits;
                            if bits_per_pixel == 0 {
                                ui.label("Set at least one RGB channel bit count above zero.");
                                return;
                            }
                            decode_rgb_pixels(bits, r_bits, g_bits, b_bits)
                        }
                        FrameBufferColorMode::YCbCr => {
                            let y_bits = color_settings_key.y_bits as usize;
                            let cb_bits = color_settings_key.cb_bits as usize;
                            let cr_bits = color_settings_key.cr_bits as usize;
                            let bits_per_pixel = y_bits + cb_bits + cr_bits;
                            if bits_per_pixel == 0 {
                                ui.label("Set at least one YCbCr channel bit count above zero.");
                                return;
                            }
                            decode_ycbcr_pixels(bits, y_bits, cb_bits, cr_bits)
                        }
                        FrameBufferColorMode::Grayscale => {
                            let gray_bits = color_settings_key.grayscale_bits as usize;
                            decode_grayscale_pixels(bits, gray_bits)
                        }
                    };

                    let decoded: std::sync::Arc<[Color32]> = decoded.into();

                    self.frame_buffer_pixel_cache = Some(FrameBufferPixelCache {
                        key: pixel_cache_key,
                        pixel_colors: decoded.clone(),
                    });
                    decoded
                };

                if pixel_colors.is_empty() {
                    ui.label("No pixels to draw with current bit settings.");
                    return;
                }

                let settings = &mut self.user.frame_buffer;
                let columns = settings.pixels_per_row.min(pixel_colors.len()).max(1);
                let rows = pixel_colors.len().div_ceil(columns);
                ui.horizontal(|ui| {
                    ui.label(format!("Var: {variable_name} | {columns}×{rows}"));

                    if ui.button("Copy image").clicked() {
                        let total = columns * rows;
                        let mut padded = pixel_colors.to_vec();
                        padded.resize(total, Color32::BLACK);
                        ui.ctx().copy_image(egui::ColorImage {
                            size: [columns, rows],
                            pixels: padded,
                            source_size: egui::vec2(columns as f32, rows as f32),
                        });
                    }
                });
                self.draw_array_index_range(ui);

                let settings = &mut self.user.frame_buffer;
                let max_columns = pixel_colors.len().max(1);
                settings.pixels_per_row = settings.pixels_per_row.clamp(1, max_columns);

                ui.horizontal(|ui| {
                    ui.label("Pixels in x-direction");
                    ui.add(
                        egui::Slider::new(&mut settings.pixels_per_row, 1..=max_columns).integer(),
                    );
                });

                ui.separator();

                let available = ui.available_size_before_wrap();

                if available.x <= 0.0 || available.y <= 0.0 {
                    return;
                }

                let (pixel_width, pixel_height) = if settings.square_pixels {
                    let side = (available.x / columns as f32).min(available.y / rows as f32);
                    (side, side)
                } else {
                    (available.x / columns as f32, available.y / rows as f32)
                };

                let image_size =
                    egui::vec2(pixel_width * columns as f32, pixel_height * rows as f32);
                let (rect, _) = ui.allocate_exact_size(image_size, Sense::hover());
                let painter = ui.painter_at(rect);

                for (index, color) in pixel_colors.iter().copied().enumerate() {
                    let x = index % columns;
                    let y = index / columns;

                    let min = Pos2 {
                        x: rect.min.x + x as f32 * pixel_width,
                        y: rect.min.y + y as f32 * pixel_height,
                    };
                    let max = Pos2 {
                        x: min.x + pixel_width,
                        y: min.y + pixel_height,
                    };

                    painter.rect_filled(Rect { min, max }, CornerRadius::ZERO, color);
                }

                painter.rect_stroke(
                    rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, ui.visuals().weak_text_color()),
                    egui::StrokeKind::Inside,
                );
            });

        if !open {
            msgs.push(Message::SetFrameBufferVisibleVariable(None));
        }
    }

    fn draw_array_index_range(&mut self, ui: &mut egui::Ui) {
        let Some(FrameBufferContent::Array {
            scope_ref: _,
            levels,
        }) = self.frame_buffer_content.as_mut()
        else {
            return;
        };

        if levels.is_empty() {
            return;
        }

        let total_levels = levels.len();

        for (i, level) in levels.iter_mut().enumerate() {
            let (min, max) = (level.min_index, level.max_index);
            level.first_index = level.first_index.clamp(min, max);
            level.last_index = level.last_index.clamp(min, max);
            if level.first_index > level.last_index {
                level.last_index = level.first_index;
            }
            ui.horizontal(|ui| {
                if total_levels == 1 {
                    ui.label("First array index");
                } else {
                    ui.label(format!("Level {} first index", i + 1));
                }
                ui.add(DragValue::new(&mut level.first_index).range(min..=max));
                if total_levels == 1 {
                    ui.label("Last array index");
                } else {
                    ui.label(format!("Level {} last index", i + 1));
                }
                ui.add(DragValue::new(&mut level.last_index).range(min..=max));
            });
            if level.first_index > level.last_index {
                level.first_index = level.last_index;
            }
        }
    }

    fn selected_variable_for_frame_buffer(
        &mut self,
    ) -> Option<(std::sync::Arc<[bool]>, FrameBufferContentCacheKey, String)> {
        let waves = self.user.waves.as_ref()?;
        let cursor = waves.cursor.as_ref()?.to_biguint()?;
        let wave_container = waves.inner.as_waves()?;
        let content = self.frame_buffer_content.clone()?;
        let cache_key = FrameBufferContentCacheKey {
            content: content.clone(),
            cursor_position: cursor.clone(),
        };
        let cached = self
            .frame_buffer_array_cache
            .as_ref()
            .filter(|cache| cache.key == cache_key)
            .cloned();

        let cached = if let Some(cached) = cached {
            cached
        } else {
            let cached = match &content {
                FrameBufferContent::Variable(variable_ref) => build_variable_frame_buffer_cache(
                    wave_container,
                    variable_ref,
                    &cursor,
                    cache_key,
                )?,
                FrameBufferContent::Array { scope_ref, levels } => {
                    if levels.is_empty() {
                        return None;
                    }

                    let sorted_variables =
                        resolve_leaf_scopes_and_variables(wave_container, scope_ref, levels)?;
                    let cached_value =
                        build_cached_variable_value(wave_container, &sorted_variables, &cursor);
                    FrameBufferArrayCache {
                        key: cache_key,
                        cached_value,
                    }
                }
            };
            self.frame_buffer_array_cache = Some(cached.clone());
            cached
        };

        let bits = cached.cached_value.as_ref()?.clone();

        let variable_name = match &content {
            FrameBufferContent::Variable(variable_ref) => variable_ref.full_path_string_no_index(),
            FrameBufferContent::Array { scope_ref, .. } => scope_ref.full_name(),
        };

        Some((bits, cached.key.clone(), variable_name))
    }
}

fn build_cached_variable_value(
    wave_container: &WaveContainer,
    sorted_variables: &[VariableRef],
    cursor: &num::BigUint,
) -> Option<std::sync::Arc<[bool]>> {
    // First pass: sum bit widths for pre-allocation.
    let capacity: usize = sorted_variables
        .iter()
        .filter_map(|v| wave_container.variable_meta(v).ok()?.num_bits)
        .map(|b| b as usize)
        .sum();

    if capacity == 0 {
        return None;
    }

    let mut concat_bits: Vec<bool> = Vec::with_capacity(capacity);

    for var_ref in sorted_variables {
        let Ok(meta) = wave_container.variable_meta(var_ref) else {
            continue;
        };
        let Some(bits) = meta.num_bits else {
            continue;
        };
        let bits = bits as usize;

        // On missing or unavailable signal, pad with zeros to preserve alignment.
        let value = wave_container
            .query_variable(var_ref, cursor)
            .ok()
            .flatten()
            .and_then(|q| q.current)
            .map(|(_, v)| v);

        match value {
            Some(VariableValue::BigUint(v)) => {
                append_biguint_lower_bits_with_left_zero_pad(&v, bits, &mut concat_bits);
            }
            Some(VariableValue::String(s)) => {
                append_str_lower_bits_with_left_zero_pad(&s, bits, &mut concat_bits);
            }
            None => {
                concat_bits.extend(std::iter::repeat_n(false, bits));
            }
        }
    }

    if concat_bits.is_empty() {
        None
    } else {
        Some(concat_bits.into())
    }
}

fn build_variable_frame_buffer_cache(
    wave_container: &WaveContainer,
    variable_ref: &VariableRef,
    cursor: &num::BigUint,
    key: FrameBufferContentCacheKey,
) -> Option<FrameBufferArrayCache> {
    let meta = wave_container.variable_meta(variable_ref).ok()?;
    let word_length = meta.num_bits? as usize;
    let query_result = wave_container
        .query_variable(variable_ref, cursor)
        .ok()
        .flatten()?;
    let (_, value) = query_result.current?;
    let padded: std::sync::Arc<[bool]> = frame_buffer_bits(&value, word_length).into();

    Some(FrameBufferArrayCache {
        key,
        cached_value: Some(padded),
    })
}

fn resolve_leaf_scopes_and_variables(
    wave_container: &WaveContainer,
    scope_ref: &ScopeRef,
    levels: &[ArrayLevel],
) -> Option<Vec<VariableRef>> {
    let (scope_levels, var_level) = levels.split_at(levels.len() - 1);
    let var_level = &var_level[0];

    let mut current_scopes = vec![scope_ref.clone()];
    for level in scope_levels {
        let clamped_first = level.first_index.clamp(level.min_index, level.max_index);
        let clamped_last = level.last_index.clamp(level.min_index, level.max_index);
        let mut next_scopes = Vec::new();
        for scope in &current_scopes {
            let mut selected: Vec<ScopeRef> = wave_container
                .child_scopes(scope)
                .unwrap_or_default()
                .into_iter()
                .filter(|s| {
                    let idx = scope_array_index(s);
                    idx >= clamped_first && idx <= clamped_last
                })
                .collect();
            selected.sort_by_key(scope_array_index);
            next_scopes.extend(selected);
        }
        current_scopes = next_scopes;
    }

    if current_scopes.is_empty() {
        return None;
    }

    let clamped_first = var_level
        .first_index
        .clamp(var_level.min_index, var_level.max_index);
    let clamped_last = var_level
        .last_index
        .clamp(var_level.min_index, var_level.max_index);
    if clamped_first > clamped_last {
        return None;
    }

    let mut sorted_variables = Vec::new();
    for leaf_scope in &current_scopes {
        let mut variables = wave_container.variables_in_scope(leaf_scope);
        variables.sort_by_key(variable_array_index);
        sorted_variables.extend(variables.into_iter().filter(|var_ref| {
            let idx = variable_array_index(var_ref);
            idx >= clamped_first && idx <= clamped_last
        }));
    }

    Some(sorted_variables)
}

/// Analyses the scope hierarchy rooted at `scope_ref` and returns:
/// - `levels`: one `ArrayLevel` per nesting level, where the last level is for variables
/// - `all_leaf_vars`: every variable reachable from the root (for pre-loading)
///
/// Returns `None` when `scope_ref` is not found in the hierarchy.
pub(crate) fn build_frame_buffer_content(
    wave_container: &WaveContainer,
    scope_ref: &ScopeRef,
) -> Option<(Vec<ArrayLevel>, Vec<VariableRef>)> {
    // Probe the hierarchy by following the min-index child at each level.
    // Stop when we reach a leaf scope that has no child scopes.
    let mut levels: Vec<ArrayLevel> = Vec::new();
    let mut probe = scope_ref.clone();
    loop {
        let children = wave_container.child_scopes(&probe).unwrap_or_default();
        if children.is_empty() {
            break;
        }
        let indices: Vec<i64> = children.iter().map(scope_array_index).collect();
        let min_idx = *indices.iter().min().unwrap_or(&0);
        let max_idx = *indices.iter().max().unwrap_or(&0);
        levels.push(ArrayLevel {
            min_index: min_idx,
            max_index: max_idx,
            first_index: min_idx,
            last_index: max_idx,
        });
        probe = children.into_iter().min_by_key(scope_array_index).unwrap();
    }

    // Determine the variable index range from the representative leaf scope.
    let leaf_vars = wave_container.variables_in_scope(&probe);
    let var_indices: Vec<i64> = leaf_vars
        .iter()
        .map(variable_array_index)
        .filter(|&i| i != i64::MAX)
        .collect();
    let (var_min, var_max) = if var_indices.is_empty() {
        (0, 0)
    } else {
        (
            *var_indices.iter().min().unwrap(),
            *var_indices.iter().max().unwrap(),
        )
    };
    levels.push(ArrayLevel {
        min_index: var_min,
        max_index: var_max,
        first_index: var_min,
        last_index: var_max,
    });

    // Walk every path to collect all leaf variables for pre-loading.
    let depth = levels.len().saturating_sub(1);
    let mut leaf_scopes = vec![scope_ref.clone()];
    for _ in 0..depth {
        leaf_scopes = leaf_scopes
            .iter()
            .flat_map(|s| wave_container.child_scopes(s).unwrap_or_default())
            .collect();
    }
    let all_leaf_vars: Vec<VariableRef> = leaf_scopes
        .iter()
        .flat_map(|s| wave_container.variables_in_scope(s))
        .collect();

    Some((levels, all_leaf_vars))
}

fn scope_array_index(scope_ref: &ScopeRef) -> i64 {
    let name = scope_ref.name();
    name.parse::<i64>()
        .ok()
        .or_else(|| {
            name.strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<i64>().ok())
        })
        .unwrap_or(i64::MAX)
}

fn variable_array_index(var_ref: &VariableRef) -> i64 {
    fn parse_index_name(name: &str) -> Option<i64> {
        name.parse::<i64>().ok().or_else(|| {
            name.strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<i64>().ok())
        })
    }

    var_ref
        .index
        .or_else(|| parse_index_name(&var_ref.name))
        .unwrap_or(i64::MAX)
}

fn frame_buffer_bits(value: &VariableValue, word_length: usize) -> Vec<bool> {
    let mut out = Vec::with_capacity(word_length);
    match value {
        VariableValue::BigUint(v) => {
            append_biguint_lower_bits_with_left_zero_pad(v, word_length, &mut out);
        }
        VariableValue::String(v) => {
            append_str_lower_bits_with_left_zero_pad(v, word_length, &mut out);
        }
    }
    out
}

fn append_str_lower_bits_with_left_zero_pad(src: &str, width: usize, out: &mut Vec<bool>) {
    if width == 0 {
        return;
    }

    let start = src.len().saturating_sub(width);
    let suffix = &src.as_bytes()[start..];

    out.extend(std::iter::repeat_n(false, width - suffix.len()));
    out.extend(suffix.iter().map(|b| *b == b'1'));
}

fn append_biguint_lower_bits_with_left_zero_pad(
    value: &num::BigUint,
    width: usize,
    out: &mut Vec<bool>,
) {
    use num::ToPrimitive as _;
    if width == 0 {
        return;
    }

    // Fast path for small values that fit in a u128.
    if let Some(v) = value.to_u128() {
        let value_bits = 128usize.saturating_sub(v.leading_zeros() as usize);
        let actual = width.min(value_bits);
        out.extend(std::iter::repeat_n(false, width - actual));
        if actual > 0 {
            let mut v = v << (127 - (actual - 1));
            for _ in 0..actual {
                out.push(v >> 127 != 0);
                v <<= 1;
            }
        }
        return;
    }

    let digits = value.to_u32_digits();
    let value_bits = value.bits() as usize;
    let actual = width.min(value_bits);
    out.extend(std::iter::repeat_n(false, width - actual));
    let top_limb = actual.saturating_sub(1) / 32;
    // Top limb first: may be partial, so align its MSB before consuming.
    let hi = (actual - 1) % 32;
    let mut limb = digits.get(top_limb).copied().unwrap_or(0) << (31 - hi);
    for _ in 0..=hi {
        out.push(limb & 0x8000_0000 != 0);
        limb <<= 1;
    }
    // Remaining limbs are always full 32-bit words; no conditional needed.
    for limb_idx in (0..top_limb).rev() {
        let mut limb = digits.get(limb_idx).copied().unwrap_or(0);
        for _ in 0..32 {
            out.push(limb & 0x8000_0000 != 0);
            limb <<= 1;
        }
    }
}

fn decode_grayscale_pixels(bits: &[bool], grayscale_bits: usize) -> Vec<Color32> {
    // Default mode: every bool maps directly to black or white.
    if grayscale_bits == 1 {
        return bits
            .iter()
            .map(|&b| {
                let v = u8::from(b) * 255;
                Color32::from_rgb(v, v, v)
            })
            .collect();
    }
    let step = grayscale_bits.max(1);
    let scale = channel_scaler(step);
    let mut chunks = bits.chunks_exact(step);
    let mut out = Vec::with_capacity(bits.len().div_ceil(step));
    for chunk in chunks.by_ref() {
        let gray = apply_scale(bits_to_u16(chunk), scale);
        out.push(Color32::from_rgb(gray, gray, gray));
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let gray = apply_scale(bits_to_u16_padded(rem, 0, step), scale);
        out.push(Color32::from_rgb(gray, gray, gray));
    }
    out
}

fn decode_rgb_pixels(bits: &[bool], r_bits: usize, g_bits: usize, b_bits: usize) -> Vec<Color32> {
    let bits_per_pixel = r_bits + g_bits + b_bits;
    let step = bits_per_pixel.max(1);
    let (r_scale, g_scale, b_scale) = (
        channel_scaler(r_bits),
        channel_scaler(g_bits),
        channel_scaler(b_bits),
    );
    let mut chunks = bits.chunks_exact(step);
    let mut out = Vec::with_capacity(bits.len().div_ceil(step));
    for chunk in chunks.by_ref() {
        let red = apply_scale(bits_to_u16(&chunk[..r_bits]), r_scale);
        let green = apply_scale(bits_to_u16(&chunk[r_bits..r_bits + g_bits]), g_scale);
        let blue = apply_scale(bits_to_u16(&chunk[r_bits + g_bits..]), b_scale);
        out.push(Color32::from_rgb(red, green, blue));
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let red = apply_scale(bits_to_u16_padded(rem, 0, r_bits), r_scale);
        let green = apply_scale(bits_to_u16_padded(rem, r_bits, g_bits), g_scale);
        let blue = apply_scale(bits_to_u16_padded(rem, r_bits + g_bits, b_bits), b_scale);
        out.push(Color32::from_rgb(red, green, blue));
    }
    out
}

fn decode_ycbcr_pixels(
    bits: &[bool],
    y_bits: usize,
    cb_bits: usize,
    cr_bits: usize,
) -> Vec<Color32> {
    let bits_per_pixel = y_bits + cb_bits + cr_bits;
    let step = bits_per_pixel.max(1);
    let (y_scale, cb_scale, cr_scale) = (
        channel_scaler(y_bits),
        channel_scaler(cb_bits),
        channel_scaler(cr_bits),
    );
    let mut chunks = bits.chunks_exact(step);
    let mut out = Vec::with_capacity(bits.len().div_ceil(step));
    for chunk in chunks.by_ref() {
        let y = apply_scale(bits_to_u16(&chunk[..y_bits]), y_scale);
        let cb = apply_scale(bits_to_u16(&chunk[y_bits..y_bits + cb_bits]), cb_scale);
        let cr = apply_scale(bits_to_u16(&chunk[y_bits + cb_bits..]), cr_scale);
        let (red, green, blue) = ycbcr_to_rgb(y, cb, cr);
        out.push(Color32::from_rgb(red, green, blue));
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let y = apply_scale(bits_to_u16_padded(rem, 0, y_bits), y_scale);
        let cb = apply_scale(bits_to_u16_padded(rem, y_bits, cb_bits), cb_scale);
        let cr = apply_scale(bits_to_u16_padded(rem, y_bits + cb_bits, cr_bits), cr_scale);
        let (red, green, blue) = ycbcr_to_rgb(y, cb, cr);
        out.push(Color32::from_rgb(red, green, blue));
    }
    out
}

/// Reads up to `len` bits starting at `start`, zero-padding if out of bounds.
fn bits_to_u16_padded(bits: &[bool], start: usize, len: usize) -> u16 {
    let mut value = 0u16;
    for offset in 0..len {
        value = (value << 1) | u16::from(bits.get(start + offset).copied().unwrap_or(false));
    }
    value
}

/// Reads exactly `bits.len()` bits from a known in-bounds slice — no bounds checks.
fn bits_to_u16(bits: &[bool]) -> u16 {
    let mut value = 0u16;
    for &b in bits {
        value = (value << 1) | u16::from(b);
    }
    value
}

// Precompute a fixed-point reciprocal: (255 << 8) / max_in.
fn channel_scaler(bits: usize) -> u32 {
    if bits == 0 {
        return 0;
    }
    (255u32 << 8) / ((1u32 << bits) - 1)
}

#[inline(always)]
fn apply_scale(value: u16, multiplier: u32) -> u8 {
    ((u32::from(value) * multiplier) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use num::BigUint;

    #[test]
    fn frame_buffer_bits_pads_to_word_length() {
        let bits = frame_buffer_bits(&VariableValue::BigUint(BigUint::from(0b101u8)), 5);
        assert_eq!(bits, vec![false, false, true, false, true]);
    }

    #[test]
    fn frame_buffer_bits_truncates_to_word_length() {
        let bits = frame_buffer_bits(&VariableValue::String("101101".to_string()), 4);
        assert_eq!(bits, vec![true, true, false, true]);
    }

    #[test]
    fn bits_to_u16_padded_reads_and_zero_pads() {
        let bits = vec![true, false, true];
        assert_eq!(bits_to_u16_padded(&bits, 0, 3), 0b101);
        assert_eq!(bits_to_u16_padded(&bits, 1, 4), 0b0100);
    }

    #[test]
    fn decode_grayscale_pixels_uses_bit_groups() {
        let bits = vec![false, false, true, true];
        let pixels = decode_grayscale_pixels(&bits, 2);
        assert_eq!(pixels.len(), 2);
        assert_eq!(pixels[0], Color32::from_rgb(0, 0, 0));
        assert_eq!(pixels[1], Color32::from_rgb(255, 255, 255));
    }

    #[test]
    fn decode_rgb_pixels_supports_different_channel_widths() {
        let bits = vec![
            true, false, false, true, true, false, // R=10 G=01 B=10 with r=2,g=2,b=2
        ];
        let pixels = decode_rgb_pixels(&bits, 2, 2, 2);
        assert_eq!(pixels.len(), 1);
        assert_eq!(pixels[0], Color32::from_rgb(170, 85, 170));
    }

    #[test]
    fn decode_ycbcr_pixels_supports_8bit_channels() {
        let bits = vec![
            true, false, false, false, false, false, false, false, // Y=128
            true, false, false, false, false, false, false, false, // Cb=128
            true, false, false, false, false, false, false, false, // Cr=128
        ];
        let pixels = decode_ycbcr_pixels(&bits, 8, 8, 8);
        assert_eq!(pixels.len(), 1);
        assert_eq!(pixels[0], Color32::from_rgb(128, 128, 128));
    }

    #[test]
    fn variable_array_index_parses_bracketed_name() {
        let var_ref = VariableRef::new(ScopeRef::empty(), "[2]".to_string());
        assert_eq!(variable_array_index(&var_ref), 2);
    }

    #[test]
    fn variable_array_index_parses_plain_numeric_name() {
        let var_ref = VariableRef::new(ScopeRef::empty(), "7".to_string());
        assert_eq!(variable_array_index(&var_ref), 7);
    }

    #[test]
    fn variable_array_index_prefers_explicit_index() {
        let var_ref = VariableRef::new_with_id_and_index(
            ScopeRef::empty(),
            "[2]".to_string(),
            Default::default(),
            Some(9),
        );
        assert_eq!(variable_array_index(&var_ref), 9);
    }

    #[test]
    fn variable_array_index_falls_back_to_max_for_non_numeric_names() {
        let var_ref = VariableRef::new(ScopeRef::empty(), "data".to_string());
        assert_eq!(variable_array_index(&var_ref), i64::MAX);
    }
}
