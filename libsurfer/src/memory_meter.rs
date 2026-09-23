//! The status bar's memory meter: this process's resident memory against the
//! machine's physical memory, drawn like Volna's memory pill (without its
//! settings menu). The fill is a true proportion; near the limit the pill
//! takes the error colour.

use std::sync::Mutex;

use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use web_time::{Duration, Instant};

use crate::config::SurferTheme;

/// Fixed so the fill is a true proportion and the bar does not jitter.
const PILL_W: f32 = 112.0;
const PILL_H: f32 = 16.0;
/// The status bar paints every frame; the process is sampled at most this often.
const SAMPLE_EVERY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryUsage {
    /// Resident (physical) memory of this process.
    pub resident: u64,
    /// Physical memory of the machine, where the platform reports it.
    pub total: Option<u64>,
}

impl MemoryUsage {
    /// From this fraction on, the meter warns.
    pub const NEARLY_FULL: f32 = 0.9;

    pub fn fraction(self) -> Option<f32> {
        let total = self.total.filter(|t| *t > 0)?;
        Some((self.resident as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }

    pub fn nearly_full(self) -> bool {
        self.fraction().is_some_and(|f| f >= Self::NEARLY_FULL)
    }

    /// Compact `used / total` in the total's unit, e.g. `1.2 / 64 GiB`.
    pub fn label(self) -> String {
        match self.total {
            Some(total) => {
                let (unit, scale) = byte_unit(total);
                format!(
                    "{} / {} {unit}",
                    short_amount(self.resident as f64 / scale),
                    short_amount(total as f64 / scale)
                )
            }
            None => {
                let (unit, scale) = byte_unit(self.resident);
                format!("{} {unit}", short_amount(self.resident as f64 / scale))
            }
        }
    }

    /// The meter's tooltip.
    pub fn detail(self) -> String {
        let (unit, scale) = byte_unit(self.total.unwrap_or(self.resident));
        let used = self.resident as f64 / scale;
        match (self.total, self.fraction()) {
            (Some(total), Some(fraction)) => format!(
                "Memory: {used:.1} {unit} resident in this process, of {} {unit} physical memory ({:.0}%).",
                short_amount(total as f64 / scale),
                f64::from(fraction) * 100.0
            ),
            _ => format!("Memory: {used:.1} {unit} resident in this process."),
        }
    }
}

fn byte_unit(bytes: u64) -> (&'static str, f64) {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= 1024.0 * MIB {
        ("GiB", 1024.0 * MIB)
    } else {
        ("MiB", MIB)
    }
}

/// One decimal below 10 (`0.3`, `1.5`), whole numbers above; no trailing `.0`.
fn short_amount(value: f64) -> String {
    let text = if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

/// The latest sample, refreshed at most every [`SAMPLE_EVERY`]; `None` where
/// the platform cannot report process memory (the web build).
pub fn current() -> Option<MemoryUsage> {
    static LAST: Mutex<Option<(Instant, Option<MemoryUsage>)>> = Mutex::new(None);
    let mut last = LAST.lock().ok()?;
    let now = Instant::now();
    match *last {
        Some((at, usage)) if now.duration_since(at) < SAMPLE_EVERY => usage,
        _ => {
            let usage = sample();
            *last = Some((now, usage));
            usage
        }
    }
}

/// Unit tests render the status bar into snapshots, which a live number
/// would make nondeterministic; there the meter is hidden.
#[cfg(test)]
fn sample() -> Option<MemoryUsage> {
    None
}

#[cfg(all(not(test), not(target_arch = "wasm32")))]
fn sample() -> Option<MemoryUsage> {
    process_usage()
}

#[cfg(not(target_arch = "wasm32"))]
fn process_usage() -> Option<MemoryUsage> {
    let stats = memory_stats::memory_stats()?;
    Some(MemoryUsage {
        resident: stats.physical_mem as u64,
        total: total_memory(),
    })
}

#[cfg(all(not(test), target_arch = "wasm32"))]
fn sample() -> Option<MemoryUsage> {
    None
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn total_memory() -> Option<u64> {
    // SAFETY: sysconf only reads system configuration values.
    let (pages, size) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    (pages > 0 && size > 0).then(|| pages as u64 * size as u64)
}

#[cfg(not(all(unix, not(target_arch = "wasm32"))))]
fn total_memory() -> Option<u64> {
    None
}

/// Paint the pill: a tinted fill from the left behind the centred label.
pub fn draw(ui: &mut Ui, usage: MemoryUsage, theme: &SurferTheme) {
    let (rect, response) = ui.allocate_exact_size(vec2(PILL_W, PILL_H), Sense::hover());
    let radius = CornerRadius::same((PILL_H / 2.0) as u8);
    let text = theme.primary_ui_color.foreground;
    let full = usage.nearly_full();
    let (tint, border, label) = if full {
        let e = theme.accent_error.background;
        (e.gamma_multiply(0.35), e.gamma_multiply(0.8), e)
    } else {
        (
            theme.accent_info.background.gamma_multiply(0.35),
            theme.border_color,
            text,
        )
    };
    let hovered = response.hovered();
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        radius,
        text.gamma_multiply(if hovered { 0.1 } else { 0.05 }),
    );
    if let Some(fraction) = usage.fraction() {
        // A sliver stays visible once anything is resident.
        let w = (fraction * PILL_W).max(if usage.resident > 0 { 3.0 } else { 0.0 });
        let fill = Rect::from_min_size(rect.min, vec2(w, PILL_H));
        painter.with_clip_rect(fill).rect_filled(rect, radius, tint);
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        usage.label(),
        FontId::monospace(10.5),
        label,
    );
    response.on_hover_text(usage.detail());
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn labels_use_the_total_unit_and_warn_near_the_limit() {
        let u = |resident, total| MemoryUsage { resident, total };
        assert_eq!(u(MIB * 3 / 10, Some(512 * MIB)).label(), "0.3 / 512 MiB");
        assert_eq!(u(1536 * MIB, Some(64 * 1024 * MIB)).label(), "1.5 / 64 GiB");
        assert_eq!(u(1536 * MIB, None).label(), "1.5 GiB");
        assert!(!u(460 * MIB, Some(512 * MIB)).nearly_full());
        assert!(u(461 * MIB, Some(512 * MIB)).nearly_full());
        assert!(!u(461 * MIB, None).nearly_full());
        assert_eq!(
            u(384 * MIB, Some(512 * MIB)).detail(),
            "Memory: 384.0 MiB resident in this process, of 512 MiB physical memory (75%)."
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn this_process_reports_resident_memory_below_the_machine_total() {
        let usage = process_usage().expect("native platforms report process memory");
        assert!(usage.resident > 0);
        if let Some(total) = usage.total {
            assert!(usage.resident < total);
        }
    }
}
