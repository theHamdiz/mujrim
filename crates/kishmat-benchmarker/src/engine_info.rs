//! Engine Info — NNUE network and search technique introspection.
//!
//! Displays which NNUE network is active and what search techniques
//! are enabled, with their tuned parameter values.

use search::SearchParams;

// ═══════════════════════════════════════════════════════════════════
// NNUE Network Info
// ═══════════════════════════════════════════════════════════════════

/// Information about the active NNUE network.
#[derive(Clone, Debug)]
pub struct NnueInfo {
    pub name: String,
    pub format: String,
    pub architecture: String,
    pub hidden_size: usize,
    pub num_buckets: usize,
    pub quantization: String,
    pub file_size: String,
}

impl NnueInfo {
    /// Detect the current embedded NNUE network info.
    pub fn detect() -> Self {
        use eval::nnue::{HIDDEN, NUM_BUCKETS};

        Self {
            name: "Embedded Akimbo 1024".into(),
            format: "Akimbo (raw repr(C) struct)".into(),
            architecture: format!("768→{}×2→1 SCReLU w/ king buckets", HIDDEN),
            hidden_size: HIDDEN,
            num_buckets: NUM_BUCKETS,
            quantization: "QA=255 (features), QB=64 (output)".into(),
            file_size: format!(
                "{:.1} MB",
                std::mem::size_of::<eval::nnue::Network>() as f64 / 1_048_576.0
            ),
        }
    }

    /// Build display info from a runtime-loaded network descriptor.
    pub fn from_runtime(info: eval::nnue::NnueNetworkInfo) -> Self {
        let file_size = if info.file_size == 0 {
            format!(
                "{:.1} MB",
                std::mem::size_of::<eval::nnue::Network>() as f64 / 1_048_576.0
            )
        } else {
            format!("{:.1} MB", info.file_size as f64 / 1_048_576.0)
        };

        Self {
            name: info.name,
            format: info.format.to_string(),
            architecture: info.architecture,
            hidden_size: info.hidden_size,
            num_buckets: info.num_buckets,
            quantization: "Engine-native quantization".into(),
            file_size,
        }
    }

    /// Format as display lines for the benchmark header.
    pub fn display_lines(&self) -> Vec<String> {
        vec![
            format!("    Network:    {} ({})", self.name, self.format),
            format!("    Arch:       {}", self.architecture),
            format!(
                "    Hidden:     {} × 2 perspectives, {} king buckets",
                self.hidden_size, self.num_buckets
            ),
            format!("    Quant:      {}", self.quantization),
            format!("    Size:       {}", self.file_size),
        ]
    }
}

// ═══════════════════════════════════════════════════════════════════
// Search Technique Info
// ═══════════════════════════════════════════════════════════════════

/// A single search technique with its configuration.
#[derive(Clone, Debug)]
pub struct Technique {
    pub name: String,
    pub enabled: bool,
    pub details: String,
}

/// Introspect all search techniques from the active `SearchParams`.
pub fn detect_techniques(params: &SearchParams) -> Vec<Technique> {
    vec![
        Technique {
            name: "Aspiration Windows".into(),
            enabled: true,
            details: format!(
                "±{} cp (depth ≥ 5, eval-adaptive)",
                params.aspiration_window
            ),
        },
        Technique {
            name: "Null-Move Pruning".into(),
            enabled: true,
            details: format!(
                "R = {} + depth/{} + min((eval-β)/{}, {}), verify depth > 12",
                params.nmp_base, params.nmp_depth_div, params.nmp_eval_div, params.nmp_eval_max,
            ),
        },
        Technique {
            name: "Reverse Futility Pruning".into(),
            enabled: true,
            details: format!(
                "margin = {}×depth − {} (improving), depth ≤ 8, TT-guarded",
                params.rfp_mul, params.rfp_improving_bonus,
            ),
        },
        Technique {
            name: "Razoring".into(),
            enabled: true,
            details: format!(
                "margin = {} + {}×depth², depth ≤ 3 → drop to QS",
                params.razoring_base, params.razoring_depth_mul,
            ),
        },
        Technique {
            name: "Late Move Reductions".into(),
            enabled: true,
            details: format!(
                "base={:.2} + ln(d)×ln(m)/{:.2}, cut-node +{}, stat-score adj /{}",
                params.lmr_base, params.lmr_divisor, params.lmr_cut_node_bonus, params.hist_lmr_div,
            ),
        },
        Technique {
            name: "Late Move Pruning".into(),
            enabled: true,
            details: format!(
                "(3 + d²) / (2 − improving), depth ≤ {}",
                params.lmp_depth_limit
            ),
        },
        Technique {
            name: "Futility Pruning".into(),
            enabled: true,
            details: format!(
                "margin = {}×depth − {}, depth ≤ {}",
                params.futility_mul, params.futility_improving_bonus, params.futility_depth_limit,
            ),
        },
        Technique {
            name: "History Pruning".into(),
            enabled: true,
            details: format!(
                "stat_score < {}×depth, depth ≤ {}",
                params.hist_prune_margin, params.hist_prune_depth_limit,
            ),
        },
        Technique {
            name: "SEE Pruning".into(),
            enabled: true,
            details: format!(
                "captures {}×depth, quiets {}×depth + stat/300, depth ≤ {}",
                params.see_capture_margin, params.see_quiet_margin, params.see_prune_depth_limit,
            ),
        },
        Technique {
            name: "Singular Extensions".into(),
            enabled: true,
            details: format!(
                "margin = {}×depth, depth ≥ 8, double if < β−40",
                params.se_margin_mul,
            ),
        },
        Technique {
            name: "Check Extensions".into(),
            enabled: true,
            details: "budget = 1.5× nominal depth".into(),
        },
        Technique {
            name: "IIR (Internal Iterative Reduction)".into(),
            enabled: true,
            details: "depth −1 when no TT move, depth ≥ 4".into(),
        },
        Technique {
            name: "Do-Deeper".into(),
            enabled: true,
            details: "depth +1 when TT stale by 5+, depth ≤ 10".into(),
        },
        Technique {
            name: "ProbCut".into(),
            enabled: true,
            details: "β+200, depth ≥ 5, eval ≥ β−200, SEE ≥ 0".into(),
        },
        Technique {
            name: "Correction History".into(),
            enabled: true,
            details: "pawn (1890), material (1461), minor (1292)".into(),
        },
        Technique {
            name: "PVS + Lazy SMP".into(),
            enabled: true,
            details: "shared TT, depth offsets ±{0,1,−1,2}, best-thread selection".into(),
        },
        Technique {
            name: "Killer Moves".into(),
            enabled: true,
            details: "2 killers per ply".into(),
        },
        Technique {
            name: "Countermove Heuristic".into(),
            enabled: true,
            details: "indexed by previous move [from][to]".into(),
        },
        Technique {
            name: "Continuation History".into(),
            enabled: true,
            details: "1-ply + 2-ply back, [piece][to][piece][to]".into(),
        },
        Technique {
            name: "Capture History".into(),
            enabled: true,
            details: "[moved_piece][to_sq][captured_piece]".into(),
        },
        Technique {
            name: "Quiescence Search".into(),
            enabled: true,
            details: format!(
                "delta={}, max QS ply={}, SEE prune, TT probe",
                params.delta_margin, params.max_qs_ply,
            ),
        },
    ]
}

/// Format all techniques as a compact display string.
pub fn format_techniques(params: &SearchParams) -> String {
    let techs = detect_techniques(params);
    let mut out = String::new();
    for t in &techs {
        let mark = if t.enabled { "✓" } else { "✗" };
        out.push_str(&format!("    {} {}: {}\n", mark, t.name, t.details));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use eval::nnue::{NetworkFormat, NnueNetworkInfo};

    #[test]
    fn test_nnue_info_detect() {
        let info = NnueInfo::detect();
        assert!(info.hidden_size > 0);
        assert!(info.num_buckets > 0);
        assert!(info.name.contains("Akimbo"));
    }

    #[test]
    fn test_from_runtime_uses_metadata_fields() {
        let info = NnueNetworkInfo {
            name: "External Test Net".into(),
            format: NetworkFormat::Akimbo,
            architecture: "768→1024×2→1 SCReLU".into(),
            hidden_size: 1024,
            num_buckets: 4,
            scale: 400,
            qa: 255,
            qb: 64,
            file_size: 6_291_458,
        };
        let display = NnueInfo::from_runtime(info);
        assert_eq!(display.name, "External Test Net");
        assert_eq!(display.format, "Akimbo");
        assert!(display.file_size.contains("MB"));
    }

    #[test]
    fn test_detect_techniques() {
        let params = SearchParams::default();
        let techs = detect_techniques(&params);
        assert!(techs.len() > 10, "Should have many techniques");
        assert!(
            techs.iter().all(|t| t.enabled),
            "All should be enabled by default"
        );
    }

    #[test]
    fn test_format_techniques() {
        let params = SearchParams::default();
        let output = format_techniques(&params);
        assert!(output.contains("Null-Move Pruning"));
        assert!(output.contains("Singular Extensions"));
        assert!(output.contains("✓"));
    }
}
