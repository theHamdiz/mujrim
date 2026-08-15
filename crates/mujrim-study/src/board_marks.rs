//! Board annotation marks: stepped, numbered, multi-colored arrows.

use types::Square;

/// Semantic role of a board arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrowRole {
    User,
    LastMove,
    Ponder,
    EngineBest,
    EngineAlternate,
    Coach,
    Opening,
    Gambit,
}

impl ArrowRole {
    /// Default opacity for the role (0.0–1.0).
    pub const fn default_opacity(self) -> f32 {
        match self {
            Self::Ponder => 0.35,
            Self::EngineAlternate => 0.55,
            Self::Opening | Self::Gambit => 0.70,
            Self::Coach => 0.75,
            Self::LastMove | Self::EngineBest | Self::User => 0.85,
        }
    }

    /// Whether this arrow should render a step number badge.
    pub const fn shows_step(self) -> bool {
        matches!(
            self,
            Self::EngineBest | Self::EngineAlternate | Self::Coach | Self::Opening | Self::Gambit
        )
    }
}

/// Discrete palette index shared by study logic and the UI renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkColor {
    Orange,
    Green,
    Blue,
    Red,
    Purple,
    Cyan,
    Gold,
    Gray,
}

impl MarkColor {
    pub const ENGINE_PALETTE: [Self; 8] = [
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Cyan,
        Self::Gold,
        Self::Orange,
        Self::Red,
        Self::Gray,
    ];

    pub fn for_engine_index(index: usize) -> Self {
        Self::ENGINE_PALETTE[index % Self::ENGINE_PALETTE.len()]
    }
}

/// A single annotation arrow on the board.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardArrow {
    pub from: Square,
    pub to: Square,
    pub color: MarkColor,
    pub role: ArrowRole,
    /// 1-based step along a variation (None = unnumbered).
    pub step: Option<u8>,
    /// Engine or coach label shown in analysis panels.
    pub label: Option<String>,
    /// Override opacity; None uses [`ArrowRole::default_opacity`].
    pub opacity: Option<f32>,
}

impl BoardArrow {
    pub fn new(from: Square, to: Square, color: MarkColor, role: ArrowRole) -> Self {
        Self {
            from,
            to,
            color,
            role,
            step: None,
            label: None,
            opacity: None,
        }
    }

    pub fn with_step(mut self, step: u8) -> Self {
        self.step = Some(step.max(1));
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = Some(opacity.clamp(0.05, 1.0));
        self
    }

    pub fn resolved_opacity(&self) -> f32 {
        self.opacity.unwrap_or_else(|| self.role.default_opacity())
    }
}

/// Build stepped arrows for a UCI PV sequence from `fen`.
pub fn arrows_from_uci_pv(
    fen: &str,
    pv: &[String],
    color: MarkColor,
    role: ArrowRole,
    max_plies: usize,
    label: Option<&str>,
) -> Result<Vec<BoardArrow>, String> {
    arrows_from_uci_pv_offset(fen, pv, color, role, max_plies, label, 0)
}

/// Like [`arrows_from_uci_pv`], numbering steps from `step_offset + 1`.
pub fn arrows_from_uci_pv_offset(
    fen: &str,
    pv: &[String],
    color: MarkColor,
    role: ArrowRole,
    max_plies: usize,
    label: Option<&str>,
    step_offset: u8,
) -> Result<Vec<BoardArrow>, String> {
    types::init();
    let mut board = types::Board::from_fen(fen)?;
    let mut arrows = Vec::new();
    let limit = max_plies.max(1).min(pv.len());
    for (index, uci) in pv.iter().take(limit).enumerate() {
        let mv = board
            .generate_legal_moves()
            .into_iter()
            .find(|candidate| candidate.to_uci() == *uci)
            .copied()
            .ok_or_else(|| format!("illegal PV move '{uci}' at ply {}", index + 1))?;
        let step = step_offset.saturating_add((index + 1) as u8);
        let mut arrow = BoardArrow::new(mv.from, mv.to, color, role).with_step(step);
        if let Some(label) = label {
            arrow = arrow.with_label(format!("{label} · {step}"));
        }
        arrows.push(arrow);
        board.make_move(mv);
    }
    Ok(arrows)
}

/// Numbered coaching disc drawn on arrow tips.
pub fn step_badge_svg(step: u8) -> String {
    let digits = if step >= 10 {
        format!(
            r##"<g transform="translate(-5 0)">{}</g><g transform="translate(5 0)">{}</g>"##,
            digit_paths(step / 10),
            digit_paths(step % 10)
        )
    } else {
        digit_paths(step).to_owned()
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
  <circle cx="16" cy="16" r="14.6" fill="#14141a" stroke="#f4f4f7" stroke-width="2.4"/>
  {digits}
</svg>"##
    )
}

fn digit_paths(digit: u8) -> String {
    // Seven-segment bars stay readable at badge size. Filled blobs were
    // painting as unrelated shapes after the Inter font switch.
    const A: &str = r##"<path fill="#fff" d="M11.1 7.5h9.8v2.3H11.1z"/>"##;
    const B: &str = r##"<path fill="#fff" d="M19.6 8.2h2.3v6.6h-2.3z"/>"##;
    const C: &str = r##"<path fill="#fff" d="M19.6 17.2h2.3v6.6h-2.3z"/>"##;
    const D: &str = r##"<path fill="#fff" d="M11.1 22.2h9.8v2.3H11.1z"/>"##;
    const E: &str = r##"<path fill="#fff" d="M10.1 17.2h2.3v6.6h-2.3z"/>"##;
    const F: &str = r##"<path fill="#fff" d="M10.1 8.2h2.3v6.6h-2.3z"/>"##;
    const G: &str = r##"<path fill="#fff" d="M11.1 14.85h9.8v2.3H11.1z"/>"##;
    let mask = match digit % 10 {
        0 => 0b1111110,
        1 => 0b0110000,
        2 => 0b1101101,
        3 => 0b1111001,
        4 => 0b0110011,
        5 => 0b1011011,
        6 => 0b1011111,
        7 => 0b1110000,
        8 => 0b1111111,
        _ => 0b1111011,
    };
    [A, B, C, D, E, F, G]
        .into_iter()
        .enumerate()
        .filter(|(index, _)| mask & (1 << (6 - index)) != 0)
        .map(|(_, segment)| segment)
        .collect()
}

/// Last-move highlight as a solid arrow.
pub fn last_move_arrow(from: Square, to: Square) -> BoardArrow {
    BoardArrow::new(from, to, MarkColor::Gold, ArrowRole::LastMove)
}

/// Ponder suggestion as a semi-transparent arrow.
pub fn ponder_arrow(from: Square, to: Square) -> BoardArrow {
    BoardArrow::new(from, to, MarkColor::Cyan, ArrowRole::Ponder).with_opacity(0.35)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pv_arrows_are_step_numbered() {
        let arrows = arrows_from_uci_pv(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            &["e2e4".into(), "e7e5".into(), "g1f3".into()],
            MarkColor::Green,
            ArrowRole::EngineBest,
            8,
            Some("SF"),
        )
        .unwrap();
        assert_eq!(arrows.len(), 3);
        assert_eq!(arrows[0].step, Some(1));
        assert_eq!(arrows[1].step, Some(2));
        assert_eq!(arrows[2].step, Some(3));
        assert!(arrows[0].label.as_deref().unwrap().contains("SF"));
        assert!(arrows.iter().all(|a| a.role == ArrowRole::EngineBest));
        let offset = arrows_from_uci_pv_offset(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            &["e2e4".into()],
            MarkColor::Orange,
            ArrowRole::Gambit,
            1,
            None,
            2,
        )
        .unwrap();
        assert_eq!(offset[0].step, Some(3));
        let badge = step_badge_svg(3);
        assert!(badge.contains("<circle"));
        assert!(badge.contains("path"));
        assert!(!badge.contains("> <"));
        assert!(!badge.contains("<text"));
        let one = step_badge_svg(1);
        let eight = step_badge_svg(8);
        assert!(eight.matches("<path").count() > one.matches("<path").count());
        assert_eq!(digit_paths(1).matches("<path").count(), 2);
        assert_eq!(digit_paths(8).matches("<path").count(), 7);
    }

    #[test]
    fn ponder_arrows_are_translucent() {
        let arrow = ponder_arrow(Square::from_index(12), Square::from_index(28));
        assert!((arrow.resolved_opacity() - 0.35).abs() < f32::EPSILON);
        assert_eq!(arrow.role, ArrowRole::Ponder);
    }

    #[test]
    fn engine_palette_cycles() {
        assert_eq!(MarkColor::for_engine_index(0), MarkColor::Green);
        assert_eq!(MarkColor::for_engine_index(8), MarkColor::Green);
        assert_ne!(
            MarkColor::for_engine_index(0),
            MarkColor::for_engine_index(1)
        );
    }
}
