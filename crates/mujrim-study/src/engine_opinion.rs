//! Multi-engine analysis opinions and PV aggregation.

use crate::board_marks::{ArrowRole, BoardArrow, MarkColor, arrows_from_uci_pv};

/// One principal variation reported by an engine.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineLine {
    pub multipv: u32,
    pub score_cp: i32,
    pub depth: i32,
    pub pv: Vec<String>,
    pub nodes: u64,
    pub nps: u64,
}

/// A complete opinion from one engine on a position.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineOpinion {
    pub engine_id: String,
    pub engine_name: String,
    pub color: MarkColor,
    pub lines: Vec<EngineLine>,
}

impl EngineOpinion {
    pub fn best_line(&self) -> Option<&EngineLine> {
        self.lines
            .iter()
            .min_by_key(|line| line.multipv.max(1))
            .or_else(|| self.lines.first())
    }

    /// Board arrows for this engine's variations (best + alternates).
    pub fn arrows(&self, fen: &str, max_plies: usize, max_lines: usize) -> Vec<BoardArrow> {
        let mut arrows = Vec::new();
        for (index, line) in self.lines.iter().take(max_lines.max(1)).enumerate() {
            let role = if index == 0 {
                ArrowRole::EngineBest
            } else {
                ArrowRole::EngineAlternate
            };
            let label = format!("{} · #{}", self.engine_name, line.multipv.max(1));
            if let Ok(mut built) =
                arrows_from_uci_pv(fen, &line.pv, self.color, role, max_plies, Some(&label))
            {
                if index > 0 {
                    for arrow in &mut built {
                        arrow.opacity = Some(role.default_opacity());
                    }
                }
                arrows.extend(built);
            }
        }
        arrows
    }
}

/// Aggregated multi-engine take on one position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiEngineAnalysis {
    pub fen: String,
    pub opinions: Vec<EngineOpinion>,
}

impl MultiEngineAnalysis {
    pub fn new(fen: impl Into<String>) -> Self {
        Self {
            fen: fen.into(),
            opinions: Vec::new(),
        }
    }

    pub fn push_opinion(&mut self, mut opinion: EngineOpinion) {
        if opinion.lines.is_empty() {
            return;
        }
        opinion.lines.sort_by_key(|line| line.multipv.max(1));
        self.opinions.push(opinion);
    }

    pub fn all_arrows(&self, max_plies: usize, max_lines_per_engine: usize) -> Vec<BoardArrow> {
        self.opinions
            .iter()
            .flat_map(|opinion| opinion.arrows(&self.fen, max_plies, max_lines_per_engine))
            .collect()
    }

    /// Consensus best move when a majority of engines agree on the first PV ply.
    pub fn consensus_best_move(&self) -> Option<String> {
        let mut votes: Vec<(String, usize)> = Vec::new();
        for opinion in &self.opinions {
            let Some(best) = opinion.best_line().and_then(|line| line.pv.first()) else {
                continue;
            };
            if let Some((_, count)) = votes.iter_mut().find(|(mv, _)| mv == best) {
                *count += 1;
            } else {
                votes.push((best.clone(), 1));
            }
        }
        votes
            .into_iter()
            .max_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)))
            .map(|(mv, _)| mv)
    }
}

/// Assign palette colors to engines in arrival order.
pub fn color_for_engine_slot(slot: usize) -> MarkColor {
    MarkColor::for_engine_index(slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_opinion(name: &str, slot: usize, first: &str) -> EngineOpinion {
        EngineOpinion {
            engine_id: name.to_owned(),
            engine_name: name.to_owned(),
            color: color_for_engine_slot(slot),
            lines: vec![EngineLine {
                multipv: 1,
                score_cp: 20,
                depth: 12,
                pv: vec![first.to_owned(), "e7e5".to_owned()],
                nodes: 1000,
                nps: 500_000,
            }],
        }
    }

    #[test]
    fn multi_engine_arrows_are_colored_and_stepped() {
        let mut analysis =
            MultiEngineAnalysis::new("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        analysis.push_opinion(sample_opinion("Mujrim", 0, "e2e4"));
        analysis.push_opinion(sample_opinion("Reckless", 1, "d2d4"));
        let arrows = analysis.all_arrows(4, 1);
        assert_eq!(arrows.len(), 4);
        assert_eq!(arrows[0].color, MarkColor::Green);
        assert_eq!(arrows[2].color, MarkColor::Blue);
        assert_eq!(arrows[0].step, Some(1));
        assert_eq!(arrows[1].step, Some(2));
    }

    #[test]
    fn consensus_picks_majority_first_move() {
        let mut analysis =
            MultiEngineAnalysis::new("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        analysis.push_opinion(sample_opinion("A", 0, "e2e4"));
        analysis.push_opinion(sample_opinion("B", 1, "e2e4"));
        analysis.push_opinion(sample_opinion("C", 2, "d2d4"));
        assert_eq!(analysis.consensus_best_move().as_deref(), Some("e2e4"));
    }
}
