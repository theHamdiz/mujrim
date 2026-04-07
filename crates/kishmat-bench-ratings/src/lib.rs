//! Approximate strength on public rating scales from Bratko–Kopec suite accuracy.
//!
//! These are **proxies** for regression tracking, not substitutes for:
//! - **[CCRL](https://computerchess.org.uk/ccrl/)** (BayesElo from long engine games)
//! - **Lichess** (Glicko-2 from online games)
//!
//! Anchoring uses this project’s PLAN estimate: **~54.2% BK → ~1963 CCRL 40/15** (historical depth 16, 120s/pos; default bench is now tighter).
//! The BK suite tops out well below Stockfish-level **match** Elo; values above ~2700 on this proxy
//! mean “very high tactical suite score at your bench settings”, not an official list placement.

/// Approximate strength on the **CCRL 40/15**-style scale from BK accuracy (percent correct, 0–100).
///
/// Same piecewise spine as historic KishMat bench: **90% → 2500**, **100% → 2750** on this proxy
/// (not 3000+ — perfect BK still undercounts true top-engine match rating).
#[must_use]
pub fn approx_ccrl_40_15_from_bk_accuracy(accuracy: f64) -> i32 {
    let elo = if accuracy <= 10.0 {
        800.0 + accuracy * 40.0
    } else if accuracy <= 30.0 {
        1200.0 + (accuracy - 10.0) * 20.0
    } else if accuracy <= 50.0 {
        1600.0 + (accuracy - 30.0) * 15.0
    } else if accuracy <= 70.0 {
        1900.0 + (accuracy - 50.0) * 15.0
    } else if accuracy <= 90.0 {
        2200.0 + (accuracy - 70.0) * 15.0
    } else {
        2500.0 + (accuracy - 90.0) * 25.0
    };
    elo.round() as i32
}

/// Rough **Lichess blitz–pool** analogue: same tactical proxy shifted upward.
///
/// Real Lichess ratings mix humans and engines; this offset (~+115) matches the typical gap between
/// a reference engine’s **CCRL 40/15** and **CCRL blitz** list points (order-of-magnitude only).
#[must_use]
pub fn approx_lichess_blitz_from_bk_accuracy(accuracy: f64) -> i32 {
    approx_ccrl_40_15_from_bk_accuracy(accuracy).saturating_add(115)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_anchor_54_2_percent() {
        let ccrl = approx_ccrl_40_15_from_bk_accuracy(54.166666666666664);
        assert!((1963 - ccrl).abs() <= 1);
    }

    #[test]
    fn top_end_capped_for_ccrl_proxy() {
        assert_eq!(approx_ccrl_40_15_from_bk_accuracy(90.0), 2500);
        assert_eq!(approx_ccrl_40_15_from_bk_accuracy(100.0), 2750);
    }

    #[test]
    fn lichess_offset() {
        assert_eq!(
            approx_lichess_blitz_from_bk_accuracy(54.166666666666664),
            approx_ccrl_40_15_from_bk_accuracy(54.166666666666664) + 115
        );
    }
}
