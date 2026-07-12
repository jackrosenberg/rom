//! Shared rendering seam for the live operations console and its final graph.
use std::io::{self, Write};

use crate::state::{State, current_time};

/// Rendering options used by the operations console.
#[derive(Clone, Copy)]
pub struct ConsoleConfig {
  pub max_tree_depth:    usize,
  pub max_visible_lines: usize,
  pub use_color:         bool,
  /// Output width in terminal columns.
  ///
  /// The rendering seam deliberately does not inspect the process terminal so
  /// callers can safely use it with files, sockets, and in-memory writers.
  pub width:             u16,
}

impl Default for ConsoleConfig {
  fn default() -> Self {
    Self {
      max_tree_depth:    10,
      max_visible_lines: 100,
      use_color:         true,
      width:             100,
    }
  }
}

/// Format a duration in seconds for console status text.
#[must_use]
pub fn format_duration(secs: f64) -> String {
  let total_seconds = if secs.is_finite() {
    secs.max(0.0).round() as u64
  } else {
    0
  };
  if total_seconds < 60 {
    format!("{total_seconds}s")
  } else if total_seconds < 3_600 {
    format!("{}m{}s", total_seconds / 60, total_seconds % 60)
  } else {
    format!(
      "{}h{}m",
      total_seconds / 3_600,
      (total_seconds % 3_600) / 60
    )
  }
}

/// Write the stable post-build graph using the live console renderer.
pub fn write_final_graph<W: Write>(
  mut writer: W,
  state: &State,
  config: ConsoleConfig,
) -> io::Result<()> {
  let tui_config = crate::tui::TuiConfig { console: config };
  let screen =
    crate::tui::render_final_graph_screen(config.width, state, &tui_config);
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

#[cfg(test)]
mod tests {
  use super::format_duration;

  #[test]
  fn duration_rounds_once_before_splitting_components() {
    assert_eq!(format_duration(59.4), "59s");
    assert_eq!(format_duration(59.5), "1m0s");
    assert_eq!(format_duration(3_599.4), "59m59s");
    assert_eq!(format_duration(3_599.5), "1h0m");
    assert_eq!(format_duration(3_659.5), "1h1m");
    assert_eq!(format_duration(-1.0), "0s");
  }
}
