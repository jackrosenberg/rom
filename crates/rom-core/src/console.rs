//! Shared rendering seam for the live operations console and its final graph.
use std::io::{self, Write};

use crate::state::{State, current_time};

/// Rendering options used by the operations console.
#[derive(Clone, Copy)]
pub struct ConsoleConfig {
  pub max_tree_depth:    usize,
  pub max_visible_lines: usize,
  pub use_color:         bool,
}

impl Default for ConsoleConfig {
  fn default() -> Self {
    Self {
      max_tree_depth:    10,
      max_visible_lines: 100,
      use_color:         true,
    }
  }
}

/// Format a duration in seconds for console status text.
#[must_use]
pub fn format_duration(secs: f64) -> String {
  if secs < 60.0 {
    format!("{secs:.0}s")
  } else if secs < 3600.0 {
    format!("{:.0}m{:.0}s", secs / 60.0, secs % 60.0)
  } else {
    format!("{:.0}h{:.0}m", secs / 3600.0, (secs % 3600.0) / 60.0)
  }
}

/// Write the stable post-build graph using the live console renderer.
pub fn write_final_graph<W: Write>(
  mut writer: W,
  state: &State,
  config: ConsoleConfig,
) -> io::Result<()> {
  let width = crossterm::terminal::size().map_or(100, |(width, _)| width);
  let tui_config = crate::tui::TuiConfig { console: config };
  let screen = crate::tui::render_final_graph_screen(width, state, &tui_config);
  for y in 0..screen.height() {
    if config.use_color {
      screen.write_ansi_row_trimmed(y, &mut writer)?;
    } else if let Some(row) = screen.row_text(y) {
      write!(writer, "{}", row.trim_end())?;
    }
    writeln!(writer)?;
  }

  write_finished_line(&mut writer, state, config.use_color)?;
  writeln!(writer)?;
  writer.flush()
}

fn write_finished_line<W: Write>(
  writer: &mut W,
  state: &State,
  use_color: bool,
) -> io::Result<()> {
  let failed = state.full_summary.failed_builds.len();
  let completed = state.full_summary.completed_builds.len();
  let nix_errors = state.nix_errors.len();
  let at = chrono::Local::now().format("%H:%M:%S");
  let duration = format_duration(current_time() - state.start_time);

  let line = if failed > 0 {
    let noun = if failed == 1 { "failure" } else { "failures" };
    format!(
      "{} {} at {} after {}",
      colored("✗", "1", use_color),
      colored(
        &format!("Exited after {failed} build {noun}"),
        "1",
        use_color
      ),
      colored(&at.to_string(), "1", use_color),
      colored(&duration, "1", use_color),
    )
  } else if nix_errors > 0 {
    let noun = if nix_errors == 1 { "error" } else { "errors" };
    format!(
      "{} {} at {} after {}",
      colored("✗", "1", use_color),
      colored(
        &format!("Exited with {nix_errors} nix {noun}"),
        "1",
        use_color
      ),
      colored(&at.to_string(), "1", use_color),
      colored(&duration, "1", use_color),
    )
  } else {
    let mut line = format!(
      "{} after {}",
      colored(&format!("Finished at {at}"), "2", use_color),
      colored(&duration, "2", use_color),
    );
    if completed > 0 {
      line.push_str(&format!("  {} {completed}", colored("✔", "2", use_color)));
    }
    line
  };

  writeln!(writer, "{line}")
}

fn colored(text: &str, ansi_color: &str, enabled: bool) -> String {
  if enabled {
    format!("\x1b[38;5;{ansi_color}m{text}\x1b[0m")
  } else {
    text.to_string()
  }
}
