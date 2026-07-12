pub(super) use std::collections::HashSet;
use std::{convert::Infallible, fmt};

pub(super) use rom_core::{
  console::ConsoleConfig,
  state::{
    BuildFail,
    BuildInfo,
    BuildStatus,
    CompletedTransferInfo,
    Derivation,
    DerivationId,
    FailType,
    InputDerivation,
    State,
    StorePath,
    StorePathId,
    TransferInfo,
    current_time,
  },
  tui::{
    Attribute,
    Color,
    Screen,
    TuiConfig,
    render_final_graph_screen,
    render_graph_screen,
  },
};

pub(super) const GRAPH_LINE_COLOR: Color = Color::Rgb {
  r: 47,
  g: 104,
  b: 126,
};
pub(super) const MOSS_GREEN: Color = Color::Rgb {
  r: 63,
  g: 236,
  b: 208,
};
pub(super) const MUTED_RED: Color = Color::Rgb {
  r: 234,
  g: 65,
  b: 83,
};
pub(super) const MUTED_YELLOW: Color = Color::Rgb {
  r: 255,
  g: 179,
  b: 76,
};

pub(super) struct TestBackend {
  screen: Screen,
}

impl TestBackend {
  pub(super) fn new(width: u16, height: u16) -> Self {
    Self {
      screen: Screen::new(width, height),
    }
  }

  pub(super) fn buffer(&self) -> &Screen {
    &self.screen
  }
}

impl fmt::Display for TestBackend {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    self.screen.fmt(formatter)
  }
}

pub(super) struct Terminal {
  backend: TestBackend,
}

impl Terminal {
  pub(super) fn new(backend: TestBackend) -> Result<Self, Infallible> {
    Ok(Self { backend })
  }

  pub(super) fn draw(
    &mut self,
    render: impl FnOnce(&mut Screen),
  ) -> Result<(), Infallible> {
    render(&mut self.backend.screen);
    Ok(())
  }

  pub(super) fn backend(&self) -> &TestBackend {
    &self.backend
  }
}

pub(super) fn draw(
  screen: &mut Screen,
  state: &rom_core::state::RenderSnapshot,
  config: &TuiConfig,
) {
  *screen = render_graph_screen(screen.width(), screen.height(), state, config);
}

pub(super) fn tui_config() -> TuiConfig {
  TuiConfig {
    console: ConsoleConfig::default(),
  }
}

pub(super) fn running_state() -> State {
  let mut state = State::new();
  let drv_id = add_derivation(&mut state, "hello-1.0");
  state.update_build_status(
    drv_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time(),
      host:        cognos::Host::Localhost,
      activity_id: None,
    }),
  );
  state.forest_roots.push(drv_id);
  state
}

pub(super) fn add_derivation(state: &mut State, name: &str) -> DerivationId {
  let drv = Derivation::parse(&format!(
    "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-{name}.drv"
  ))
  .unwrap();
  state.get_or_create_derivation_id(drv)
}

pub(super) fn add_output_path(
  state: &mut State,
  drv_id: DerivationId,
  name: &str,
) -> StorePathId {
  let path_id = add_store_path(state, name);
  state.get_store_path_info_mut(path_id).unwrap().producer = Some(drv_id);
  state
    .get_derivation_info_mut(drv_id)
    .unwrap()
    .outputs
    .insert(cognos::OutputName::parse("out"), path_id);
  path_id
}

pub(super) fn add_store_path(state: &mut State, name: &str) -> StorePathId {
  let path = StorePath::parse(&format!(
    "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-{name}"
  ))
  .unwrap();
  state.get_or_create_store_path_id(path)
}

pub(super) fn row_text(terminal: &Terminal, row: u16) -> String {
  terminal
    .backend()
    .buffer()
    .row_text(row)
    .unwrap_or_default()
}

pub(super) fn row_containing(terminal: &Terminal, needle: &str) -> Option<u16> {
  let buffer = terminal.backend().buffer();
  (0..buffer.height()).find(|row| row_text(terminal, *row).contains(needle))
}
