//! Helpers for the Tournament Studio Results panel.

use crate::tournament_live::{LiveTournamentSnapshot, StandingRow};

/// Whether the Results panel should render standings/game tables.
pub fn panel_open(snapshot: &LiveTournamentSnapshot, forced: bool) -> bool {
    forced || snapshot.show_results_panel
}

pub fn standings_ready(standings: &[StandingRow]) -> bool {
    !standings.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_open_respects_forced_flag() {
        let snap = LiveTournamentSnapshot::default();
        assert!(!panel_open(&snap, false));
        assert!(panel_open(&snap, true));
        let mut open = snap;
        open.show_results_panel = true;
        assert!(panel_open(&open, false));
    }

    #[test]
    fn standings_ready_is_false_when_empty() {
        assert!(!standings_ready(&[]));
    }
}
