//! Presentation presets and the internal rendering boundary.
use std::{fmt, str::FromStr};

/// A named presentation preset accepted by the CLI `--style` flag and the
/// library option types.
///
/// Presets are deliberately data rather than renderer implementations. This
/// keeps terminal ownership in the CLI while allowing renderers to evolve
/// behind a stable selection API. The canonical spellings are returned by
/// [`PresentationStyle::as_str`]; the parser also accepts `tree` for
/// [`PresentationStyle::Connected`], `table` for
/// [`PresentationStyle::TableSummary`], and `full` for
/// [`PresentationStyle::FullSummary`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PresentationStyle {
  /// Default connected activity tree with the standard build/cache footer.
  #[default]
  Connected,
  /// Connected activity tree with a single-line summary footer.
  Compact,
  /// Connected activity tree with expanded status, cache, and outcome details.
  Verbose,
  /// Minimal flat text view without graph chrome.
  Plain,
  /// Dashboard-oriented live status view.
  Dashboard,
  /// Connected graph plus a tabular final summary.
  TableSummary,
  /// Connected graph plus the most complete final summary.
  FullSummary,
}

impl PresentationStyle {
  /// Canonical command-line spelling of this preset.
  #[must_use]
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Connected => "connected",
      Self::Compact => "compact",
      Self::Verbose => "verbose",
      Self::Plain => "plain",
      Self::Dashboard => "dashboard",
      Self::TableSummary => "table-summary",
      Self::FullSummary => "full-summary",
    }
  }
}

impl fmt::Display for PresentationStyle {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(self.as_str())
  }
}

/// Error returned when a presentation preset name is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePresentationStyleError {
  value: String,
}

impl fmt::Display for ParsePresentationStyleError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(formatter, "unknown presentation style `{}`", self.value)
  }
}

impl std::error::Error for ParsePresentationStyleError {}

impl FromStr for PresentationStyle {
  type Err = ParsePresentationStyleError;

  fn from_str(value: &str) -> Result<Self, Self::Err> {
    match value {
      "connected" | "tree" => Ok(Self::Connected),
      "compact" => Ok(Self::Compact),
      "verbose" => Ok(Self::Verbose),
      "plain" => Ok(Self::Plain),
      "dashboard" => Ok(Self::Dashboard),
      "table-summary" | "table" => Ok(Self::TableSummary),
      "full-summary" | "full" => Ok(Self::FullSummary),
      _ => {
        Err(ParsePresentationStyleError {
          value: value.to_string(),
        })
      },
    }
  }
}

/// Options shared by live and final presentation renderers.
///
/// This additive options type is intentionally small and copyable so callers
/// can opt into presentation presets without taking terminal ownership or
/// depending on renderer internals. [`RenderOptions::default`] preserves the
/// historical connected style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderOptions {
  /// Named presentation preset to use when rendering.
  pub style: PresentationStyle,
}

impl From<PresentationStyle> for RenderOptions {
  fn from(style: PresentationStyle) -> Self {
    Self { style }
  }
}

/// Presentation options used by an incremental [`crate::Monitor`].
///
/// Existing callers can keep constructing monitors with the default APIs. New
/// callers can pass this type to [`crate::create_monitor_with_options`] or
/// [`crate::monitor_stream_with_options`] to select one of the seven
/// [`PresentationStyle`] presets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonitorOptions {
  /// Options for the monitor's live and final rendering passes.
  pub render: RenderOptions,
}

impl From<RenderOptions> for MonitorOptions {
  fn from(render: RenderOptions) -> Self {
    Self { render }
  }
}

impl From<PresentationStyle> for MonitorOptions {
  fn from(style: PresentationStyle) -> Self {
    Self {
      render: style.into(),
    }
  }
}

/// Colors used by the connected renderer.
pub(crate) struct Palette;

impl Palette {
  pub(crate) const TEXT_PRIMARY: crate::tui::Color = crate::tui::Color::Rgb {
    r: 231,
    g: 236,
    b: 248,
  };
  pub(crate) const TEXT_MUTED: crate::tui::Color = crate::tui::Color::Rgb {
    r: 135,
    g: 148,
    b: 173,
  };
  pub(crate) const GRAPH_LINE: crate::tui::Color = crate::tui::Color::Rgb {
    r: 75,
    g: 88,
    b: 112,
  };
  pub(crate) const TABLE_HEADER: crate::tui::Color = crate::tui::Color::Rgb {
    r: 174,
    g: 187,
    b: 221,
  };
  pub(crate) const MOSS_GREEN: crate::tui::Color = crate::tui::Color::Rgb {
    r: 109,
    g: 145,
    b: 229,
  };
  pub(crate) const BUILT_GREEN: crate::tui::Color = crate::tui::Color::Rgb {
    r: 119,
    g: 190,
    b: 146,
  };
  pub(crate) const DOWNLOAD_BLUE: crate::tui::Color = crate::tui::Color::Rgb {
    r: 85,
    g: 180,
    b: 204,
  };
  pub(crate) const UPLOAD_PURPLE: crate::tui::Color = crate::tui::Color::Rgb {
    r: 169,
    g: 138,
    b: 221,
  };
  pub(crate) const MUTED_RED: crate::tui::Color = crate::tui::Color::Rgb {
    r: 224,
    g: 111,
    b: 114,
  };
  pub(crate) const MUTED_YELLOW: crate::tui::Color = crate::tui::Color::Rgb {
    r: 215,
    g: 155,
    b: 91,
  };
}

/// Glyphs used by connected presentation chrome.
pub(crate) struct Glyphs;

impl Glyphs {
  pub(crate) const FAILURE: &'static str = "✗";
  pub(crate) const SUCCESS: &'static str = "✔";
  pub(crate) const SPINNER_FRAMES: &'static [&'static str] =
    &["⢄", "⢂", "⢁", "⡁", "⡈", "⡐", "⡠"];
}
