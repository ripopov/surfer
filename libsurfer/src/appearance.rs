//! Layout rules for an IDE: compact rows, quiet chrome, and distinct input surfaces.
use egui::{CornerRadius, Stroke, Style, Ui, vec2};

pub(crate) fn configure(style: &mut Style) {
    style.spacing.item_spacing = vec2(6.0, 3.0);
    style.spacing.button_padding = vec2(6.0, 3.0);
    style.spacing.interact_size = vec2(20.0, 20.0);
    style.spacing.indent = 16.0;
}

/// Rows are content, not individual buttons. Keep selection rectangular and
/// preserve one baseline for disclosure arrows, icons and text.
pub(crate) fn list(ui: &mut Ui) {
    let style = ui.style_mut();
    style.spacing.item_spacing.y = 0.0;
    style.spacing.button_padding = vec2(4.0, 2.0);
    style.spacing.interact_size.y = 20.0;
    style.spacing.indent = 16.0;
    style.visuals.selection.stroke.color = style.visuals.text_color();
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::ZERO;
        widget.bg_stroke = Stroke::NONE;
        widget.expansion = 0.0;
    }
}

/// Virtual scrolling must budget the same height as the real row buttons.
pub(crate) fn row_height(ui: &Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Body)
        .max(ui.text_style_height(&egui::TextStyle::Monospace))
        + ui.spacing().button_padding.y * 2.0)
        .max(ui.spacing().interact_size.y)
        .ceil()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_rows_match_rendered_rows_at_normal_and_large_font_sizes() {
        for size in [14.0, 18.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let mut reserved = 0.0;
            for _ in 0..2 {
                rects.clear();
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.style_mut()
                        .text_styles
                        .insert(egui::TextStyle::Body, egui::FontId::proportional(size));
                    list(ui);
                    reserved = row_height(ui);
                    egui::ScrollArea::vertical().show_rows(ui, reserved, 100, |ui, range| {
                        for row in range {
                            let response = ui.horizontal(|ui| {
                                ui.add(
                                    egui::Button::selectable(
                                        row == 1,
                                        egui::RichText::new(format!("signal_{row}"))
                                            .font(egui::FontId::proportional(size)),
                                    )
                                    .min_size(vec2(0.0, reserved)),
                                )
                            });
                            rects.push(response.response.rect);
                        }
                    });
                });
                output.textures_delta.clear();
            }
            assert!(rects.len() > 2);
            for pair in rects.windows(2) {
                assert!(
                    (pair[0].height() - reserved).abs() < 0.1,
                    "font={size}, actual={}, reserved={reserved}",
                    pair[0].height()
                );
                assert!((pair[1].top() - pair[0].top() - reserved).abs() < 0.1);
            }
        }
    }
}
