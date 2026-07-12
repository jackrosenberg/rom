//! ROM core - stream monitoring, state tracking, and operations-console
//! rendering.
//!
//! The default APIs (`monitor_stream`, `create_monitor`, and `Monitor::new`)
//! preserve ROM's historical connected presentation. Use
//! [`PresentationStyle`], [`RenderOptions`], and [`MonitorOptions`] with the
//! `_with_options` constructors to select one of the seven presentation presets
//! also exposed by the CLI `--style` flag.
use std::io::{BufRead, Write};

pub mod console;
pub mod error;
pub mod graph;
pub mod monitor;
pub mod presentation;
pub mod state;
pub mod tui;
pub mod types;
pub mod update;

pub use error::{Result, RomError};
pub use monitor::Monitor;
pub use presentation::{
  MonitorOptions,
  ParsePresentationStyleError,
  PresentationStyle,
  RenderOptions,
};
pub use types::{Config, InputMode};

/// Monitor a Nix output stream and append its final operations graph.
pub fn monitor_stream<R: BufRead, W: Write>(
  config: Config,
  reader: R,
  writer: W,
) -> Result<()> {
  monitor_stream_with_options(config, MonitorOptions::default(), reader, writer)
}

/// Monitor a Nix output stream using explicit presentation options.
///
/// This is the non-breaking counterpart to [`monitor_stream`]; pass
/// [`MonitorOptions::default`] for identical behavior, or convert a
/// [`PresentationStyle`] into [`MonitorOptions`] to choose another preset.
pub fn monitor_stream_with_options<R: BufRead, W: Write>(
  config: Config,
  options: MonitorOptions,
  reader: R,
  writer: W,
) -> Result<()> {
  let mut monitor = Monitor::new_with_options(config, options, writer)?;
  monitor.process_stream(reader)
}

/// Create an incremental monitor backed by an arbitrary writer.
pub fn create_monitor<W: Write>(
  config: Config,
  writer: W,
) -> Result<Monitor<W>> {
  create_monitor_with_options(config, MonitorOptions::default(), writer)
}

/// Create an incremental monitor with explicit presentation options.
///
/// This is the non-breaking counterpart to [`create_monitor`]; pass
/// [`MonitorOptions::default`] for identical behavior, or convert a
/// [`PresentationStyle`] into [`MonitorOptions`] to choose another preset.
pub fn create_monitor_with_options<W: Write>(
  config: Config,
  options: MonitorOptions,
  writer: W,
) -> Result<Monitor<W>> {
  Monitor::new_with_options(config, options, writer)
}
