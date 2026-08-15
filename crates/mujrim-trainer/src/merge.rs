//! Weighted interleave of heterogeneous training sources.
//!
//! Concatenating dumps lets the longest file dominate an epoch. Mix weights
//! instead pick the next source by residual credit, cycling inside each file
//! so a small self-play set can hold its share next to a large binpack.

use crate::datagen::TrainingPosition;

pub fn parse_csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn parse_mix_weights(raw: &str, n: usize) -> Result<Vec<f32>, String> {
    if n == 0 {
        return Err("need at least one data source".into());
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(vec![1.0; n]);
    }
    let weights = parse_csv_list(trimmed)
        .into_iter()
        .map(|part| {
            part.parse::<f32>()
                .map_err(|error| format!("invalid mix weight `{part}`: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if weights.len() != n {
        return Err(format!(
            "--mix has {} weight(s) for {n} source(s)",
            weights.len()
        ));
    }
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight <= 0.0)
    {
        return Err("mix weights must be finite and positive".into());
    }
    Ok(weights)
}

pub fn merge_weighted(
    sources: &[Vec<TrainingPosition>],
    weights: &[f32],
    seed: u64,
) -> Result<Vec<TrainingPosition>, String> {
    if sources.is_empty() {
        return Err("need at least one data source".into());
    }
    if sources.len() != weights.len() {
        return Err(format!(
            "mix has {} weight(s) for {} source(s)",
            weights.len(),
            sources.len()
        ));
    }
    if sources.iter().any(Vec::is_empty) {
        return Err("a data source decoded to zero positions".into());
    }
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight <= 0.0)
    {
        return Err("mix weights must be finite and positive".into());
    }

    let total: usize = sources.iter().map(Vec::len).sum();
    let weight_sum = f64::from(weights.iter().sum::<f32>());
    let mut credits = vec![0.0f64; sources.len()];
    let mut cursors = vec![0usize; sources.len()];
    let mut out = Vec::with_capacity(total);
    for _ in 0..total {
        for (credit, &weight) in credits.iter_mut().zip(weights) {
            *credit += f64::from(weight);
        }
        let pick = credits
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(index, _)| index)
            .expect("credits are non-empty");
        credits[pick] -= weight_sum;
        let source = &sources[pick];
        out.push(source[cursors[pick] % source.len()].clone());
        cursors[pick] += 1;
    }
    shuffle_positions(&mut out, seed);
    Ok(out)
}

fn shuffle_positions(positions: &mut [TrainingPosition], seed: u64) {
    let mut state = seed | 1;
    for index in (1..positions.len()).rev() {
        state = splitmix64(state);
        let swap = (state as usize) % (index + 1);
        positions.swap(index, swap);
    }
}

fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(tag: &str) -> TrainingPosition {
        TrainingPosition {
            fen: tag.into(),
            score: 0,
            wdl: 0.5,
        }
    }

    fn source(tag: &str, n: usize) -> Vec<TrainingPosition> {
        (0..n).map(|i| pos(&format!("{tag}{i}"))).collect()
    }

    #[test]
    fn parse_mix_defaults_and_rejects_mismatch() {
        assert_eq!(parse_mix_weights("", 2).unwrap(), vec![1.0, 1.0]);
        assert_eq!(parse_mix_weights("2,1", 2).unwrap(), vec![2.0, 1.0]);
        assert!(parse_mix_weights("2", 2).unwrap_err().contains("weight"));
        assert!(parse_mix_weights("0,1", 2).is_err());
        assert_eq!(parse_csv_list(" a.txt, b.plain , "), ["a.txt", "b.plain"]);
    }

    #[test]
    fn equal_weights_keep_both_sources_in_the_epoch() {
        let merged = merge_weighted(&[source("a", 2), source("b", 2)], &[1.0, 1.0], 1).unwrap();
        assert_eq!(merged.len(), 4);
        let a = merged.iter().filter(|p| p.fen.starts_with('a')).count();
        let b = merged.iter().filter(|p| p.fen.starts_with('b')).count();
        assert_eq!(a, 2);
        assert_eq!(b, 2);
    }

    #[test]
    fn three_to_one_mix_oversamples_the_heavy_source() {
        let merged = merge_weighted(&[source("a", 8), source("b", 8)], &[3.0, 1.0], 7).unwrap();
        assert_eq!(merged.len(), 16);
        let a = merged.iter().filter(|p| p.fen.starts_with('a')).count();
        let b = merged.iter().filter(|p| p.fen.starts_with('b')).count();
        assert!(a >= 11, "expected ~12 from A, got {a}");
        assert!(b <= 5, "expected ~4 from B, got {b}");
    }

    #[test]
    fn small_selfplay_holds_share_against_a_large_dump() {
        let merged = merge_weighted(&[source("sp", 2), source("sf", 20)], &[1.0, 1.0], 3).unwrap();
        let selfplay = merged.iter().filter(|p| p.fen.starts_with("sp")).count();
        assert!(
            selfplay >= 10,
            "1:1 mix should oversample 2 self-play rows against 20 dump rows, got {selfplay}"
        );
    }

    #[test]
    fn same_seed_is_deterministic() {
        let left = merge_weighted(&[source("a", 4), source("b", 4)], &[1.0, 2.0], 99).unwrap();
        let right = merge_weighted(&[source("a", 4), source("b", 4)], &[1.0, 2.0], 99).unwrap();
        assert_eq!(left, right);
        let other = merge_weighted(&[source("a", 4), source("b", 4)], &[1.0, 2.0], 100).unwrap();
        assert_ne!(left, other);
    }

    #[test]
    fn empty_source_is_rejected() {
        assert!(merge_weighted(&[Vec::new()], &[1.0], 1).is_err());
    }
}
