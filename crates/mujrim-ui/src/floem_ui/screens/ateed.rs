//! Password-gated Ateed studio: sources, dry-run train, live metrics.

use floem::ext_event::create_ext_action;
use floem::prelude::*;
use floem::taffy::style::{Display, FlexWrap, Overflow};

use crate::app_core::ateed_studio::{
    AteedJobKind, AteedMonitorTick, AteedPerfReport, AteedSourceKind, AteedStrengthReport,
    dry_run_train, evaluate_zero_net, format_perf, format_strength, plan_job, probe_compute,
    unlock_ateed, validate_source,
};

use super::super::state::{AppHandles, AppState};
use super::super::theme;
use super::super::widgets;

pub fn studio(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::new((
        lock_gate(state).style(move |s| {
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

fn lock_gate(state: AppState) -> impl IntoView {
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
                "Enter the studio key to plan multi-source datasets, dry-run MoE training, and inspect in-memory network strength. Downloads and full trains stay queued until you start them from the CLI.",
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
            widgets::primary_button(state, "Unlock studio", move || try_unlock(state))
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
    let sources = widgets::card(state, sources_panel(state));
    let train = widgets::card(state, train_panel(state));
    let monitor = widgets::card(state, monitor_panel(state));
    let strength = widgets::card(state, strength_panel(state, handles));
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
        Label::new("Queue sources, dry-run a train, then evaluate the in-memory net. Nothing leaves this machine until you run `mujrim train fetch` or `mujrim train ateed`.")
            .style(move |s| {
                s.font_size(13.0)
                    .color(theme::rgba(pal().text_secondary))
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
        Stack::horizontal((
            Stack::vertical((sources, train))
                .style(|s| s.row_gap(16.0).flex_grow(1.0f32).min_width(280.0).max_width(560.0)),
            Stack::vertical((monitor, strength))
                .style(|s| s.row_gap(16.0).flex_grow(1.0f32).min_width(280.0).max_width(640.0)),
        ))
        .style(|s| {
            s.width_full()
                .col_gap(16.0)
                .row_gap(16.0)
                .flex_wrap(FlexWrap::Wrap)
                .items_stretch()
        }),
    ))
    .style(move |s| {
        s.size_full()
            .padding(20.0)
            .row_gap(14.0)
            .min_width(0.0)
            .background(theme::rgba(pal().bg))
    })
    .scroll()
}

fn sources_panel(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Data sources", pal),
        widgets::body_copy(
            "Local files, HTTP Range URLs, or a self-play game count. Adding a source only validates it.",
            pal,
        ),
        Stack::horizontal((
            source_kind_chip(state, "http", "HTTP"),
            source_kind_chip(state, "local", "Local"),
            source_kind_chip(state, "datagen", "Datagen"),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
        TextInput::new(state.ateed.source_value).style(|s| {
            s.width_full()
                .height(36.0)
                .border_radius(10.0)
        }),
        Stack::horizontal((
            widgets::primary_button(state, "Add source", move || add_source(state)),
            widgets::ghost_button(state, "Queue fetch", move || queue_fetch(state)),
            widgets::ghost_button(state, "Clear", move || {
                state.ateed.sources.set(Vec::new());
                push_log(state, "cleared sources");
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
        Label::derived(move || {
            let sources = state.ateed.sources.get();
            if sources.is_empty() {
                "No sources queued.".to_owned()
            } else {
                sources
                    .iter()
                    .map(|source| format!("{} · {}", source.kind.label(), source.value))
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
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn train_panel(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Training plan", pal),
        widgets::body_copy(
            "Scope heads, expert0, or moe. Start runs a timed dry-run so the monitor updates without writing a net.",
            pal,
        ),
        labeled_field(state, "Scope", state.ateed.scope),
        labeled_field(state, "Epochs", state.ateed.epochs),
        labeled_field(state, "Learning rate", state.ateed.lr),
        labeled_field(state, "WDL weight", state.ateed.wdl_weight),
        Stack::horizontal((
            widgets::primary_button(state, "Dry-run train", move || start_dry_train(state)),
            widgets::ghost_button(state, "Lock studio", move || {
                state.ateed.unlocked.set(false);
                state.ateed.password.set(String::new());
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn monitor_panel(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Live monitor", pal),
        progress_bar(state),
        Stack::horizontal((
            metric_chip(state, "Epoch", move || state.ateed.epoch.get().to_string()),
            metric_chip(state, "Loss", move || {
                format!("{:.3}", state.ateed.loss.get())
            }),
            metric_chip(state, "Expert", move || {
                state.ateed.expert.get().to_string()
            }),
            metric_chip(state, "State", move || {
                if state.ateed.running.get() {
                    "running".to_owned()
                } else {
                    "idle".to_owned()
                }
            }),
        ))
        .style(|s| {
            s.width_full()
                .col_gap(8.0)
                .row_gap(8.0)
                .flex_wrap(FlexWrap::Wrap)
        }),
        Label::derived(move || {
            let lines = state.ateed.log.get();
            if lines.is_empty() {
                "Job log is empty.".to_owned()
            } else {
                lines
                    .iter()
                    .rev()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
        .style(move |s| {
            s.font_size(12.0)
                .font_family("monospace".to_owned())
                .color(theme::rgba(pal().text_secondary))
                .min_width(0.0)
                .width_full()
                .text_wrap()
        }),
    ))
    .style(|s| s.row_gap(10.0).width_full())
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

fn labeled_field(state: AppState, label: &'static str, value: RwSignal<String>) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::new(label)
            .style(move |s| s.font_size(11.0).color(theme::rgba(pal().text_secondary))),
        TextInput::new(value).style(|s| s.width_full().height(34.0).border_radius(10.0)),
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

fn try_unlock(state: AppState) {
    let password = state.ateed.password.get_untracked();
    if unlock_ateed(&password) {
        state.ateed.unlocked.set(true);
        state.ateed.gate_error.set(String::new());
        state.ateed.password.set(String::new());
        push_log(state, "studio unlocked");
        state.status.set("Ateed studio unlocked.".to_owned());
    } else {
        state.ateed.gate_error.set("Access denied.".to_owned());
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
    match validate_source(kind, &state.ateed.source_value.get_untracked()) {
        Ok(source) => {
            state
                .ateed
                .sources
                .update(|sources| sources.push(source.clone()));
            state.ateed.source_value.set(String::new());
            push_log(
                state,
                &format!("queued {} · {}", source.kind.label(), source.value),
            );
        }
        Err(error) => push_log(state, &error),
    }
}

fn queue_fetch(state: AppState) {
    let sources = state.ateed.sources.get_untracked();
    match plan_job(
        AteedJobKind::Fetch,
        &sources,
        &state.ateed.scope.get_untracked(),
        1,
    ) {
        Ok(plan) => push_log(state, &plan.summary),
        Err(error) => push_log(state, &error),
    }
}

fn start_dry_train(state: AppState) {
    if state.ateed.running.get_untracked() {
        push_log(state, "a job is already running");
        return;
    }
    let sources = state.ateed.sources.get_untracked();
    let scope = state.ateed.scope.get_untracked();
    let epochs = state
        .ateed
        .epochs
        .get_untracked()
        .parse::<u32>()
        .unwrap_or(0);
    match plan_job(AteedJobKind::Train, &sources, &scope, epochs) {
        Ok(plan) => {
            state.ateed.running.set(true);
            state.ateed.progress.set(0.0);
            push_log(state, &plan.summary);
            pump_ticks_ui(state, dry_run_train(epochs, 0), 0);
        }
        Err(error) => push_log(state, &error),
    }
}

fn pump_ticks_ui(state: AppState, ticks: Vec<AteedMonitorTick>, index: usize) {
    if index >= ticks.len() {
        state.ateed.running.set(false);
        push_log(state, "dry-run complete");
        return;
    }
    apply_tick(state, &ticks[index]);
    floem::action::exec_after(std::time::Duration::from_millis(90), move |_| {
        pump_ticks_ui(state, ticks, index + 1);
    });
}

fn apply_tick(state: AppState, tick: &AteedMonitorTick) {
    state.ateed.epoch.set(tick.epoch);
    state.ateed.progress.set(tick.progress);
    state.ateed.loss.set(tick.loss);
    state.ateed.expert.set(tick.expert);
    push_log(state, &tick.message);
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
    if state.ateed.running.get_untracked() {
        push_log(state, "a job is already running");
        return false;
    }
    match plan_job(
        kind,
        &state.ateed.sources.get_untracked(),
        &state.ateed.scope.get_untracked(),
        1,
    ) {
        Ok(plan) => {
            state.ateed.running.set(true);
            push_log(state, &plan.summary);
            true
        }
        Err(error) => {
            push_log(state, &error);
            false
        }
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
    #[test]
    fn ateed_screen_keeps_lock_and_dashboard_mounted() {
        let src = include_str!("ateed.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("lock_gate"));
        assert!(production.contains("dashboard"));
        assert!(production.contains("Display::None"));
        assert!(!production.contains("JAHANAM"));
        assert!(production.contains("ateed_studio"));
        assert!(
            !production.contains("dyn_view"),
            "lock/dashboard must stay mounted"
        );
    }
}
