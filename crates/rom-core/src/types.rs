//! Shared command-line types.
use std::{convert::Infallible, str::FromStr};

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
