//! Password-gated Ateed studio: sources, CLI jobs, live metrics.

use std::path::PathBuf;

use floem::ext_event::create_ext_action;
use floem::prelude::*;
use floem::taffy::style::{Display, FlexWrap, Overflow};

use crate::app_core::ateed_studio::{
    AteedCliCommand, AteedJobKind, AteedMonitorTick, AteedPerfReport, AteedSourceKind,
    AteedStrengthReport, CliProcessSignal, LossRing, MetricRing, catalog_draft, cli_args,
    continuing_train_base, datagen_batch_size, dataset_format_for_path, ensure_local_source,
    evaluate_zero_net, field_help, format_perf, format_strength, index_tournament_positions,
    local_mix, monitor_from_progress, plan_job, probe_compute, run_mujrim_cli, signal_live_cli,
    unlock_ateed, validate_source, validate_weighted_source,
};
use crate::app_core::layout;

use super::super::state::{AppHandles, AppState, offer_ateed_index, refresh_ateed_cli};
use super::super::telemetry_charts;
use super::super::theme;
use super::super::widgets;

pub fn studio(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::new((
        lock_gate(state, handles.clone()).style(move |s| {
            let s = s.size_full().min_width(0.0).min_height(0.0);
            if state.ateed.unlocked.get() {
                s.display(Display::None)
            } else {
                s
            }
        }),
        dashboard(state, handles).style(move |s| {
            let s = s.size_full().min_width(0.0).min_height(0.0);
            if state.ateed.unlocked.get() {
                s
            } else {
                s.display(Display::None)
            }
        }),
    ))
    .style(|s| s.size_full().min_width(0.0).min_height(0.0))
}

fn ateed_resume_banner(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        Label::derived(move || {
            state
                .ateed
                .resume_prompt
                .get()
                .map_or_else(String::new, |job| job.summary)
        })
        .style(move |s| {
            s.font_size(13.0)
                .font_bold()
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(
                    theme::palette(state.settings.get().board_theme).accent_alt,
                ))
        }),
        Label::new(
            "The last fetch, train, datagen, decode, or merge job can continue from its sidecar.",
        )
        .style(move |s| {
            s.font_size(11.0)
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(
                    theme::palette(state.settings.get().board_theme).text_secondary,
                ))
        }),
        Stack::horizontal((
            widgets::primary_button(state, "Resume job", {
                let handles = handles.clone();
                move || resume_ateed_job(state, &handles)
            }),
            widgets::ghost_button(state, "Discard", move || discard_ateed_job(state)),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let s = s
            .width_full()
            .row_gap(8.0)
            .padding(12.0)
            .border_radius(12.0)
            .border(1.0)
            .border_color(theme::rgba(pal.accent))
            .background(theme::rgba(pal.panel));
        if state.ateed.resume_prompt.get().is_some() && !state.ateed.running.get() {
            s
        } else {
            s.display(Display::None)
        }
    })
}

fn resume_ateed_job(state: AppState, handles: &AppHandles) {
    let Some(job) = state.ateed.resume_prompt.get_untracked() else {
        return;
    };
    if state.ateed.running.get_untracked() {
        push_log(state, "a job is already running");
        return;
    }
    state.ateed.running.set(true);
    state.ateed.progress.set(0.0);
    reset_telemetry(state, progress_kind_from_command(&job.command));
    push_log(state, &job.summary);
    spawn_cli(state, handles, job.command, "resumed job complete");
}

fn discard_ateed_job(state: AppState) {
    state.ateed.resume_prompt.set(None);
    crate::app_core::ateed_resume::ActiveAteedJob::clear();
    push_log(state, "interrupted Ateed job discarded");
}

fn ateed_index_banner(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        Label::derived(move || {
            state
                .ateed
                .index_prompt
                .get()
                .map_or_else(String::new, |prompt| prompt.summary)
        })
        .style(move |s| {
            s.font_size(13.0)
                .font_bold()
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(
                    theme::palette(state.settings.get().board_theme).accent_alt,
                ))
        }),
        Label::new(
            "Tournament games can be deduped into the shared Ateed dataset before the next train.",
        )
        .style(move |s| {
            s.font_size(11.0)
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(
                    theme::palette(state.settings.get().board_theme).text_secondary,
                ))
        }),
        Stack::horizontal((
            widgets::primary_button(state, "Index games", {
                let handles = handles.clone();
                move || index_tournament_games(state, &handles)
            }),
            widgets::ghost_button(state, "Later", move || {
                state.ateed.index_prompt.set(None);
                push_log(state, "tournament index deferred");
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let s = s
            .width_full()
            .row_gap(8.0)
            .padding(12.0)
            .border_radius(12.0)
            .border(1.0)
            .border_color(theme::rgba(pal.accent))
            .background(theme::rgba(pal.panel));
        if state.ateed.index_prompt.get().is_some() && !state.ateed.running.get() {
            s
        } else {
            s.display(Display::None)
        }
    })
}

fn index_tournament_games(state: AppState, handles: &AppHandles) {
    let result = {
        let study = handles.study.borrow();
        let Some(db) = study.as_ref() else {
            push_log(state, "study database is not open");
            return;
        };
        index_tournament_positions(db)
    };
    match result {
        Ok((dataset, summary)) => {
            state.ateed.sources.update(|sources| {
                ensure_local_source(sources, &dataset);
            });
            state.ateed.index_prompt.set(None);
            push_log(state, &summary);
        }
        Err(error) => push_log(state, &error),
    }
}

fn lock_gate(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let card = widgets::glass_card(
        state,
        Stack::vertical((
            widgets::curious_title("ATEED", 42.0)
                .style(move |s| s.color(theme::rgba(pal().text_primary))),
            Label::new("Restricted training studio").style(move |s| {
                s.font_size(16.0)
                    .color(theme::rgba(pal().accent_alt))
            }),
            widgets::body_copy(
                "Enter the studio key to fetch Stockfish/Lc0/self-play dumps, decode them, merge mix weights, generate data, and train Ateed from the CLI.",
                pal,
            ),
            TextInput::new(state.ateed.password).style(|s| {
                s.width_full()
                    .height(40.0)
                    .border_radius(12.0)
            }),
            Label::derived(move || state.ateed.gate_error.get()).style(move |s| {
                s.font_size(12.0)
                    .color(theme::rgba(pal().accent))
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            widgets::primary_button(state, "Unlock studio", {
                let handles = handles.clone();
                move || try_unlock(state, &handles)
            })
                .style(|s| s.width_full().height(44.0)),
        ))
        .style(|s| s.row_gap(12.0).width_full().max_width(460.0)),
    );
    Stack::new((card,)).style(move |s| {
        s.size_full()
            .items_center()
            .justify_center()
            .padding(24.0)
            .background(theme::rgba(pal().bg))
    })
}

fn dashboard(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let sources = widgets::card(
        state,
        widgets::capped_scroll(
            sources_panel(state, handles.clone()),
            layout::ATEED_PANEL_SCROLL_PX,
        ),
    );
    let train = widgets::card(
        state,
        widgets::capped_scroll(
            train_panel(state, handles.clone()),
            layout::ATEED_PANEL_SCROLL_PX,
        ),
    );
    let status = widgets::card(state, status_panel(state));
    let datagen = widgets::card(state, datagen_panel(state, handles.clone()));
    let trainer = widgets::card(state, trainer_panel(state));
    let strength = widgets::card(state, strength_panel(state, handles.clone()));
    let strategy = widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("How to train Ateed", pal),
            widgets::body_copy(field_help("strategy"), pal),
            widgets::body_copy(field_help("selfplay"), pal),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
    );
    Stack::vertical((
        Stack::horizontal((
            widgets::curious_title("Ateed Control", 28.0)
                .style(move |s| s.color(theme::rgba(pal().text_primary))),
            Label::new("MoE · WDL · live telemetry").style(move |s| {
                s.font_size(13.0)
                    .color(theme::rgba(pal().accent_alt))
            }),
        ))
        .style(|s| {
            s.width_full()
                .col_gap(12.0)
                .row_gap(6.0)
                .items_center()
                .flex_wrap(FlexWrap::Wrap)
        }),
        ateed_resume_banner(state, handles.clone()),
        ateed_index_banner(state, handles.clone()),
        Label::derived(move || {
            if state.ateed.cli_available.get() {
                format!("CLI ready · {}", state.ateed.cli_path.get())
            } else {
                "CLI missing · put mujrim-train (or mujrim-ateed) in engines/mujrim/ next to the UI."
                    .to_owned()
            }
        })
        .style(move |s| {
            s.font_size(13.0)
                .color(theme::rgba(if state.ateed.cli_available.get() {
                    pal().accent_alt
                } else {
                    pal().text_secondary
                }))
                .min_width(0.0)
                .width_full()
                .text_wrap()
        }),
        strategy,
        Stack::horizontal((
            sources.style(|s| s.flex_grow(1.0f32).flex_basis(0.0).min_width(0.0)),
            train.style(|s| s.flex_grow(1.0f32).flex_basis(0.0).min_width(0.0)),
            widgets::filling_scroll(
                Stack::vertical((datagen, status, trainer, strength))
                    .style(|s| s.row_gap(16.0).width_full()),
            )
            .style(|s| s.flex_grow(1.4f32).flex_basis(0.0).min_width(0.0)),
        ))
        .style(|s| {
            s.width_full()
                .flex_grow(1.0f32)
                .min_height(0.0)
                .col_gap(16.0)
                .items_stretch()
        }),
    ))
    .style(move |s| {
        s.size_full()
            .padding(20.0)
            .row_gap(14.0)
            .min_width(0.0)
            .min_height(0.0)
            .background(theme::rgba(pal().bg))
    })
}

fn sources_panel(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Data sources", pal),
        widgets::body_copy(field_help("source"), pal),
        Stack::horizontal((
            catalog_chip(state, "stockfish-plain", "Stockfish"),
            catalog_chip(state, "lc0-training", "Lc0"),
            catalog_chip(state, "selfplay-gz", "Self-play"),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
        Stack::horizontal((
            source_kind_chip(state, "http", "HTTP"),
            source_kind_chip(state, "local", "Local"),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
        source_value_field(state),
        explained_field(state, "Mix weight", "mix", state.ateed.source_weight),
        Stack::horizontal((
            widgets::primary_button(state, "Add source", move || add_source(state)),
            widgets::primary_button_when(state, "Fetch", move || cli_ready(state), {
                let handles = handles.clone();
                move || start_fetch(state, &handles)
            }),
            widgets::primary_button_when(state, "Decode", move || cli_ready(state), {
                let handles = handles.clone();
                move || start_decode(state, &handles)
            }),
            widgets::primary_button_when(state, "Merge", move || cli_ready(state), {
                let handles = handles.clone();
                move || start_merge(state, &handles)
            }),
            widgets::ghost_button(state, "Clear", move || {
                state.ateed.sources.set(Vec::new());
                push_log(state, "cleared sources");
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
        widgets::capped_scroll(
            Label::derived(move || {
                let sources = state.ateed.sources.get();
                if sources.is_empty() {
                    "No sources queued.".to_owned()
                } else {
                    sources
                        .iter()
                        .map(|source| {
                            format!(
                                "{} · {} · w={}",
                                source.kind.label(),
                                source.value,
                                source.weight
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            })
            .style(move |s| {
                s.font_size(12.0)
                    .color(theme::rgba(pal().text_secondary))
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            layout::LIST_SCROLL_PX,
        ),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn train_panel(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Training plan", pal),
        widgets::body_copy(
            "Train on the dataset file after you have data. Training always starts from the output net if that file already exists — it does not wipe the brain and start over. Datagen lives in its own panel and does not start with train.",
            pal,
        ),
        explained_field(state, "Scope", "scope", state.ateed.scope),
        explained_field(state, "Epochs", "epochs", state.ateed.epochs),
        explained_field(state, "Learning rate", "lr", state.ateed.lr),
        explained_field(state, "WDL weight", "wdl", state.ateed.wdl_weight),
        explained_field(state, "Dataset", "dataset", state.ateed.data_path),
        explained_field(state, "Output net", "output", state.ateed.output_path),
        Stack::horizontal((
            widgets::primary_button_when(
                state,
                "Start train",
                move || cli_ready(state),
                {
                    let handles = handles.clone();
                    move || start_train(state, &handles)
                },
            ),
            widgets::ghost_button(state, "Rescan CLI", move || {
                refresh_ateed_cli(state);
                if state.ateed.cli_available.get_untracked() {
                    push_log(state, "Mujrim CLI found");
                } else {
                    push_log(state, "Mujrim CLI not found");
                }
            }),
            widgets::ghost_button(state, "Lock studio", move || {
                state.ateed.unlocked.set(false);
                state.ateed.password.set(String::new());
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn status_panel(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Telemetry", pal),
        Label::derived(move || telemetry_status_label(state)).style(move |s| {
            s.font_size(16.0)
                .font_bold()
                .color(theme::rgba(if telemetry_connected(state) {
                    pal().accent_alt
                } else {
                    pal().text_secondary
                }))
        }),
        Label::derived(move || {
            let lines = state.ateed.log.get();
            if lines.is_empty() {
                "Job log is empty.".to_owned()
            } else {
                lines
                    .iter()
                    .rev()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
        .style(move |s| {
            s.font_size(12.0)
                .font_family({
                    let family = state.settings.get().mono_font;
                    if family.is_empty() {
                        super::super::theme::MONO_FAMILY.to_owned()
                    } else {
                        family
                    }
                })
                .color(theme::rgba(pal().text_secondary))
                .min_width(0.0)
                .width_full()
                .text_wrap()
        }),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn datagen_panel(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Datagen", pal),
        widgets::body_copy(field_help("datagen_when"), pal),
        explained_field(
            state,
            "Batch positions",
            "positions",
            state.ateed.batch_positions,
        ),
        explained_field(state, "Self-play depth", "depth", state.ateed.batch_depth),
        Stack::horizontal((
            widgets::primary_button_when(state, "Play", move || datagen_can_play(state), {
                let handles = handles.clone();
                move || play_or_resume_datagen(state, &handles)
            }),
            widgets::primary_button_when(state, "Pause", move || datagen_can_pause(state), {
                move || pause_datagen(state)
            }),
            widgets::primary_button_when(state, "Stop", move || datagen_can_stop(state), {
                move || stop_datagen(state)
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
        Stack::horizontal((
            metric_chip(state, "NPS", move || format_u64(state.ateed.nps.get())),
            metric_chip(state, "Positions", move || {
                format_u64(state.ateed.positions.get())
            }),
            metric_chip(state, "MB/s", move || {
                format!("{:.2}", state.ateed.mbps.get())
            }),
        ))
        .style(|s| {
            s.width_full()
                .col_gap(8.0)
                .row_gap(8.0)
                .flex_wrap(FlexWrap::Wrap)
        }),
        wdl_bar(state),
        telemetry_charts::score_histogram(state, 72.0),
        Label::derived(move || {
            format!(
                "pass {} · drop {} · games {}",
                format_u64(state.ateed.pass.get()),
                format_u64(state.ateed.drop.get()),
                format_u64(state.ateed.games.get())
            )
        })
        .style(move |s| {
            s.font_size(11.0)
                .color(theme::rgba(pal().text_secondary))
                .min_width(0.0)
                .width_full()
        }),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn trainer_panel(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Bullet trainer", pal),
        telemetry_charts::loss_sparkline(state, 72.0),
        progress_bar(state),
        Stack::horizontal((
            metric_chip(state, "Epoch", move || {
                format!("{:.0}%", state.ateed.progress.get().clamp(0.0, 1.0) * 100.0)
            }),
            metric_chip(state, "MPos/s", move || {
                format!("{:.3}", state.ateed.mpos.get())
            }),
            metric_chip(state, "LR", move || {
                format!("{:.4}", state.ateed.train_lr.get())
            }),
            metric_chip(state, "Train", move || {
                format!("{:.3}", state.ateed.loss.get())
            }),
            metric_chip(state, "Val", move || {
                format!("{:.3}", state.ateed.val_loss.get())
            }),
        ))
        .style(|s| {
            s.width_full()
                .col_gap(8.0)
                .row_gap(8.0)
                .flex_wrap(FlexWrap::Wrap)
        }),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn wdl_bar(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::horizontal((
        Empty::new().style(move |s| {
            let (white, draw, black) = state.ateed.wdl.get();
            let total = (white + draw + black).max(1) as f32;
            s.height(10.0)
                .flex_grow((white as f32 / total).max(0.001))
                .border_radius(99.0)
                .background(theme::rgba(pal().text_primary))
        }),
        Empty::new().style(move |s| {
            let (white, draw, black) = state.ateed.wdl.get();
            let total = (white + draw + black).max(1) as f32;
            s.height(10.0)
                .flex_grow((draw as f32 / total).max(0.001))
                .background(theme::rgba(pal().text_secondary))
        }),
        Empty::new().style(move |s| {
            let (white, draw, black) = state.ateed.wdl.get();
            let total = (white + draw + black).max(1) as f32;
            s.height(10.0)
                .flex_grow((black as f32 / total).max(0.001))
                .border_radius(99.0)
                .background(theme::rgba(pal().accent))
        }),
    ))
    .style(move |s| {
        s.width_full()
            .height(10.0)
            .border_radius(99.0)
            .background(theme::rgba(pal().bg))
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn strength_panel(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Network strength", pal),
        widgets::body_copy(
            "Evaluate the in-memory zero Ateed net and probe CPU matvec plus eval latency. No dataset is required.",
            pal,
        ),
        Stack::horizontal((
            metric_chip(state, "Score", move || format!("{:+}", state.ateed.score.get())),
            metric_chip(state, "WDL σ²", move || state.ateed.variance.get().to_string()),
            metric_chip(state, "Latency", move || state.ateed.latency.get()),
        ))
        .style(|s| {
            s.width_full()
                .col_gap(8.0)
                .row_gap(8.0)
                .flex_wrap(FlexWrap::Wrap)
        }),
        Label::derived(move || state.ateed.strength.get()).style(move |s| {
            s.font_size(13.0)
                .color(theme::rgba(pal().text_primary))
                .min_width(0.0)
                .width_full()
                .text_wrap()
        }),
        Stack::horizontal((
            widgets::primary_button(state, "Evaluate net", {
                let handles = handles.clone();
                move || start_evaluate(state, &handles)
            }),
            widgets::ghost_button(state, "Probe latency", {
                let handles = handles.clone();
                move || start_bench(state, &handles)
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn catalog_chip(state: AppState, id: &'static str, label: &'static str) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Button::new(label)
        .action(move || apply_catalog(state, id))
        .style(move |s| {
            s.padding_horiz(12.0)
                .padding_vert(8.0)
                .border_radius(10.0)
                .border(1.0)
                .border_color(theme::rgba(pal().border))
                .font_size(12.0)
                .background(theme::rgba(pal().panel))
                .color(theme::rgba(pal().text_primary))
        })
}

fn source_value_caption(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "local" => ("File path", field_help("source_path")),
        _ => ("Download URL", field_help("source_url")),
    }
}

fn source_value_field(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::derived(move || {
            source_value_caption(&state.ateed.source_kind.get())
                .0
                .to_owned()
        })
        .style(move |s| s.font_size(11.0).color(theme::rgba(pal().text_secondary))),
        TextInput::new(state.ateed.source_value)
            .style(|s| s.width_full().height(36.0).border_radius(10.0)),
        Label::derived(move || {
            source_value_caption(&state.ateed.source_kind.get())
                .1
                .to_owned()
        })
        .style(move |s| {
            s.font_size(11.0)
                .color(theme::rgba(pal().text_secondary))
                .min_width(0.0)
                .width_full()
                .text_wrap()
        }),
    ))
    .style(move |s| {
        let hide = state.ateed.source_kind.get() == "datagen";
        let s = s.row_gap(4.0).width_full();
        if hide { s.display(Display::None) } else { s }
    })
}

fn source_kind_chip(state: AppState, kind: &'static str, label: &'static str) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Button::new(label)
        .action(move || state.ateed.source_kind.set(kind.to_owned()))
        .style(move |s| {
            let active = state.ateed.source_kind.get() == kind;
            s.padding_horiz(12.0)
                .padding_vert(8.0)
                .border_radius(10.0)
                .border(1.0)
                .border_color(theme::rgba(pal().border))
                .font_size(12.0)
                .background(theme::rgba(if active { pal().accent } else { pal().panel }))
                .color(theme::rgba(pal().text_primary))
        })
}

fn explained_field(
    state: AppState,
    label: &'static str,
    help_id: &'static str,
    value: RwSignal<String>,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::new(label)
            .style(move |s| s.font_size(11.0).color(theme::rgba(pal().text_secondary))),
        TextInput::new(value).style(|s| s.width_full().height(34.0).border_radius(10.0)),
        Label::new(field_help(help_id)).style(move |s| {
            s.font_size(11.0)
                .color(theme::rgba(pal().text_secondary))
                .min_width(0.0)
                .width_full()
                .text_wrap()
        }),
    ))
    .style(|s| s.row_gap(4.0).width_full())
}

fn metric_chip(
    state: AppState,
    label: &'static str,
    value: impl Fn() -> String + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::derived(value).style(move |s| {
            s.font_size(18.0)
                .font_bold()
                .color(theme::rgba(pal().text_primary))
        }),
        Label::new(label)
            .style(move |s| s.font_size(10.0).color(theme::rgba(pal().text_secondary))),
    ))
    .style(move |s| {
        s.flex_grow(1.0f32)
            .min_width(90.0)
            .padding(10.0)
            .row_gap(2.0)
            .border_radius(12.0)
            .background(theme::rgba(pal().bg))
    })
}

fn progress_bar(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::horizontal((
        Empty::new().style(move |s| {
            let done = state.ateed.progress.get().clamp(0.0, 1.0);
            let s = s
                .height(8.0)
                .flex_grow(done.max(0.001))
                .border_radius(99.0)
                .background(theme::rgba(pal().accent));
            if done <= 0.0 {
                s.display(Display::None)
            } else {
                s
            }
        }),
        Empty::new().style(move |s| {
            let rest = (1.0 - state.ateed.progress.get().clamp(0.0, 1.0)).max(0.0);
            s.height(8.0).flex_grow(rest.max(0.001))
        }),
    ))
    .style(move |s| {
        s.width_full()
            .height(8.0)
            .border_radius(99.0)
            .background(theme::rgba(pal().bg))
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn try_unlock(state: AppState, handles: &AppHandles) {
    let password = state.ateed.password.get_untracked();
    if unlock_ateed(&password) {
        state.ateed.unlocked.set(true);
        state.ateed.gate_error.set(String::new());
        state.ateed.password.set(String::new());
        refresh_ateed_cli(state);
        offer_ateed_index(state, handles);
        push_log(state, "studio unlocked");
        state.status.set("Ateed studio unlocked.".to_owned());
    } else {
        state.ateed.gate_error.set("Access denied.".to_owned());
    }
}

fn cli_ready(state: AppState) -> bool {
    state.ateed.cli_available.get() && !state.ateed.running.get()
}

fn datagen_job_active(state: AppState) -> bool {
    state.ateed.running.get()
        && state.ateed.telemetry_kind.get() == Some(updater::progress::JobKind::Datagen)
}

fn datagen_can_play(state: AppState) -> bool {
    if state.ateed.datagen_paused.get() {
        return true;
    }
    cli_ready(state)
}

fn datagen_can_pause(state: AppState) -> bool {
    datagen_job_active(state) && !state.ateed.datagen_paused.get()
}

fn datagen_can_stop(state: AppState) -> bool {
    datagen_job_active(state) || state.ateed.datagen_paused.get()
}

fn play_or_resume_datagen(state: AppState, handles: &AppHandles) {
    if state.ateed.datagen_paused.get_untracked() {
        resume_datagen(state);
    } else {
        start_datagen(state, handles);
    }
}

fn pause_datagen(state: AppState) {
    match signal_live_cli(CliProcessSignal::Pause) {
        Ok(()) => {
            state.ateed.datagen_paused.set(true);
            push_log(state, "datagen paused");
        }
        Err(error) => push_log(state, &error),
    }
}

fn resume_datagen(state: AppState) {
    match signal_live_cli(CliProcessSignal::Resume) {
        Ok(()) => {
            state.ateed.datagen_paused.set(false);
            state
                .ateed
                .last_tick_ms
                .set(crate::app_core::tournament_live::now_unix_ms());
            push_log(state, "datagen resumed");
        }
        Err(error) => push_log(state, &error),
    }
}

fn stop_datagen(state: AppState) {
    match signal_live_cli(CliProcessSignal::Stop) {
        Ok(()) => {
            state.ateed.datagen_paused.set(false);
            state.ateed.running.set(false);
            push_log(state, "datagen stopped — sidecar kept for Resume job");
        }
        Err(error) => push_log(state, &error),
    }
}

fn apply_catalog(state: AppState, id: &'static str) {
    match catalog_draft(id) {
        Ok((kind, value)) => {
            state.ateed.source_kind.set(match kind {
                AteedSourceKind::Http => "http".to_owned(),
                AteedSourceKind::LocalFile => "local".to_owned(),
                AteedSourceKind::Datagen => "datagen".to_owned(),
            });
            state.ateed.source_value.set(value.to_owned());
            push_log(
                state,
                &format!(
                    "catalog {id} — append a filename if this is a directory root, then Add source"
                ),
            );
        }
        Err(error) => push_log(state, &error),
    }
}

fn add_source(state: AppState) {
    let kind = match AteedSourceKind::parse(&state.ateed.source_kind.get_untracked()) {
        Ok(kind) => kind,
        Err(error) => {
            push_log(state, &error);
            return;
        }
    };
    let weight = state
        .ateed
        .source_weight
        .get_untracked()
        .parse::<u32>()
        .unwrap_or(0);
    match validate_weighted_source(kind, &state.ateed.source_value.get_untracked(), weight) {
        Ok(source) => {
            state
                .ateed
                .sources
                .update(|sources| sources.push(source.clone()));
            state.ateed.source_value.set(String::new());
            push_log(
                state,
                &format!(
                    "queued {} · {} · w={}",
                    source.kind.label(),
                    source.value,
                    source.weight
                ),
            );
        }
        Err(error) => push_log(state, &error),
    }
}

fn start_fetch(state: AppState, handles: &AppHandles) {
    let sources = state.ateed.sources.get_untracked();
    if !begin_cli_job(state, AteedJobKind::Fetch) {
        return;
    }
    let Some(url) = sources
        .iter()
        .find(|source| source.kind == AteedSourceKind::Http)
        .map(|source| source.value.clone())
    else {
        state.ateed.running.set(false);
        push_log(state, "add at least one HTTP data source");
        return;
    };
    let output = state.ateed.data_path.get_untracked();
    spawn_cli(
        state,
        handles,
        AteedCliCommand::Fetch {
            id: None,
            url,
            output,
        },
        "fetch complete",
    );
}

fn start_train(state: AppState, handles: &AppHandles) {
    let sources = state.ateed.sources.get_untracked();
    let epochs = state
        .ateed
        .epochs
        .get_untracked()
        .parse::<u32>()
        .unwrap_or(0);
    if !begin_cli_job(state, AteedJobKind::Train) {
        return;
    }
    let (data, mix) = local_mix(&sources);
    let data = if data.is_empty() {
        state.ateed.data_path.get_untracked()
    } else {
        data
    };
    let output = state.ateed.output_path.get_untracked();
    let base = continuing_train_base(&output);
    if base.is_some() {
        push_log(
            state,
            "continuing from the existing output net — this session will not start from zero",
        );
    }
    spawn_cli(
        state,
        handles,
        AteedCliCommand::Train {
            data,
            mix,
            output,
            epochs,
            lr: state.ateed.lr.get_untracked(),
            wdl_weight: state.ateed.wdl_weight.get_untracked(),
            scope: state.ateed.scope.get_untracked(),
            base,
        },
        "train complete",
    );
}

fn start_decode(state: AppState, handles: &AppHandles) {
    let sources = state.ateed.sources.get_untracked();
    if !begin_cli_job(state, AteedJobKind::Decode) {
        return;
    }
    let input = sources
        .iter()
        .find(|source| source.kind == AteedSourceKind::LocalFile)
        .map(|source| source.value.clone())
        .unwrap_or_else(|| state.ateed.data_path.get_untracked());
    let output = state.ateed.data_path.get_untracked();
    spawn_cli(
        state,
        handles,
        AteedCliCommand::Decode {
            format: dataset_format_for_path(&output).to_owned(),
            input,
            output,
        },
        "decode complete",
    );
}

fn start_merge(state: AppState, handles: &AppHandles) {
    let sources = state.ateed.sources.get_untracked();
    if !begin_cli_job(state, AteedJobKind::Merge) {
        return;
    }
    let (data, mix) = local_mix(&sources);
    let output = state.ateed.data_path.get_untracked();
    spawn_cli(
        state,
        handles,
        AteedCliCommand::Merge {
            data,
            mix,
            format: dataset_format_for_path(&output).to_owned(),
            output,
        },
        "merge complete",
    );
}

fn start_datagen(state: AppState, handles: &AppHandles) {
    if !begin_cli_job(state, AteedJobKind::Datagen) {
        return;
    }
    let positions = datagen_batch_size(&state.ateed.sources.get_untracked()).max(
        state
            .ateed
            .batch_positions
            .get_untracked()
            .parse()
            .unwrap_or(0),
    );
    let depth = state.ateed.batch_depth.get_untracked().parse().unwrap_or(6);
    spawn_cli(
        state,
        handles,
        AteedCliCommand::Datagen {
            games: positions.max(1),
            positions: Some(positions),
            depth,
            output: state.ateed.data_path.get_untracked(),
            format: dataset_format_for_path(&state.ateed.data_path.get_untracked()).to_owned(),
        },
        "datagen complete",
    );
}

fn spawn_cli(state: AppState, _handles: &AppHandles, command: AteedCliCommand, done: &'static str) {
    let job = crate::app_core::ateed_resume::ActiveAteedJob::from_command(command.clone());
    job.save();
    state.ateed.resume_prompt.set(Some(job));
    let cli = PathBuf::from(state.ateed.cli_path.get_untracked());
    let args = cli_args(&command);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let send = |event: StudioEvent| {
            let _ = tx.send(event);
        };
        match run_mujrim_cli(&cli, &args, |line| {
            send(StudioEvent::Line(line.to_owned()));
        }) {
            Ok(0) => send(StudioEvent::Done(Ok(()))),
            Ok(code) => send(StudioEvent::Done(Err(format!("CLI exited {code}")))),
            Err(error) => send(StudioEvent::Done(Err(error))),
        }
    });
    pump_cli_events(state, rx, done);
}

fn pump_cli_events(
    state: AppState,
    rx: std::sync::mpsc::Receiver<StudioEvent>,
    done: &'static str,
) {
    let mut finished = false;
    let mut last_progress = None;
    loop {
        match rx.try_recv() {
            Ok(StudioEvent::Line(line)) => {
                if let Some(progress) = updater::progress::parse_progress_line(&line) {
                    last_progress = Some(progress);
                } else {
                    push_log(state, &line);
                }
            }
            Ok(StudioEvent::Done(Ok(()))) => {
                let was_running = state.ateed.running.get_untracked();
                state.ateed.running.set(false);
                state.ateed.datagen_paused.set(false);
                if was_running {
                    state.ateed.progress.set(1.0);
                    state.ateed.telemetry_kind.set(None);
                    state.ateed.resume_prompt.set(None);
                    crate::app_core::ateed_resume::ActiveAteedJob::clear();
                    push_log(state, done);
                }
                finished = true;
            }
            Ok(StudioEvent::Done(Err(error))) => {
                let was_running = state.ateed.running.get_untracked();
                state.ateed.running.set(false);
                state.ateed.datagen_paused.set(false);
                if was_running {
                    state.ateed.telemetry_kind.set(None);
                    push_log(state, &error);
                }
                finished = true;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if state.ateed.running.get_untracked() {
                    state.ateed.running.set(false);
                    state.ateed.datagen_paused.set(false);
                    state.ateed.telemetry_kind.set(None);
                    push_log(state, "CLI stopped");
                }
                finished = true;
                break;
            }
        }
    }
    if let Some(progress) = last_progress {
        apply_tick(state, &monitor_from_progress(&progress));
    }
    if !finished {
        floem::action::exec_after(std::time::Duration::from_millis(250), move |_| {
            pump_cli_events(state, rx, done);
        });
    }
}

#[derive(Clone)]
enum StudioEvent {
    Line(String),
    Done(Result<(), String>),
}

fn apply_tick(state: AppState, tick: &AteedMonitorTick) {
    state.ateed.telemetry_kind.set(Some(tick.kind));
    state
        .ateed
        .last_tick_ms
        .set(crate::app_core::tournament_live::now_unix_ms());
    state.ateed.epoch.set(tick.epoch);
    state.ateed.progress.set(tick.progress);
    state.ateed.loss.set(tick.loss);
    state.ateed.val_loss.set(tick.val_loss);
    state.ateed.expert.set(tick.expert);
    state.ateed.nps.set(tick.nps);
    state.ateed.games.set(tick.games);
    state.ateed.positions.set(tick.positions);
    state.ateed.mbps.set(tick.mbps);
    state.ateed.mpos.set(tick.mpos);
    state.ateed.train_lr.set(tick.lr);
    state.ateed.wdl.set(tick.wdl);
    state.ateed.pass.set(tick.pass);
    state.ateed.drop.set(tick.drop);
    state.ateed.hist.set(tick.hist);
    if tick.kind == updater::progress::JobKind::Train {
        state.ateed.loss_ring.update(|ring| {
            ring.train.push(tick.loss);
            ring.val.push(tick.val_loss);
        });
    }
    if tick.kind == updater::progress::JobKind::Datagen && tick.nps > 0 {
        state
            .ateed
            .nps_ring
            .update(|ring| ring.push(tick.nps as f32));
    }
}

fn start_evaluate(state: AppState, handles: &AppHandles) {
    if !begin_job(state, AteedJobKind::Evaluate) {
        return;
    }
    let on_done = create_ext_action(handles.ui_scope, move |report: AteedStrengthReport| {
        state.ateed.score.set(report.score);
        state.ateed.variance.set(report.variance);
        state.ateed.expert.set(report.expert);
        state.ateed.strength.set(format_strength(&report));
        state.ateed.running.set(false);
        push_log(state, "in-memory evaluation complete");
    });
    std::thread::spawn(move || on_done(evaluate_zero_net()));
}

fn start_bench(state: AppState, handles: &AppHandles) {
    if !begin_job(state, AteedJobKind::Bench) {
        return;
    }
    let on_done = create_ext_action(handles.ui_scope, move |report: AteedPerfReport| {
        state.ateed.latency.set(format_perf(&report));
        state.ateed.running.set(false);
        push_log(state, &format_perf(&report));
    });
    std::thread::spawn(move || on_done(probe_compute()));
}

fn begin_job(state: AppState, kind: AteedJobKind) -> bool {
    begin_planned_job(state, kind, 1)
}

fn begin_cli_job(state: AppState, kind: AteedJobKind) -> bool {
    let epochs = state
        .ateed
        .epochs
        .get_untracked()
        .parse::<u32>()
        .unwrap_or(1);
    begin_planned_job(state, kind, epochs)
}

fn begin_planned_job(state: AppState, kind: AteedJobKind, epochs: u32) -> bool {
    if state.ateed.running.get_untracked() {
        push_log(state, "a job is already running");
        return false;
    }
    refresh_ateed_cli(state);
    let mut sources = state.ateed.sources.get_untracked();
    if matches!(kind, AteedJobKind::Train | AteedJobKind::Decode)
        && sources
            .iter()
            .all(|source| source.kind != AteedSourceKind::LocalFile)
    {
        let data = state.ateed.data_path.get_untracked();
        if let Ok(source) = validate_source(AteedSourceKind::LocalFile, &data) {
            sources.push(source);
        }
    }
    if kind == AteedJobKind::Datagen
        && sources
            .iter()
            .all(|source| source.kind != AteedSourceKind::Datagen)
    {
        let batch = state.ateed.batch_positions.get_untracked();
        if let Ok(source) = validate_source(AteedSourceKind::Datagen, &batch) {
            sources.push(source);
        }
    }
    match plan_job(
        kind,
        &sources,
        &state.ateed.scope.get_untracked(),
        epochs,
        state.ateed.cli_available.get_untracked(),
    ) {
        Ok(plan) => {
            state.ateed.running.set(true);
            state.ateed.datagen_paused.set(false);
            state.ateed.progress.set(0.0);
            reset_telemetry(state, progress_kind(kind));
            push_log(state, &plan.summary);
            true
        }
        Err(error) => {
            push_log(state, &error);
            false
        }
    }
}

fn progress_kind(kind: AteedJobKind) -> Option<updater::progress::JobKind> {
    match kind {
        AteedJobKind::Fetch => Some(updater::progress::JobKind::Fetch),
        AteedJobKind::Train => Some(updater::progress::JobKind::Train),
        AteedJobKind::Datagen => Some(updater::progress::JobKind::Datagen),
        _ => None,
    }
}

fn progress_kind_from_command(command: &AteedCliCommand) -> Option<updater::progress::JobKind> {
    match command {
        AteedCliCommand::Fetch { .. } => Some(updater::progress::JobKind::Fetch),
        AteedCliCommand::Train { .. } => Some(updater::progress::JobKind::Train),
        AteedCliCommand::Datagen { .. } => Some(updater::progress::JobKind::Datagen),
        AteedCliCommand::Decode { .. } | AteedCliCommand::Merge { .. } => None,
    }
}

fn reset_telemetry(state: AppState, kind: Option<updater::progress::JobKind>) {
    state.ateed.telemetry_kind.set(kind);
    state
        .ateed
        .last_tick_ms
        .set(crate::app_core::tournament_live::now_unix_ms());
    state.ateed.nps.set(0);
    state.ateed.games.set(0);
    state.ateed.positions.set(0);
    state.ateed.mbps.set(0.0);
    state.ateed.mpos.set(0.0);
    state.ateed.train_lr.set(0.0);
    state.ateed.val_loss.set(0.0);
    state.ateed.wdl.set((0, 0, 0));
    state.ateed.pass.set(0);
    state.ateed.drop.set(0);
    state.ateed.hist.set([0; updater::progress::HIST_BUCKETS]);
    state.ateed.loss_ring.set(LossRing::default());
    state.ateed.nps_ring.set(MetricRing::default());
}

fn telemetry_has_visible_data(state: AppState) -> bool {
    !state.ateed.log.get().is_empty()
        || state.ateed.positions.get() > 0
        || state.ateed.nps.get() > 0
        || state.ateed.games.get() > 0
        || state.ateed.progress.get() > 0.0
        || state.ateed.last_tick_ms.get() > 0
}

fn telemetry_connected(state: AppState) -> bool {
    state.ateed.running.get()
        || state.ateed.datagen_paused.get()
        || telemetry_has_visible_data(state)
}

fn telemetry_status_text(
    running: bool,
    paused: bool,
    kind: Option<updater::progress::JobKind>,
    has_data: bool,
) -> String {
    let job = match kind {
        Some(updater::progress::JobKind::Datagen) => "datagen",
        Some(updater::progress::JobKind::Train) => "train",
        Some(updater::progress::JobKind::Fetch) => "fetch",
        None => "",
    };
    if paused {
        return if job.is_empty() {
            "Paused".to_owned()
        } else {
            format!("Paused · {job}")
        };
    }
    if running {
        return if job.is_empty() {
            "Live".to_owned()
        } else {
            format!("Live · {job}")
        };
    }
    if has_data {
        return if job.is_empty() {
            "Idle · last batch".to_owned()
        } else {
            format!("Idle · last {job}")
        };
    }
    "Idle".to_owned()
}

fn telemetry_status_label(state: AppState) -> String {
    telemetry_status_text(
        state.ateed.running.get(),
        state.ateed.datagen_paused.get(),
        state.ateed.telemetry_kind.get(),
        telemetry_has_visible_data(state),
    )
}

fn format_u64(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 10_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn push_log(state: AppState, line: &str) {
    state.ateed.log.update(|lines| {
        lines.push(line.to_owned());
        if lines.len() > 48 {
            let drop = lines.len() - 48;
            lines.drain(..drop);
        }
    });
    state.status.set(line.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ateed_screen_keeps_lock_and_dashboard_mounted() {
        let src = include_str!("ateed.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("lock_gate"));
        assert!(production.contains("dashboard"));
        assert!(production.contains("ateed_resume_banner"));
        assert!(production.contains("ateed_index_banner"));
        assert!(production.contains("Index games"));
        assert!(production.contains("Display::None"));
        assert!(!production.contains("JAHANAM"));
        assert!(production.contains("ateed_studio"));
        assert!(production.contains("cli_ready"));
        assert!(production.contains("Start train"));
        assert!(production.contains("engines/mujrim"));
        assert!(production.contains("continuing_train_base"));
        assert!(production.contains("Batch positions"));
        assert!(production.contains("field_help"));
        assert!(production.contains("stockfish-plain"));
        assert!(production.contains("Decode"));
        assert!(production.contains("Merge"));
        assert!(production.contains("Live ·"));
        assert!(production.contains("Paused ·"));
        assert!(!production.contains("Disconnected / Idle"));
        assert!(production.contains("Bullet trainer"));
        assert!(production.contains("Datagen"));
        assert!(production.contains("\"Play\""));
        assert!(production.contains("\"Pause\""));
        assert!(production.contains("\"Stop\""));
        assert!(production.contains("capped_scroll"));
        assert!(production.contains("filling_scroll"));
        assert!(production.contains("ATEED_PANEL_SCROLL_PX"));
        assert!(production.contains("Download URL"));
        assert!(production.contains("File path"));
        assert!(!production.contains("primary_button_when(state, \"Datagen\""));
        assert!(production.contains("from_millis(250)"));
        assert!(
            !production.contains("dyn_view"),
            "lock/dashboard must stay mounted"
        );
    }

    #[test]
    fn telemetry_status_stays_live_when_metrics_are_visible() {
        use updater::progress::JobKind;
        assert_eq!(
            telemetry_status_text(true, false, Some(JobKind::Datagen), true),
            "Live · datagen"
        );
        assert_eq!(
            telemetry_status_text(true, true, Some(JobKind::Datagen), true),
            "Paused · datagen"
        );
        assert_eq!(
            telemetry_status_text(false, false, Some(JobKind::Datagen), true),
            "Idle · last datagen"
        );
        assert_eq!(telemetry_status_text(false, false, None, false), "Idle");
        assert_ne!(
            telemetry_status_text(false, false, None, true),
            "Disconnected / Idle"
        );
    }

    #[test]
    fn format_u64_uses_billions_for_default_chunk() {
        assert_eq!(format_u64(1_000_000_000), "1.0B");
        assert_eq!(format_u64(1_000_000), "1.0M");
        assert_eq!(source_value_caption("http").0, "Download URL");
        assert_eq!(source_value_caption("local").0, "File path");
    }
}
