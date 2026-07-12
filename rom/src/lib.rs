//! ROM - Rust Output Monitor
pub use rom_core::{
  Result,
  RomError,
  console,
  error,
  graph,
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
