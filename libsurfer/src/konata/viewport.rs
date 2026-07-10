use serde::{Deserialize, Serialize};

const MIN_ROW_HEIGHT: f64 = 1.0 / 1024.0;
const MAX_ROW_HEIGHT: f64 = 48.0;
const MIN_PX_PER_TICK: f64 = 1.0 / 4096.0;
const MAX_PX_PER_TICK: f64 = 4096.0;

/// A precision-preserving two-dimensional pipeline viewport.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KonataViewport {
    pub left_tick: i64,
    pub left_frac: f64,
    pub px_per_tick: f64,
    pub top_visible_row: f64,
    pub row_height_px: f64,
}

impl Default for KonataViewport {
    fn default() -> Self {
        Self {
            left_tick: 0,
            left_frac: 0.0,
            px_per_tick: 16.0,
            top_visible_row: 0.0,
            row_height_px: 24.0,
        }
    }
}

impl KonataViewport {
    #[must_use]
    pub fn interpolate(start: Self, target: Self, factor: f64) -> Self {
        let factor = factor.clamp(0.0, 1.0);
        if factor >= 1.0 {
            return target;
        }
        let mix = |from: f64, to: f64| from + (to - from) * factor;
        let mut viewport = start;
        viewport.set_left(mix(start.left_value(), target.left_value()));
        viewport.px_per_tick = mix(start.px_per_tick, target.px_per_tick);
        viewport.top_visible_row = mix(start.top_visible_row, target.top_visible_row);
        viewport.row_height_px = mix(start.row_height_px, target.row_height_px);
        viewport
    }

    #[must_use]
    pub fn tick_to_x(self, tick: u64, canvas_left: f32) -> f32 {
        let delta = i128::from(tick) - i128::from(self.left_tick);
        canvas_left + ((delta as f64 - self.left_frac) * self.px_per_tick) as f32
    }

    #[must_use]
    pub fn x_to_tick(self, x: f32, canvas_left: f32) -> f64 {
        self.left_tick as f64 + self.left_frac + f64::from(x - canvas_left) / self.px_per_tick
    }

    #[must_use]
    pub fn visible_row_to_y(self, row: f64, canvas_top: f32) -> f32 {
        canvas_top + ((row - self.top_visible_row) * self.row_height_px) as f32
    }

    #[must_use]
    pub fn y_to_visible_row(self, y: f32, canvas_top: f32) -> f64 {
        self.top_visible_row + f64::from(y - canvas_top) / self.row_height_px
    }

    pub fn pan_pixels(&mut self, delta_x: f64, delta_y: f64) {
        self.set_left(self.left_value() - delta_x / self.px_per_tick);
        self.top_visible_row -= delta_y / self.row_height_px;
    }

    pub fn scroll_rows(&mut self, rows: f64, horizontal_ticks: f64) {
        self.top_visible_row += rows;
        self.set_left(self.left_value() + horizontal_ticks);
    }

    pub fn zoom_at(&mut self, factor: f64, anchor_x: f64, anchor_y: f64) {
        let factor = factor.clamp(0.01, 100.0);
        let anchor_tick = self.left_value() + anchor_x / self.px_per_tick;
        let anchor_row = self.top_visible_row + anchor_y / self.row_height_px;
        self.px_per_tick = (self.px_per_tick * factor).clamp(MIN_PX_PER_TICK, MAX_PX_PER_TICK);
        self.row_height_px = (self.row_height_px * factor).clamp(MIN_ROW_HEIGHT, MAX_ROW_HEIGHT);
        self.set_left(anchor_tick - anchor_x / self.px_per_tick);
        self.top_visible_row = anchor_row - anchor_y / self.row_height_px;
    }

    pub fn align_row(&mut self, begin: u64, visible_row: usize) {
        self.left_tick = i64::try_from(begin).unwrap_or(i64::MAX);
        self.left_frac = 0.0;
        self.top_visible_row = visible_row as f64;
    }

    pub fn set_left_tick(&mut self, tick: i64) {
        self.left_tick = tick;
        self.left_frac = 0.0;
    }

    #[must_use]
    pub fn left_value(self) -> f64 {
        self.left_tick as f64 + self.left_frac
    }

    fn set_left(&mut self, value: f64) {
        let whole = value.floor().clamp(i64::MIN as f64, i64::MAX as f64);
        self.left_tick = whole as i64;
        self.left_frac = value - whole;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_ticks_map_after_integer_subtraction() {
        let viewport = KonataViewport {
            left_tick: 1_000_000_000_000,
            left_frac: 0.25,
            px_per_tick: 32.0,
            ..Default::default()
        };
        assert_eq!(viewport.tick_to_x(1_000_000_000_001, 10.0), 34.0);
    }

    #[test]
    fn zoom_preserves_pointer_anchor() {
        let mut viewport = KonataViewport::default();
        viewport.left_tick = 100;
        viewport.top_visible_row = 20.0;
        let before_tick = viewport.x_to_tick(240.0, 0.0);
        let before_row = viewport.y_to_visible_row(120.0, 0.0);
        viewport.zoom_at(2.0, 240.0, 120.0);
        assert!((viewport.x_to_tick(240.0, 0.0) - before_tick).abs() < 1e-9);
        assert!((viewport.y_to_visible_row(120.0, 0.0) - before_row).abs() < 1e-9);
    }

    #[test]
    fn panning_can_move_before_tick_zero() {
        let mut viewport = KonataViewport::default();
        viewport.pan_pixels(160.0, 0.0);
        assert_eq!(viewport.left_tick, -10);
        assert_eq!(viewport.left_frac, 0.0);
    }

    #[test]
    fn interpolation_preserves_fractional_left_and_exact_endpoints() {
        let mut start = KonataViewport::default();
        start.left_tick = -3;
        start.left_frac = 0.25;
        let mut target = start;
        target.left_tick = 101;
        target.left_frac = 0.75;
        target.px_per_tick = 32.0;
        target.top_visible_row = 42.0;
        target.row_height_px = 12.0;

        assert_eq!(KonataViewport::interpolate(start, target, 0.0), start);
        assert_eq!(KonataViewport::interpolate(start, target, 1.0), target);
        let middle = KonataViewport::interpolate(start, target, 0.5);
        assert!((middle.left_value() - 49.5).abs() < 1e-9);
        assert_eq!(middle.top_visible_row, 21.0);
    }
}
