//! Cross-view X-axis (time) synchronization.
//!
//! The Konata "Synchronize scroll" option ties the time axis of every participating Konata tile
//! together with the primary waveform viewport so that they all show the same raw-time window.
//! The coupling is bidirectional: zooming or panning any participant drives the others.
//!
//! A shared *time window* (in raw trace ticks) is the single source of truth. Each frame, every
//! participant reports its currently visible window; whichever one the user moved becomes the
//! source, the shared window is updated, and the remaining participants are told to adopt it.
//!
//! The arbitration in [`ViewportSyncState::arbitrate`] is deliberately free of any egui or
//! viewport dependency so it can be unit-tested in isolation. The caller ([`crate::SystemState`])
//! reads the concrete viewports, feeds their windows in, and applies the returned targets —
//! reporting the *actual* window each viewport settled on (after its own zoom/edge clamping) so a
//! clamped participant never re-broadcasts a window it could not honour.

use std::collections::HashMap;

use crate::konata::KonataTileId;

/// A view whose time axis takes part in synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SyncParticipant {
    /// A waveform viewport, identified by its index in `WaveData::viewports`.
    Waveform(usize),
    /// A Konata pipeline tile.
    Konata(KonataTileId),
}

/// A visible time window `[left, right)` in raw trace ticks.
pub(crate) type TimeWindow = (f64, f64);

#[derive(Debug, Clone, Copy)]
struct Applied {
    window: TimeWindow,
    generation: u64,
}

/// Persistent, runtime-only state backing one synchronization group.
#[derive(Debug, Default)]
pub(crate) struct ViewportSyncState {
    /// The authoritative shared window, once any participant has established one.
    window: Option<TimeWindow>,
    /// Bumped every time the shared window changes, so participants can tell they are stale.
    generation: u64,
    /// The window last written to (or read from) each participant, and the generation at which
    /// that happened. Lets us distinguish a user-driven change from our own last write.
    applied: HashMap<SyncParticipant, Applied>,
}

/// Two window edges are "the same" when they differ by less than this fraction of the window
/// width. Keeps floating-point round-trips (relative<->absolute on the waveform, tick/px on
/// Konata) from being mistaken for user input, while staying well below one pixel of real motion.
const REL_TOLERANCE: f64 = 1e-6;
const ABS_TOLERANCE: f64 = 1e-2;

fn windows_differ(a: TimeWindow, b: TimeWindow) -> bool {
    let width = (a.1 - a.0).abs().max((b.1 - b.0).abs());
    let tol = (width * REL_TOLERANCE).max(ABS_TOLERANCE);
    (a.0 - b.0).abs() > tol || (a.1 - b.1).abs() > tol
}

impl ViewportSyncState {
    /// Decide, for one frame, which participant drove a change and which participants must adopt
    /// the shared window as a result.
    ///
    /// `participants` lists every currently-eligible view with the window it presently shows, in a
    /// stable priority order (the first entry wins ties when two views changed in the same frame).
    /// `primary` names the view to seed the shared window from the first time the group forms.
    ///
    /// Returns the participants that should be moved and the window to move them to. The caller
    /// must apply each and then call [`Self::record_applied`] with the window actually achieved.
    pub(crate) fn arbitrate(
        &mut self,
        participants: &[(SyncParticipant, TimeWindow)],
        primary: Option<SyncParticipant>,
    ) -> Vec<(SyncParticipant, TimeWindow)> {
        // Forget participants that are no longer eligible (tile closed, sync disabled, ...).
        self.applied
            .retain(|key, _| participants.iter().any(|(p, _)| p == key));

        if participants.is_empty() {
            self.window = None;
            self.generation = 0;
            return Vec::new();
        }

        // A participant is the source of change if its current window drifted from what we last
        // wrote to (or read from) it.
        let source = participants.iter().copied().find(|(p, window)| {
            self.applied
                .get(p)
                .is_some_and(|applied| windows_differ(applied.window, *window))
        });

        if let Some((key, window)) = source {
            self.generation += 1;
            self.window = Some(window);
            self.applied.insert(
                key,
                Applied {
                    window,
                    generation: self.generation,
                },
            );
        } else if self.window.is_none() && participants.len() >= 2 {
            // First time two or more views coexist: adopt the primary view's window as the baseline
            // so the others align to it, rather than waiting for the user to move something. A lone
            // participant is left alone so it does not dominate a view that joins later.
            let seed = primary
                .and_then(|pref| participants.iter().copied().find(|(p, _)| *p == pref))
                .or_else(|| participants.first().copied());
            if let Some((key, window)) = seed {
                self.generation += 1;
                self.window = Some(window);
                self.applied.insert(
                    key,
                    Applied {
                        window,
                        generation: self.generation,
                    },
                );
            }
        }

        let Some(window) = self.window else {
            return Vec::new();
        };
        let generation = self.generation;

        participants
            .iter()
            .filter(|(p, _)| {
                self.applied
                    .get(p)
                    .is_none_or(|applied| applied.generation != generation)
            })
            .map(|(p, _)| (*p, window))
            .collect()
    }

    /// Record the window a participant actually settled on after being asked to adopt the shared
    /// window. Storing the clamped result (rather than the requested window) prevents a view that
    /// cannot reach the requested zoom or scroll extent from re-broadcasting a spurious change on
    /// the next frame.
    pub(crate) fn record_applied(&mut self, participant: SyncParticipant, window: TimeWindow) {
        self.applied.insert(
            participant,
            Applied {
                window,
                generation: self.generation,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(idx: usize) -> SyncParticipant {
        SyncParticipant::Waveform(idx)
    }

    fn konata(id: u64) -> SyncParticipant {
        SyncParticipant::Konata(KonataTileId(id))
    }

    #[test]
    fn seeds_shared_window_from_primary_on_first_frame() {
        let mut state = ViewportSyncState::default();
        let targets = state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (40.0, 60.0))],
            Some(wave(0)),
        );
        // The waveform is the primary and keeps its window; the Konata tile must adopt it.
        assert_eq!(targets, vec![(konata(1), (0.0, 100.0))]);
    }

    #[test]
    fn stable_once_everyone_matches() {
        let mut state = ViewportSyncState::default();
        let participants = [(wave(0), (0.0, 100.0)), (konata(1), (40.0, 60.0))];
        let targets = state.arbitrate(&participants, Some(wave(0)));
        // Apply the target exactly.
        state.record_applied(konata(1), (0.0, 100.0));
        assert_eq!(targets, vec![(konata(1), (0.0, 100.0))]);

        // Next frame, both show the shared window -> nothing to do.
        let targets = state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (0.0, 100.0))],
            Some(wave(0)),
        );
        assert!(targets.is_empty());
    }

    /// Drive `state` to a settled baseline where both views show `(0, 100)`.
    fn settled_baseline() -> ViewportSyncState {
        let mut state = ViewportSyncState::default();
        let targets = state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (0.0, 100.0))],
            Some(wave(0)),
        );
        for (participant, window) in targets {
            state.record_applied(participant, window);
        }
        // Confirm steady state: no further movement.
        let targets = state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (0.0, 100.0))],
            Some(wave(0)),
        );
        assert!(targets.is_empty());
        state
    }

    #[test]
    fn konata_pan_drives_waveform() {
        let mut state = settled_baseline();
        // User pans the Konata tile.
        let targets = state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (20.0, 120.0))],
            Some(wave(0)),
        );
        assert_eq!(targets, vec![(wave(0), (20.0, 120.0))]);
    }

    #[test]
    fn waveform_zoom_drives_konata() {
        let mut state = settled_baseline();
        let targets = state.arbitrate(
            &[(wave(0), (25.0, 75.0)), (konata(1), (0.0, 100.0))],
            Some(wave(0)),
        );
        assert_eq!(targets, vec![(konata(1), (25.0, 75.0))]);
    }

    #[test]
    fn clamped_participant_does_not_rebroadcast() {
        let mut state = settled_baseline();

        // Waveform zooms very tight; Konata cannot zoom that far and clamps to a wider window.
        let targets = state.arbitrate(
            &[(wave(0), (49.0, 51.0)), (konata(1), (0.0, 100.0))],
            Some(wave(0)),
        );
        assert_eq!(targets, vec![(konata(1), (49.0, 51.0))]);
        // Konata only reaches (40, 60) after clamping.
        state.record_applied(konata(1), (40.0, 60.0));

        // The clamped window must not be mistaken for a user pan next frame.
        let targets = state.arbitrate(
            &[(wave(0), (49.0, 51.0)), (konata(1), (40.0, 60.0))],
            Some(wave(0)),
        );
        assert!(targets.is_empty());
    }

    #[test]
    fn a_lone_participant_establishes_no_shared_window() {
        let mut state = ViewportSyncState::default();
        let targets = state.arbitrate(&[(konata(1), (10.0, 20.0))], Some(konata(1)));
        assert!(targets.is_empty());
        assert!(state.window.is_none());

        // A waveform joining later seeds from whichever is primary, not from the stale lone view.
        let targets = state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (10.0, 20.0))],
            Some(konata(1)),
        );
        assert_eq!(targets, vec![(wave(0), (10.0, 20.0))]);
    }

    #[test]
    fn dropping_a_participant_forgets_it() {
        let mut state = ViewportSyncState::default();
        state.arbitrate(
            &[(wave(0), (0.0, 100.0)), (konata(1), (0.0, 100.0))],
            Some(wave(0)),
        );
        state.record_applied(konata(1), (0.0, 100.0));
        assert!(state.applied.contains_key(&konata(1)));

        state.arbitrate(&[(wave(0), (0.0, 100.0))], Some(wave(0)));
        assert!(!state.applied.contains_key(&konata(1)));
    }
}
