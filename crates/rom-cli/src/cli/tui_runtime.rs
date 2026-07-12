use std::{
  io::{self, Write},
  process::{Child, ExitStatus},
  sync::atomic::Ordering,
  thread,
  time::Duration,
};

use crossterm::{
  cursor,
  event::{self, KeyCode, KeyModifiers},
  execute,
  queue,
  style::{Attribute, Print, SetAttribute},
  terminal::{
    self,
    BeginSynchronizedUpdate,
    Clear,
    ClearType,
    EndSynchronizedUpdate,
  },
};

use super::{
  DEPENDENCY_POPULATE_BUDGET_PER_FRAME,
  MonitorOutcome,
  MonitorShared,
  WrapperConfig,
  console_config,
  run_streaming_render_loop,
  snapshot_logs,
};

const MAX_STREAMED_LOG_RECORDS_PER_FRAME: usize = 32;

struct TerminalSession {
  stderr:          io::Stderr,
  origin_y:        u16,
  terminal_width:  u16,
  terminal_height: u16,
  previous:        Option<rom_core::tui::Screen>,
}

fn graph_region_height(terminal_height: u16) -> u16 {
  if terminal_height == 0 {
    return 0;
  }
  let maximum = terminal_height.saturating_sub(1).max(1);
  terminal_height.div_ceil(3).max(4).min(maximum)
}

impl TerminalSession {
  fn enter() -> io::Result<Self> {
    let mut stderr = io::stderr();
    terminal::enable_raw_mode()?;

    let entered = (|| {
      let (terminal_width, terminal_height) = terminal::size()?;
      // Reserve only one row initially. The first render grows the compact
      // region to its actual content height without leaving a blank block.
      let height = terminal_height.min(1);
      let origin_y = terminal_height.saturating_sub(height);
      execute!(
        stderr,
        BeginSynchronizedUpdate,
        cursor::Hide,
        Print("\r\n".repeat(usize::from(height.saturating_sub(1)))),
        cursor::MoveTo(0, origin_y),
        EndSynchronizedUpdate
      )?;
      Ok(Self {
        stderr,
        origin_y,
        terminal_width,
        terminal_height,
        previous: None,
      })
    })();

    if entered.is_err() {
      let _ = execute!(
        io::stderr(),
        EndSynchronizedUpdate,
        SetAttribute(Attribute::Reset),
        cursor::Show
      );
      let _ = terminal::disable_raw_mode();
    }
    entered
  }

  fn invalidate(&mut self) {
    self.previous = None;
  }

  fn draw(
    &mut self,
    state: &rom_core::state::RenderSnapshot,
    logs: &[String],
    new_logs: &[String],
    config: &rom_core::tui::TuiConfig,
  ) -> io::Result<()> {
    let (width, terminal_height) = terminal::size()?;
    let minimum_height = u16::try_from(
      rom_core::tui::minimum_required_graph_rows_at_width(width, state),
    )
    .unwrap_or(u16::MAX);
    let soft_height = graph_region_height(terminal_height)
      .max(minimum_height)
      .min(terminal_height);
    let screen =
      rom_core::tui::render_graph_screen(width, soft_height, state, config);
    let height = screen.height();
    let origin_y = terminal_height.saturating_sub(height);
    let previous_height = self.previous.as_ref().map_or(
      self.terminal_height.saturating_sub(self.origin_y),
      |screen| screen.height(),
    );
    let terminal_resized =
      width != self.terminal_width || terminal_height != self.terminal_height;
    let region_resized = terminal_resized
      || origin_y != self.origin_y
      || self
        .previous
        .as_ref()
        .is_some_and(|screen| screen.height() != height);

    execute!(self.stderr, BeginSynchronizedUpdate)?;
    let update_result: io::Result<()> = (|| {
      if terminal_resized {
        // Existing normal-screen rows may have been reflowed by the terminal,
        // so their old cursor coordinates are no longer meaningful. Clearing
        // individual retained rows would leave wrapped graph fragments behind.
        queue!(self.stderr, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        if origin_y > 0 {
          let log_tail = rom_core::tui::render_retained_log_tail(
            width,
            origin_y,
            logs,
            new_logs.len(),
          );
          for y in 0..log_tail.height() {
            queue!(self.stderr, cursor::MoveTo(0, y))?;
            log_tail.write_ansi_row(y, &mut self.stderr)?;
          }
        }
        self.previous = None;
      } else if region_resized {
        self.clear_previous()?;
        self.previous = None;
      } else if !new_logs.is_empty() {
        // The newline-driven scroll moves the old graph naturally. Keep it on
        // screen during the burst, then repaint every graph row afterward.
        self.previous = None;
      }
      if region_resized && !terminal_resized && height > previous_height {
        queue!(
          self.stderr,
          cursor::MoveTo(0, terminal_height.saturating_sub(1)),
          Print("\r\n".repeat(usize::from(height - previous_height)))
        )?;
      }
      self.terminal_width = width;
      self.terminal_height = terminal_height;
      self.origin_y = origin_y;

      for line in new_logs {
        let rendered = rom_core::tui::render_streamed_log(width, line);
        for y in 0..rendered.height() {
          queue!(
            self.stderr,
            cursor::MoveTo(0, self.origin_y),
            Clear(ClearType::CurrentLine)
          )?;
          rendered.write_ansi_row(y, &mut self.stderr)?;
          // A newline at the physical bottom scrolls this row into normal
          // terminal history, leaving the transient region anchored in place.
          queue!(
            self.stderr,
            cursor::MoveTo(0, terminal_height.saturating_sub(1)),
            Print("\r\n")
          )?;
        }
      }

      for y in 0..screen.height() {
        let unchanged = self.previous.as_ref().is_some_and(|previous| {
          previous.width() == screen.width() && previous.row(y) == screen.row(y)
        });
        if unchanged {
          continue;
        }
        queue!(self.stderr, cursor::MoveTo(0, self.origin_y + y))?;
        screen.write_ansi_row(y, &mut self.stderr)?;
      }
      queue!(
        self.stderr,
        cursor::MoveTo(0, terminal_height.saturating_sub(1))
      )?;
      self.stderr.flush()?;
      self.previous = Some(screen);
      Ok(())
    })();
    let end_result = execute!(self.stderr, EndSynchronizedUpdate);

    update_result?;
    end_result
  }

  fn clear_previous(&mut self) -> io::Result<()> {
    let height = self.previous.as_ref().map_or(
      self.terminal_height.saturating_sub(self.origin_y),
      rom_core::tui::Screen::height,
    );
    for y in 0..height {
      queue!(
        self.stderr,
        cursor::MoveTo(0, self.origin_y.saturating_add(y)),
        Clear(ClearType::CurrentLine)
      )?;
    }
    Ok(())
  }
}

impl Drop for TerminalSession {
  fn drop(&mut self) {
    let _ = execute!(
      self.stderr,
      BeginSynchronizedUpdate,
      cursor::MoveTo(0, self.origin_y),
      Clear(ClearType::FromCursorDown),
      SetAttribute(Attribute::Reset),
      cursor::Show,
      EndSynchronizedUpdate
    );
    let _ = terminal::disable_raw_mode();
  }
}

#[derive(Default)]
struct TuiRuntime;

impl TuiRuntime {
  fn draw(
    &mut self,
    terminal: &mut TerminalSession,
    shared: &MonitorShared,
    silent: bool,
    config: &rom_core::tui::TuiConfig,
  ) -> io::Result<()> {
    if shared.screen_dirty.swap(false, Ordering::AcqRel) {
      terminal.invalidate();
    }
    let new_logs = {
      let mut logs = shared.log_store.lock().unwrap();
      if silent {
        logs.drain_pending();
        Vec::new()
      } else {
        logs.drain_pending_up_to(MAX_STREAMED_LOG_RECORDS_PER_FRAME)
      }
    };
    let state = shared.state.lock().unwrap().render_snapshot();
    let logs = snapshot_logs(shared, silent);
    terminal.draw(&state, &logs, &new_logs, config)
  }
}

pub(super) fn run_tui_render_loop(
  child: &mut Child,
  shared: &MonitorShared,
  cfg: &WrapperConfig,
) -> eyre::Result<MonitorOutcome> {
  let Ok(mut terminal) = TerminalSession::enter() else {
    return run_streaming_render_loop(child, shared, cfg);
  };

  let tui_config = rom_core::tui::TuiConfig {
    console: console_config(true),
  };
  let mut runtime = TuiRuntime;
  let mut status: Option<ExitStatus> = None;

  loop {
    if let Some(outcome) = handle_tui_events(child, &mut status)? {
      drop(terminal);
      return Ok(outcome);
    }

    if status.is_none() {
      status = child.try_wait().map_err(rom_core::error::RomError::Io)?;
    }

    populate_pending_dependencies(shared);

    runtime
      .draw(&mut terminal, shared, cfg.silent, &tui_config)
      .map_err(rom_core::error::RomError::Io)?;

    let logs_pending =
      !cfg.silent && shared.log_store.lock().unwrap().has_pending();
    if status.is_some()
      && shared.stderr_done.load(Ordering::Acquire)
      && !logs_pending
    {
      break;
    }

    thread::sleep(Duration::from_millis(50));
  }

  drop(terminal);
  let exit_code = status.and_then(|status| status.code()).unwrap_or(1);
  Ok(MonitorOutcome::Completed(exit_code))
}

fn handle_tui_events(
  child: &mut Child,
  status: &mut Option<ExitStatus>,
) -> eyre::Result<Option<MonitorOutcome>> {
  while event::poll(Duration::from_millis(0))
    .map_err(rom_core::error::RomError::Io)?
  {
    let event = event::read().map_err(rom_core::error::RomError::Io)?;
    if let Some(key) = event.as_key_press_event() {
      let cancel = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c' | 'C'))
        || matches!(key.code, KeyCode::Char('q' | 'Q'));
      if cancel {
        return cancel_child(child, status).map(Some);
      }
    }
  }
  Ok(None)
}

fn cancel_child(
  child: &mut Child,
  status: &mut Option<ExitStatus>,
) -> eyre::Result<MonitorOutcome> {
  if let Some(status) = status.as_ref() {
    return Ok(MonitorOutcome::Completed(status.code().unwrap_or(1)));
  }
  if let Some(done) = child.try_wait().map_err(rom_core::error::RomError::Io)? {
    let exit_code = done.code().unwrap_or(1);
    *status = Some(done);
    return Ok(MonitorOutcome::Completed(exit_code));
  }
  child.kill().map_err(rom_core::error::RomError::Io)?;
  let killed = child.wait().map_err(rom_core::error::RomError::Io)?;
  *status = Some(killed);
  Ok(MonitorOutcome::Cancelled)
}

fn populate_pending_dependencies(shared: &MonitorShared) {
  let mut state = shared.state.lock().unwrap();
  let mut graph = shared.graph.lock().unwrap();
  graph.populate_pending(&mut state, DEPENDENCY_POPULATE_BUDGET_PER_FRAME);
}

#[cfg(test)]
mod tests {
  use super::graph_region_height;

  #[test]
  fn graph_region_targets_one_third_without_exceeding_terminal() {
    assert_eq!(graph_region_height(24), 8);
    assert_eq!(graph_region_height(25), 9);
    assert_eq!(graph_region_height(6), 4);
    assert_eq!(graph_region_height(2), 1);
    assert_eq!(graph_region_height(0), 0);
  }
}
