//! Shared command-line types.
use std::{fmt, str::FromStr};

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

/// Legend display style retained for source compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendStyle {
  Compact,
  Table,
  Verbose,
}

#[allow(clippy::should_implement_trait)]
impl LegendStyle {
  #[must_use]
  pub fn from_str(value: &str) -> Self {
    match value.to_ascii_lowercase().as_str() {
      "compact" => Self::Compact,
      "verbose" => Self::Verbose,
      _ => Self::Table,
    }
  }
}

/// Display format retained for source compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayFormat {
  Tree,
  Plain,
  Dashboard,
}

#[allow(clippy::should_implement_trait)]
impl DisplayFormat {
  #[must_use]
  pub fn from_str(value: &str) -> Self {
    match value.to_ascii_lowercase().as_str() {
      "plain" => Self::Plain,
      "dashboard" => Self::Dashboard,
      _ => Self::Tree,
    }
  }
}

/// Final summary style retained for source compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryStyle {
  Concise,
  Table,
  Full,
}

#[allow(clippy::should_implement_trait)]
impl SummaryStyle {
  #[must_use]
  pub fn from_str(value: &str) -> Self {
    match value.to_ascii_lowercase().as_str() {
      "table" => Self::Table,
      "full" => Self::Full,
      _ => Self::Concise,
    }
  }
}

/// Configuration for append-only stream monitoring.
///
/// The field layout is preserved from ROM 0.2 so existing struct literals keep
/// compiling. Presentation presets are selected separately with
/// [`crate::MonitorOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
  pub piping:           bool,
  pub silent:           bool,
  pub input_mode:       InputMode,
  pub show_timers:      bool,
  pub width:            Option<usize>,
  pub format:           DisplayFormat,
  pub legend_style:     LegendStyle,
  pub summary_style:    SummaryStyle,
  pub log_prefix_style: LogPrefixStyle,
  pub log_line_limit:   Option<usize>,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      piping:           false,
      silent:           false,
      input_mode:       InputMode::Human,
      show_timers:      true,
      width:            None,
      format:           DisplayFormat::Tree,
      legend_style:     LegendStyle::Table,
      summary_style:    SummaryStyle::Concise,
      log_prefix_style: LogPrefixStyle::Short,
      log_line_limit:   None,
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

#[allow(clippy::should_implement_trait)]
impl LogPrefixStyle {
  /// Parse the historical lenient API (unknown values select `short`).
  #[must_use]
  pub fn from_str(value: &str) -> Self {
    match value.to_ascii_lowercase().as_str() {
      "full" => Self::Full,
      "none" => Self::None,
      _ => Self::Short,
    }
  }
}

/// Error returned by strict command-line log-prefix parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLogPrefixStyleError(String);

impl fmt::Display for ParseLogPrefixStyleError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      formatter,
      "unknown log prefix style {:?}; expected short, full, or none",
      self.0
    )
  }
}

impl std::error::Error for ParseLogPrefixStyleError {}

impl FromStr for LogPrefixStyle {
  type Err = ParseLogPrefixStyleError;

  fn from_str(value: &str) -> Result<Self, Self::Err> {
    match value.to_ascii_lowercase().as_str() {
      "short" => Ok(Self::Short),
      "full" => Ok(Self::Full),
      "none" => Ok(Self::None),
      _ => Err(ParseLogPrefixStyleError(value.to_string())),
    }
  }
}
