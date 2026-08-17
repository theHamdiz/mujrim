//! Phase 4 CPU trainer for Ateed output heads and a single expert.

use std::path::Path;

use eval::nnue::ateed_format::{
    EXPERTS, FEATURES, KING_BUCKETS, L1, L2, L3, QA, QB, SCALE, WDL_OUTPUTS, stm_piece_features,
};
use eval::nnue::{AteedExpertUpdate, AteedNetwork};
use gpu::{TrainCompute, training_compute};
use types::{Board, Color};

use crate::config::TrainingConfig;
use crate::datagen::TrainingPosition;
use crate::dataset::load_mixed_positions;

const SCORE_SCALE: f32 = SCALE as f32 / (QA * QB) as f32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AteedTrainScope {
    /// SGD on eval/WDL biases. Works from a zero net.
    OutputBiases,
    /// Sparse FT + expert 0 dense layers.
    Expert0,
    /// Top-1 MoE: train the routed expert's output heads and the gate.
    Moe,
}

impl AteedTrainScope {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "heads" | "output-biases" => Ok(Self::OutputBiases),
            "expert0" => Ok(Self::Expert0),
            "moe" => Ok(Self::Moe),
            other => Err(format!("unknown Ateed train scope `{other}`")),
        }
    }
}

struct AteedTrainState {
    ft: Vec<f32>,
    ft_bias: Vec<f32>,
    l1_w: Vec<f32>,
    l1_b: [f32; L2],
    l2_w: Vec<f32>,
    l2_b: [f32; L3],
    eval_w: [f32; L3],
    eval_b: f32,
    wdl_w: [f32; L3 * WDL_OUTPUTS],
    wdl_b: [f32; WDL_OUTPUTS],
    gate_w: Vec<f32>,
    gate_b: [f32; EXPERTS],
    moe_eval_b: [f32; EXPERTS],
    moe_wdl_b: [[f32; WDL_OUTPUTS]; EXPERTS],
}

impl AteedTrainState {
    fn from_network(net: &AteedNetwork) -> Self {
        let mut state = Self::zero_like();
        state.load_from(net);
        state
    }

    fn zero_like() -> Self {
        Self {
            ft: vec![0.0; KING_BUCKETS * FEATURES * L1],
            ft_bias: vec![0.0; L1],
            l1_w: vec![0.0; L2 * L1],
            l1_b: [0.0; L2],
            l2_w: vec![0.0; L3 * L2],
            l2_b: [0.0; L3],
            eval_w: [0.0; L3],
            eval_b: 0.0,
            wdl_w: [0.0; L3 * WDL_OUTPUTS],
            wdl_b: [0.0; WDL_OUTPUTS],
            gate_w: vec![0.0; L1 * EXPERTS],
            gate_b: [0.0; EXPERTS],
            moe_eval_b: [0.0; EXPERTS],
            moe_wdl_b: [[0.0; WDL_OUTPUTS]; EXPERTS],
        }
    }

    fn load_from(&mut self, net: &AteedNetwork) {
        for (slot, &weight) in self.ft.iter_mut().zip(net.feature_weights()) {
            *slot = f32::from(weight);
        }
        for (slot, &bias) in self.ft_bias.iter_mut().zip(net.feature_biases()) {
            *slot = f32::from(bias);
        }
        let expert = net.expert(0).expect("Ateed expert 0 is present");
        for (slot, &weight) in self.l1_w.iter_mut().zip(expert.l1_weights()) {
            *slot = f32::from(weight);
        }
        for (slot, &bias) in self.l1_b.iter_mut().zip(expert.l1_biases()) {
            *slot = bias as f32;
        }
        for (slot, &weight) in self.l2_w.iter_mut().zip(expert.l2_weights()) {
            *slot = f32::from(weight);
        }
        for (slot, &bias) in self.l2_b.iter_mut().zip(expert.l2_biases()) {
            *slot = bias as f32;
        }
        for (slot, &weight) in self.eval_w.iter_mut().zip(expert.eval_weights()) {
            *slot = f32::from(weight);
        }
        self.eval_b = expert.eval_bias() as f32;
        for (slot, &weight) in self.wdl_w.iter_mut().zip(expert.wdl_weights()) {
            *slot = f32::from(weight);
        }
        for (slot, &bias) in self.wdl_b.iter_mut().zip(expert.wdl_biases()) {
            *slot = bias as f32;
        }
        for (slot, &weight) in self.gate_w.iter_mut().zip(net.gate_weights()) {
            *slot = f32::from(weight);
        }
        for (slot, &bias) in self.gate_b.iter_mut().zip(net.gate_biases()) {
            *slot = bias as f32;
        }
        for (index, eval_b) in self.moe_eval_b.iter_mut().enumerate() {
            let expert = net.expert(index).expect("Ateed expert is present");
            *eval_b = expert.eval_bias() as f32;
            for (slot, &bias) in self.moe_wdl_b[index].iter_mut().zip(expert.wdl_biases()) {
                *slot = bias as f32;
            }
        }
    }

    fn seed_expert0_signal(&mut self) {
        for (index, weight) in self.ft.iter_mut().enumerate() {
            *weight = (index % 11) as f32 * 0.02 - 0.1;
        }
        for (index, weight) in self.l1_w.iter_mut().enumerate() {
            *weight = (index % 7) as f32 * 0.01 - 0.03;
        }
        for (index, weight) in self.l2_w.iter_mut().enumerate() {
            *weight = (index % 5) as f32 * 0.02 - 0.04;
        }
        self.eval_w[0] = 1.0;
        self.l2_b[0] = 8.0;
        self.l1_b[0] = 8.0;
    }

    fn apply_to_network(&self, net: &mut AteedNetwork) -> Result<(), String> {
        let ft: Vec<i16> = self.ft.iter().copied().map(quant_i16).collect();
        let ft_bias: Vec<i16> = self.ft_bias.iter().copied().map(quant_i16).collect();
        net.set_feature_transformer(&ft, &ft_bias)?;
        let l1_w: Vec<i8> = self.l1_w.iter().copied().map(quant_i8).collect();
        let l1_b: Vec<i32> = self.l1_b.iter().copied().map(quant_i32).collect();
        let l2_w: Vec<i8> = self.l2_w.iter().copied().map(quant_i8).collect();
        let l2_b: Vec<i32> = self.l2_b.iter().copied().map(quant_i32).collect();
        let eval_w: Vec<i8> = self.eval_w.iter().copied().map(quant_i8).collect();
        let wdl_w: Vec<i8> = self.wdl_w.iter().copied().map(quant_i8).collect();
        let wdl_b: Vec<i32> = self.wdl_b.iter().copied().map(quant_i32).collect();
        net.set_expert(
            0,
            AteedExpertUpdate {
                l1_weights: &l1_w,
                l1_biases: &l1_b,
                l2_weights: &l2_w,
                l2_biases: &l2_b,
                eval_weights: &eval_w,
                eval_bias: quant_i32(self.eval_b),
                wdl_weights: &wdl_w,
                wdl_biases: &wdl_b,
            },
        )?;
        let gate_w: Vec<i8> = self.gate_w.iter().copied().map(quant_i8).collect();
        let gate_b: Vec<i32> = self.gate_b.iter().copied().map(quant_i32).collect();
        net.set_gate(&gate_w, &gate_b)?;
        for expert in 0..EXPERTS {
            net.set_expert_output_biases(
                expert,
                quant_i32(self.moe_eval_b[expert]),
                [
                    quant_i32(self.moe_wdl_b[expert][0]),
                    quant_i32(self.moe_wdl_b[expert][1]),
                    quant_i32(self.moe_wdl_b[expert][2]),
                ],
            )?;
        }
        Ok(())
    }
}

fn quant_i16(value: f32) -> i16 {
    value.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn quant_i8(value: f32) -> i8 {
    value.round().clamp(i8::MIN as f32, i8::MAX as f32) as i8
}

fn quant_i32(value: f32) -> i32 {
    value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn stm_wdl(board: &Board, white_wdl: f32) -> f32 {
    if board.side_to_move == Color::White {
        white_wdl
    } else {
        1.0 - white_wdl
    }
}

fn wdl_target(wdl: f32) -> [f32; WDL_OUTPUTS] {
    let win = (2.0 * wdl - 1.0).max(0.0);
    let loss = (1.0 - 2.0 * wdl).max(0.0);
    [win, 1.0 - win - loss, loss]
}

fn softmax(logits: [f32; WDL_OUTPUTS]) -> [f32; WDL_OUTPUTS] {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut exp = [0.0; WDL_OUTPUTS];
    let mut sum = 0.0;
    for (slot, logit) in exp.iter_mut().zip(logits) {
        *slot = ((logit - max) / 64.0).exp();
        sum += *slot;
    }
    if sum <= 0.0 {
        return [1.0 / 3.0; WDL_OUTPUTS];
    }
    for slot in &mut exp {
        *slot /= sum;
    }
    exp
}

fn clamp_ste(value: f32, lo: f32, hi: f32) -> (f32, bool) {
    (value.clamp(lo, hi), value > lo && value < hi)
}

struct Expert0Forward {
    score: f32,
    wdl: [f32; WDL_OUTPUTS],
    features: Vec<usize>,
    l1: [f32; L1],
    l2: [f32; L2],
    l3: [f32; L3],
    a1: [f32; L1],
    a2: [f32; L2],
    a3: [f32; L3],
}

fn forward_heads(state: &AteedTrainState) -> (f32, [f32; WDL_OUTPUTS]) {
    (state.eval_b * SCORE_SCALE, state.wdl_b)
}

fn forward_expert0(
    state: &AteedTrainState,
    board: &Board,
    compute: &impl TrainCompute,
) -> Expert0Forward {
    let features = stm_piece_features(board);
    let mut l1 = [0.0; L1];
    l1.copy_from_slice(&state.ft_bias);
    for &feature in &features {
        let start = feature * L1;
        for (dst, &weight) in l1.iter_mut().zip(&state.ft[start..start + L1]) {
            *dst += weight;
        }
    }
    let mut a1 = [0.0; L1];
    for (dst, &value) in a1.iter_mut().zip(&l1) {
        *dst = value.clamp(0.0, QA as f32);
    }
    let mut l2 = [0.0; L2];
    compute.matvec_f32(&state.l1_w, &a1, L2, L1, &mut l2);
    for (dst, &bias) in l2.iter_mut().zip(&state.l1_b) {
        *dst += bias;
    }
    let mut a2 = [0.0; L2];
    for (dst, &value) in a2.iter_mut().zip(&l2) {
        *dst = value.clamp(0.0, 127.0);
    }
    let mut l3 = [0.0; L3];
    compute.matvec_f32(&state.l2_w, &a2, L3, L2, &mut l3);
    for (dst, &bias) in l3.iter_mut().zip(&state.l2_b) {
        *dst += bias;
    }
    let mut a3 = [0.0; L3];
    for (dst, &value) in a3.iter_mut().zip(&l3) {
        *dst = value.clamp(0.0, 127.0);
    }
    let eval = state.eval_b
        + a3.iter()
            .zip(&state.eval_w)
            .map(|(a, w)| a * w)
            .sum::<f32>();
    let mut wdl = state.wdl_b;
    for (logit, row) in wdl.iter_mut().zip(state.wdl_w.chunks_exact(L3)) {
        *logit += a3.iter().zip(row).map(|(a, w)| a * w).sum::<f32>();
    }
    Expert0Forward {
        score: eval * SCORE_SCALE,
        wdl,
        features,
        l1,
        l2,
        l3,
        a1,
        a2,
        a3,
    }
}

fn step_heads(
    state: &mut AteedTrainState,
    target_score: f32,
    target_wdl: f32,
    lr: f32,
    wdl_weight: f32,
) -> f32 {
    let (pred, logits) = forward_heads(state);
    let score_weight = 1.0 - wdl_weight;
    state.eval_b -= lr * score_weight * (pred - target_score) / SCORE_SCALE;
    let probs = softmax(logits);
    let target = wdl_target(target_wdl);
    for (bias, (&prob, &tgt)) in state.wdl_b.iter_mut().zip(probs.iter().zip(&target)) {
        *bias -= lr * wdl_weight * (prob - tgt) / 64.0;
    }
    (pred - target_score).abs()
}

fn route_gate(logits: [f32; EXPERTS]) -> usize {
    let mut best = 0;
    for (index, &logit) in logits.iter().enumerate().skip(1) {
        if logit > logits[best] {
            best = index;
        }
    }
    best
}

fn step_moe(
    state: &mut AteedTrainState,
    target_score: f32,
    target_wdl: f32,
    lr: f32,
    wdl_weight: f32,
) {
    let expert = route_gate(state.gate_b);
    let pred = state.moe_eval_b[expert] * SCORE_SCALE;
    let score_weight = 1.0 - wdl_weight;
    state.moe_eval_b[expert] -= lr * score_weight * (pred - target_score) / SCORE_SCALE;
    let probs = softmax(state.moe_wdl_b[expert]);
    let target = wdl_target(target_wdl);
    for (bias, (&prob, &tgt)) in state.moe_wdl_b[expert]
        .iter_mut()
        .zip(probs.iter().zip(&target))
    {
        *bias -= lr * wdl_weight * (prob - tgt) / 64.0;
    }
}

fn step_expert0(
    state: &mut AteedTrainState,
    board: &Board,
    target_score: f32,
    target_wdl: f32,
    lr: f32,
    wdl_weight: f32,
    compute: &impl TrainCompute,
) {
    let fwd = forward_expert0(state, board, compute);
    let score_weight = 1.0 - wdl_weight;
    let d_eval = score_weight * (fwd.score - target_score) / SCORE_SCALE;
    let probs = softmax(fwd.wdl);
    let target = wdl_target(target_wdl);
    let mut d_wdl = [0.0; WDL_OUTPUTS];
    for (grad, (&prob, &tgt)) in d_wdl.iter_mut().zip(probs.iter().zip(&target)) {
        *grad = wdl_weight * (prob - tgt) / 64.0;
    }

    state.eval_b -= lr * d_eval;
    for (bias, &grad) in state.wdl_b.iter_mut().zip(&d_wdl) {
        *bias -= lr * grad;
    }

    let mut d_a3 = [0.0; L3];
    for (i, (d_act, &a3)) in d_a3.iter_mut().zip(&fwd.a3).enumerate() {
        *d_act += d_eval * state.eval_w[i];
        state.eval_w[i] -= lr * d_eval * a3;
        for (k, &wdl_grad) in d_wdl.iter().enumerate() {
            *d_act += wdl_grad * state.wdl_w[k * L3 + i];
            state.wdl_w[k * L3 + i] -= lr * wdl_grad * a3;
        }
    }

    let mut d_l3 = [0.0; L3];
    for (((d_pre, &pre), &d_act), bias) in
        d_l3.iter_mut().zip(&fwd.l3).zip(&d_a3).zip(&mut state.l2_b)
    {
        let (_, pass) = clamp_ste(pre, 0.0, 127.0);
        *d_pre = if pass { d_act } else { 0.0 };
        *bias -= lr * *d_pre;
    }
    let mut d_a2 = [0.0; L2];
    for (i, &d_pre) in d_l3.iter().enumerate() {
        let row = i * L2;
        for (d_act, &weight) in d_a2.iter_mut().zip(&state.l2_w[row..row + L2]) {
            *d_act += d_pre * weight;
        }
        for (weight, &a2) in state.l2_w[row..row + L2].iter_mut().zip(&fwd.a2) {
            *weight -= lr * d_pre * a2;
        }
    }
    let mut d_l2 = [0.0; L2];
    for (((d_pre, &pre), &d_act), bias) in
        d_l2.iter_mut().zip(&fwd.l2).zip(&d_a2).zip(&mut state.l1_b)
    {
        let (_, pass) = clamp_ste(pre, 0.0, 127.0);
        *d_pre = if pass { d_act } else { 0.0 };
        *bias -= lr * *d_pre;
    }
    let mut d_a1 = [0.0; L1];
    for (j, &d_pre) in d_l2.iter().enumerate() {
        let row = j * L1;
        for (d_act, &weight) in d_a1.iter_mut().zip(&state.l1_w[row..row + L1]) {
            *d_act += d_pre * weight;
        }
        for (weight, &a1) in state.l1_w[row..row + L1].iter_mut().zip(&fwd.a1) {
            *weight -= lr * d_pre * a1;
        }
    }
    let mut d_l1 = [0.0; L1];
    for (((d_pre, &pre), &d_act), bias) in d_l1
        .iter_mut()
        .zip(&fwd.l1)
        .zip(&d_a1)
        .zip(&mut state.ft_bias)
    {
        let (_, pass) = clamp_ste(pre, 0.0, QA as f32);
        *d_pre = if pass { d_act } else { 0.0 };
        *bias -= lr * *d_pre;
    }
    for &feature in &fwd.features {
        let start = feature * L1;
        for (weight, &grad) in state.ft[start..start + L1].iter_mut().zip(&d_l1) {
            *weight -= lr * grad;
        }
    }
}

pub fn train_ateed(
    positions: &[TrainingPosition],
    config: &TrainingConfig,
    scope: AteedTrainScope,
) -> Result<AteedNetwork, String> {
    if positions.is_empty() {
        return Err("Ateed trainer needs at least one position".to_string());
    }
    let net = if let Some(base) = &config.base_network {
        load_ateed_network(Path::new(base))?
    } else {
        AteedNetwork::zero()
    };
    train_ateed_from(positions, config, scope, net, 1, |_, _| Ok(()))
}

fn snapshot_network(
    state: &mut AteedTrainState,
    scope: AteedTrainScope,
    net: &mut AteedNetwork,
) -> Result<(), String> {
    if scope != AteedTrainScope::Moe {
        for eval_b in &mut state.moe_eval_b {
            *eval_b = state.eval_b;
        }
        for wdl_b in &mut state.moe_wdl_b {
            *wdl_b = state.wdl_b;
        }
    }
    state.apply_to_network(net)
}

pub fn train_ateed_from(
    positions: &[TrainingPosition],
    config: &TrainingConfig,
    scope: AteedTrainScope,
    mut net: AteedNetwork,
    start_epoch: u32,
    mut on_epoch: impl FnMut(u32, &AteedNetwork) -> Result<(), String>,
) -> Result<AteedNetwork, String> {
    if positions.is_empty() {
        return Err("Ateed trainer needs at least one position".to_string());
    }
    let mut state = AteedTrainState::from_network(&net);
    if scope == AteedTrainScope::Expert0
        && state.l1_w.iter().all(|w| *w == 0.0)
        && state.ft.iter().all(|w| *w == 0.0)
    {
        state.seed_expert0_signal();
    }
    let compute = training_compute();
    let lr = config.learning_rate as f32;
    let wdl_weight = config.wdl_weight.clamp(0.0, 1.0) as f32;
    let start_epoch = start_epoch.max(1);
    let holdout = holdout_len(positions.len());
    let train_end = positions.len() - holdout;
    let train_set = &positions[..train_end];
    let val_set = &positions[train_end..];
    for epoch in start_epoch..=config.epochs {
        let epoch_start = std::time::Instant::now();
        let mut abs_err = 0.0f32;
        for position in train_set {
            let board = Board::from_fen(&position.fen)?;
            let target_wdl = stm_wdl(&board, position.wdl);
            let residual = match scope {
                AteedTrainScope::OutputBiases => step_heads(
                    &mut state,
                    position.score as f32,
                    target_wdl,
                    lr,
                    wdl_weight,
                ),
                AteedTrainScope::Expert0 => {
                    step_expert0(
                        &mut state,
                        &board,
                        position.score as f32,
                        target_wdl,
                        lr,
                        wdl_weight,
                        &compute,
                    );
                    0.0
                }
                AteedTrainScope::Moe => {
                    step_moe(
                        &mut state,
                        position.score as f32,
                        target_wdl,
                        lr,
                        wdl_weight,
                    );
                    0.0
                }
            };
            abs_err += residual;
        }
        let train_len = train_set.len().max(1);
        let loss = abs_err / train_len as f32;
        let val_loss = if val_set.is_empty() {
            None
        } else {
            Some(mean_residual(&state, val_set, scope, &compute)?)
        };
        let elapsed = epoch_start.elapsed().as_secs_f64().max(0.001);
        let mpos = train_set.len() as f32 / elapsed as f32 / 1_000_000.0;
        updater::progress::emit_progress(&epoch_progress(
            epoch,
            config.epochs,
            loss,
            val_loss,
            lr,
            mpos,
        ));
        snapshot_network(&mut state, scope, &mut net)?;
        on_epoch(epoch, &net)?;
    }
    Ok(net)
}

/// Last 10% of a dataset is reserved for validation when at least 10 positions exist.
pub fn holdout_len(len: usize) -> usize {
    if len < 10 { 0 } else { (len / 10).max(1) }
}

pub fn epoch_progress(
    epoch: u32,
    epochs: u32,
    loss: f32,
    val_loss: Option<f32>,
    lr: f32,
    mpos: f32,
) -> updater::progress::JobProgress {
    updater::progress::JobProgress::train_batch(updater::progress::TrainerBatch {
        epoch,
        epochs,
        loss,
        val_loss,
        expert: 0,
        lr,
        mpos,
    })
}

fn mean_residual(
    state: &AteedTrainState,
    positions: &[TrainingPosition],
    scope: AteedTrainScope,
    compute: &impl TrainCompute,
) -> Result<f32, String> {
    let mut abs_err = 0.0f32;
    for position in positions {
        abs_err += residual_of(state, position, scope, compute)?;
    }
    Ok(abs_err / positions.len() as f32)
}

fn residual_of(
    state: &AteedTrainState,
    position: &TrainingPosition,
    scope: AteedTrainScope,
    compute: &impl TrainCompute,
) -> Result<f32, String> {
    let board = Board::from_fen(&position.fen)?;
    let target = position.score as f32;
    let pred = match scope {
        AteedTrainScope::OutputBiases => forward_heads(state).0,
        AteedTrainScope::Expert0 => forward_expert0(state, &board, compute).score,
        AteedTrainScope::Moe => {
            let expert = route_gate(state.gate_b);
            state.moe_eval_b[expert] * SCORE_SCALE
        }
    };
    Ok((pred - target).abs())
}

fn load_ateed_network(path: &Path) -> Result<AteedNetwork, String> {
    match eval::nnue::load_network(path).map_err(|error| error.to_string())? {
        eval::nnue::ActiveNetwork::ExternalAteed { network, .. } => Ok(*network),
        _ => Err("checkpoint is not Ateed".to_string()),
    }
}

pub fn train_ateed_to_file(config: &TrainingConfig, scope: AteedTrainScope) -> Result<(), String> {
    let mut config = config.clone();
    config.output_path = eval::nnue::resolve_ateed_output_path(&config.output_path)
        .to_string_lossy()
        .into_owned();
    let positions = load_mixed_positions(&config.data_path, &config.mix_weights, config.mix_seed)?;
    let output = Path::new(&config.output_path);
    let partial = crate::job::JobCheckpoint::partial_path(output);
    let completed = crate::job::resume_train(&config, scope);
    let net = if completed > 0 {
        if partial.exists() {
            load_ateed_network(&partial)?
        } else if output.exists() {
            load_ateed_network(output)?
        } else {
            return Err("train sidecar exists but the partial network is missing".to_string());
        }
    } else if let Some(base) = &config.base_network {
        load_ateed_network(Path::new(base))?
    } else if output.exists() {
        load_ateed_network(output)?
    } else {
        AteedNetwork::zero()
    };
    if completed >= config.epochs {
        crate::ateed::emit_network(output, &net).map_err(|error| error.to_string())?;
        crate::job::JobCheckpoint::clear(output);
        return Ok(());
    }
    let net = train_ateed_from(
        &positions,
        &config,
        scope,
        net,
        completed + 1,
        |epoch, snapshot| {
            crate::ateed::emit_network(&partial, snapshot).map_err(|error| error.to_string())?;
            crate::job::train_checkpoint(&config, scope, epoch).save()
        },
    )?;
    crate::ateed::emit_network(output, &net).map_err(|error| error.to_string())?;
    crate::job::JobCheckpoint::clear(output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn startpos_line(score: i32, wdl: f32) -> TrainingPosition {
        TrainingPosition {
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".to_string(),
            score,
            wdl,
        }
    }

    #[test]
    fn train_scope_parse_accepts_heads_and_expert0() {
        assert_eq!(
            AteedTrainScope::parse("heads").unwrap(),
            AteedTrainScope::OutputBiases
        );
        assert_eq!(
            AteedTrainScope::parse("expert0").unwrap(),
            AteedTrainScope::Expert0
        );
        assert_eq!(AteedTrainScope::parse("moe").unwrap(), AteedTrainScope::Moe);
        assert!(AteedTrainScope::parse("bullet").is_err());
    }

    #[test]
    fn heads_trainer_moves_zero_net_toward_a_constant_score() {
        types::init();
        let config = TrainingConfig {
            epochs: 8,
            learning_rate: 1.0,
            wdl_weight: 0.0,
            ..Default::default()
        };
        let net = train_ateed(
            &[startpos_line(80, 0.5)],
            &config,
            AteedTrainScope::OutputBiases,
        )
        .expect("train heads");
        let score = net.evaluate(&Board::new());
        let bias = net.expert(0).expect("expert 0").eval_bias();
        assert!(
            score > 40,
            "heads trainer should approach +80, got {score} bias={bias} scale={SCORE_SCALE}"
        );
    }

    #[test]
    fn expert0_trainer_changes_a_zero_net_eval() {
        types::init();
        let config = TrainingConfig {
            epochs: 2,
            learning_rate: 0.05,
            wdl_weight: 0.0,
            ..Default::default()
        };
        let before = AteedNetwork::zero().evaluate(&Board::new());
        let net = train_ateed(&[startpos_line(60, 0.5)], &config, AteedTrainScope::Expert0)
            .expect("train expert0");
        assert_ne!(net.evaluate(&Board::new()), before);
    }

    #[test]
    fn train_to_file_resumes_from_a_sidecar_epoch() {
        types::init();
        let dir = std::env::temp_dir().join(format!(
            "mujrim-train-resume-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let data = dir.join("data.txt");
        let output = dir.join("net.bin");
        std::fs::write(
            &data,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|80|0.5\n",
        )
        .expect("data");
        let config = TrainingConfig {
            data_path: data.display().to_string(),
            output_path: output.display().to_string(),
            epochs: 2,
            learning_rate: 1.0,
            wdl_weight: 0.0,
            ..Default::default()
        };
        train_ateed_to_file(&config, AteedTrainScope::OutputBiases).expect("first run");
        crate::job::train_checkpoint(&config, AteedTrainScope::OutputBiases, 1)
            .save()
            .expect("sidecar");
        crate::ateed::emit_network(
            &crate::job::JobCheckpoint::partial_path(&output),
            &AteedNetwork::zero(),
        )
        .expect("partial");
        train_ateed_to_file(&config, AteedTrainScope::OutputBiases).expect("resume");
        assert!(!crate::job::JobCheckpoint::path_for(&output).exists());
        assert!(output.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn train_to_file_continues_from_the_existing_artifact() {
        types::init();
        let dir = std::env::temp_dir().join(format!(
            "mujrim-train-cumulative-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let data = dir.join("data.txt");
        let output = dir.join("net.bin");
        std::fs::write(
            &data,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|80|0.5\n",
        )
        .expect("data");
        let config = TrainingConfig {
            data_path: data.display().to_string(),
            output_path: output.display().to_string(),
            epochs: 1,
            learning_rate: 0.05,
            wdl_weight: 0.0,
            ..Default::default()
        };
        train_ateed_to_file(&config, AteedTrainScope::OutputBiases).expect("first");
        let first = load_ateed_network(&output).expect("load first");
        let first_score = first.evaluate(&Board::new());
        let first_bias = first.expert(0).expect("expert 0").eval_bias();
        train_ateed_to_file(&config, AteedTrainScope::OutputBiases).expect("second");
        let second = load_ateed_network(&output).expect("load second");
        let second_score = second.evaluate(&Board::new());
        let second_bias = second.expert(0).expect("expert 0").eval_bias();
        let _ = std::fs::remove_dir_all(dir);
        assert_ne!(first_score, 0, "first run must leave a trained artifact");
        assert_ne!(first_bias, 0);
        assert_ne!(
            second_bias, first_bias,
            "a second run must keep training the same artifact"
        );
        assert!(
            (second_score - 80).abs() <= (first_score - 80).abs(),
            "cumulative train should not move away from the target, first={first_score} second={second_score}"
        );
    }

    #[test]
    fn moe_trainer_updates_only_the_routed_expert() {
        types::init();
        let mut base = AteedNetwork::zero();
        base.set_gate(&vec![0; L1 * EXPERTS], &[0, 8, 0, 0])
            .expect("seed gate toward expert 1");
        let path = std::env::temp_dir().join("mujrim-ateed-moe-base.bin");
        crate::ateed::emit_network(&path, &base).expect("write base");
        let config = TrainingConfig {
            epochs: 8,
            learning_rate: 1.0,
            wdl_weight: 0.0,
            base_network: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let net = train_ateed(&[startpos_line(80, 0.5)], &config, AteedTrainScope::Moe)
            .expect("train moe");
        let _ = std::fs::remove_file(&path);
        let eval = net.evaluate_full(&Board::new());
        assert_eq!(eval.expert, 1);
        assert!(
            eval.score > 40,
            "routed expert should approach +80, got {}",
            eval.score
        );
        assert_eq!(net.expert(0).expect("expert 0").eval_bias(), 0);
        assert_ne!(net.expert(1).expect("expert 1").eval_bias(), 0);
    }

    #[test]
    fn heads_trainer_one_epoch_stays_within_a_latency_budget() {
        types::init();
        let config = TrainingConfig {
            epochs: 1,
            learning_rate: 1.0,
            wdl_weight: 0.0,
            ..Default::default()
        };
        let start = std::time::Instant::now();
        let net = train_ateed(
            &[startpos_line(40, 0.5)],
            &config,
            AteedTrainScope::OutputBiases,
        )
        .expect("train heads");
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 2_000,
            "one-epoch heads train budget exceeded: {elapsed:?}"
        );
        assert!(net.evaluate(&Board::new()).abs() < 10_000);
    }

    #[test]
    fn train_ateed_to_file_mixes_two_decoded_sources() {
        types::init();
        let dir = std::env::temp_dir();
        let a = dir.join(format!("mujrim-train-mix-a-{}.txt", std::process::id()));
        let b = dir.join(format!("mujrim-train-mix-b-{}.plain", std::process::id()));
        let out = dir.join(format!("mujrim-train-mix-{}.bin", std::process::id()));
        std::fs::write(
            &a,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1|40|0.5\n",
        )
        .unwrap();
        std::fs::write(
            &b,
            crate::formats::encode_stockfish_plain(&[startpos_line(40, 0.5)]),
        )
        .unwrap();
        let config = TrainingConfig {
            data_path: format!("{},{}", a.display(), b.display()),
            output_path: out.to_string_lossy().into_owned(),
            epochs: 1,
            learning_rate: 1.0,
            wdl_weight: 0.0,
            mix_weights: "1,1".into(),
            mix_seed: 2,
            ..Default::default()
        };
        train_ateed_to_file(&config, AteedTrainScope::OutputBiases).expect("mix train");
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
        let size = std::fs::metadata(&out).map(|meta| meta.len()).unwrap_or(0);
        let _ = std::fs::remove_file(&out);
        assert!(size > 0);
    }

    #[test]
    fn holdout_split_reserves_ten_percent_after_ten_positions() {
        assert_eq!(holdout_len(1), 0);
        assert_eq!(holdout_len(9), 0);
        assert_eq!(holdout_len(10), 1);
        assert_eq!(holdout_len(100), 10);
    }

    #[test]
    fn epoch_progress_includes_val_loss_mpos_and_lr() {
        let progress = epoch_progress(2, 8, 0.3, Some(0.4), 0.01, 1.5);
        assert_eq!(progress.epoch, Some(2));
        assert_eq!(progress.epochs, Some(8));
        assert!((progress.loss.unwrap() - 0.3).abs() < 0.001);
        assert!((progress.val_loss.unwrap() - 0.4).abs() < 0.001);
        assert!((progress.lr.unwrap() - 0.01).abs() < 0.0001);
        assert!((progress.mpos.unwrap() - 1.5).abs() < 0.001);
        let line = updater::progress::format_progress(&progress);
        assert!(line.contains("val_loss="));
        assert!(line.contains("mpos="));
        assert!(line.contains("lr="));
    }

    #[test]
    fn empty_dataset_is_rejected() {
        let err = train_ateed(
            &[],
            &TrainingConfig::default(),
            AteedTrainScope::OutputBiases,
        )
        .err()
        .expect("empty dataset");
        assert!(err.contains("at least one position"));
    }
}
