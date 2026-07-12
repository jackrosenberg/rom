//! ROM core - stream monitoring, state tracking, and operations-console
//! rendering.
use std::io::{BufRead, Write};

pub mod console;
pub mod error;
pub mod graph;
pub mod monitor;
pub mod state;
pub mod tui;
pub mod types;
pub mod update;

pub use error::{Result, RomError};
pub use monitor::Monitor;
pub use types::{Config, InputMode};

/// Monitor a Nix output stream and append its final operations graph.
pub fn monitor_stream<R: BufRead, W: Write>(
  config: Config,
  reader: R,
  writer: W,
) -> Result<()> {
  let mut monitor = Monitor::new(config, writer)?;
  monitor.process_stream(reader)
}

/// Create an incremental monitor backed by an arbitrary writer.
pub fn create_monitor<W: Write>(
  config: Config,
  writer: W,
) -> Result<Monitor<W>> {
  Monitor::new(config, writer)
}
