//! TUI — optional ratatui-based terminal UI for live benchmark progress.
//!
//! Renders a table of positions with real-time updates as each position
//! completes. Displays NNUE info, hardware detection, and final summary.

use std::io::{self, stdout};

use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
};

use crate::suite::{BenchSummary, PositionResult, format_nps};

/// Live TUI state during a benchmark run.
pub struct BenchTui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    results: Vec<PositionResult>,
    total_positions: usize,
    header_lines: Vec<String>,
}

impl BenchTui {
    /// Initialize the TUI in alternate screen mode.
    pub fn new(total_positions: usize, header_lines: Vec<String>) -> Result<Self, String> {
        enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {e}"))?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen)
            .map_err(|e| format!("Failed to enter alternate screen: {e}"))?;
        let backend = CrosstermBackend::new(stdout);
        let terminal =
            Terminal::new(backend).map_err(|e| format!("Failed to create terminal: {e}"))?;

        Ok(Self {
            terminal,
            results: Vec::with_capacity(total_positions),
            total_positions,
            header_lines,
        })
    }

    /// Update with a new position result and re-render.
    pub fn update(&mut self, result: PositionResult) {
        self.results.push(result);
        let _ = self.render();
    }

    /// Render the current state.
    fn render(&mut self) -> Result<(), String> {
        let results = &self.results;
        let total = self.total_positions;
        let header = &self.header_lines;

        self.terminal
            .draw(|f| draw_frame(f, results, total, header))
            .map_err(|e| format!("Draw error: {e}"))?;
        Ok(())
    }

    /// Show final summary and wait for user to press any key.
    pub fn show_summary(&mut self, summary: &BenchSummary) {
        let _ = self.render();
        // Restore terminal before printing summary to stdout
        let _ = self.restore();
        println!("{summary}");
    }

    /// Restore terminal to normal mode.
    pub fn restore(&mut self) -> Result<(), String> {
        disable_raw_mode().map_err(|e| format!("Failed to disable raw mode: {e}"))?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
            .map_err(|e| format!("Failed to leave alternate screen: {e}"))?;
        self.terminal
            .show_cursor()
            .map_err(|e| format!("Failed to show cursor: {e}"))?;
        Ok(())
    }
}

impl Drop for BenchTui {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Draw a single TUI frame.
fn draw_frame(f: &mut Frame, results: &[PositionResult], total: usize, header: &[String]) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header.len() as u16 + 2), // Header
            Constraint::Length(3),                       // Progress bar
            Constraint::Min(10),                         // Results table
        ])
        .split(f.area());

    // Header
    let header_text: Vec<Line> = header
        .iter()
        .map(|l| Line::from(Span::styled(l.as_str(), Style::default().fg(Color::Cyan))))
        .collect();
    let header_widget = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Mujrim Benchmarker "),
    );
    f.render_widget(header_widget, chunks[0]);

    // Progress bar
    let completed = results.len();
    let correct = results.iter().filter(|r| r.correct).count();
    let pct = if total > 0 {
        (completed as f64 / total as f64 * 100.0) as u16
    } else {
        0
    };
    let label = format!("{completed}/{total} positions — {correct} correct");
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" Progress "))
        .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
        .percent(pct)
        .label(label);
    f.render_widget(gauge, chunks[1]);

    // Results table
    let header_row = Row::new(vec![
        Cell::from("#"),
        Cell::from("Status"),
        Cell::from("Found"),
        Cell::from("Expected"),
        Cell::from("Score"),
        Cell::from("Depth"),
        Cell::from("Nodes"),
        Cell::from("NPS"),
        Cell::from("Time"),
    ])
    .style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Yellow),
    );

    let rows: Vec<Row> = results
        .iter()
        .map(|r| {
            let status_style = if r.correct {
                Style::default().fg(Color::Green)
            } else if r.expected_move.is_empty() {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(Color::Red)
            };
            let status = if r.expected_move.is_empty() {
                "—"
            } else if r.correct {
                "✓"
            } else {
                "✗"
            };
            Row::new(vec![
                Cell::from(format!("{:>2}", r.index + 1)),
                Cell::from(Span::styled(status, status_style)),
                Cell::from(r.found_move.clone()),
                Cell::from(if r.expected_move.is_empty() {
                    "N/A".to_string()
                } else {
                    r.expected_move.clone()
                }),
                Cell::from(format!("{:>5}cp", r.score)),
                Cell::from(format!("{}", r.depth)),
                Cell::from(format_nps(r.nodes)),
                Cell::from(format_nps(r.nps)),
                Cell::from(format!("{}ms", r.elapsed.as_millis())),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(header_row)
    .block(Block::default().borders(Borders::ALL).title(" Results "));

    f.render_widget(table, chunks[2]);
}
