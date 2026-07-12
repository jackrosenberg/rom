//! ROM - Rust Output Monitor
pub use rom_core::{
  Config,
  InputMode,
  Monitor,
  MonitorOptions,
  ParsePresentationStyleError,
  PresentationStyle,
  RenderOptions,
  Result,
  RomError,
  console,
  create_monitor,
  create_monitor_with_options,
  error,
  graph,
  monitor,
  monitor_stream,
  monitor_stream_with_options,
  presentation,
  state,
  tui,
  types,
  update,
};

pub mod cli {
  pub use rom_cli::{Cli, Commands, parse_args_with_separator};
}

/// Run the CLI application with the provided arguments.
///
/// This is the main entry point for the CLI application.
pub fn run() -> eyre::Result<()> {
  rom_cli::run()
}
