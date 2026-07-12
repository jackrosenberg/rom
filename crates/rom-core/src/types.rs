//! Shared command-line types.
use std::{convert::Infallible, str::FromStr};

/// Input syntax accepted by the stream monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
  /// Human-readable output produced by Nix's default log formatter.
  #[default]
  Human,
  /// Unprefixed `internal-json` records. Prefixed `@nix ` records are accepted
  /// in either mode.
  Json,
}

/// Configuration for append-only stream monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
  /// Input syntax for lines which do not carry the `@nix ` marker.
  pub input_mode: InputMode,
  /// Emit ANSI colors in the final operations graph.
  pub use_color:  bool,
  /// Width of the final operations graph in terminal columns.
  pub width:      u16,
  /// Suppress passthrough log lines while retaining state updates.
  pub silent:     bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      input_mode: InputMode::Human,
      use_color:  false,
      width:      100,
      silent:     false,
    }
  }
}

/// Log prefix style for build logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogPrefixStyle {
  /// Just package name (pname).
  Short,
  /// Full derivation name with version.
  Full,
  /// No prefix.
  None,
}

impl FromStr for LogPrefixStyle {
  type Err = Infallible;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ok(match s.to_lowercase().as_str() {
      "full" => Self::Full,
      "none" => Self::None,
      _ => Self::Short,
    })
  }
}
