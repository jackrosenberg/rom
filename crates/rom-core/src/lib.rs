//! ROM core - state tracking and operations-console rendering.
pub mod console;
pub mod error;
pub mod graph;
pub mod state;
pub mod tui;
pub mod types;
pub mod update;

pub use error::{Result, RomError};
