use std::{
  io::{self, Write},
  process::{Child, ExitStatus},
  sync::{atomic::Ordering, mpsc::Receiver},
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
    DisableLineWrap,
    EnableLineWrap,
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
};

const MAX_STREAMED_LOG_RECORDS_PER_FRAME: usize = 32;

struct TerminalSession {
  stderr:          io::Stderr,
  stdout:          io::Stdout,
  stdout_is_tty:   bool,
  origin_y:        u16,
  terminal_width:  u16,
  terminal_height: u16,
  scroll_bottom:   Option<u16>,
  previous:        Option<rom_core::tui::Screen>,
  raw_mode:        bool,
}

fn graph_region_height(terminal_height: u16) -> u16 {
  if terminal_height == 0 {
    return 0;
  }
  let maximum = terminal_height.saturating_sub(1).max(1);
  terminal_height.div_ceil(3).max(4).min(maximum)
}

fn graph_height_budget(terminal_height: u16, mandatory_height: u16) -> u16 {
  graph_region_height(terminal_height)
    .max(mandatory_height)
    .min(terminal_height.saturating_sub(1))
}

impl TerminalSession {
  fn enter(stdout_is_tty: bool, interactive: bool) -> io::Result<Self> {
    let mut stderr = io::stderr();
    if interactive {
      terminal::enable_raw_mode()?;
    }

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
        stdout: io::stdout(),
        stdout_is_tty,
        origin_y,
        terminal_width,
        terminal_height,
        scroll_bottom: None,
        previous: None,
        raw_mode: interactive,
      })
    })();

    if entered.is_err() {
      let _ = execute!(
        io::stderr(),
        EndSynchronizedUpdate,
        SetAttribute(Attribute::Reset),
        cursor::Show
      );
      if interactive {
        let _ = terminal::disable_raw_mode();
      }
    }
    entered
  }

  fn invalidate(&mut self) {
    self.previous = None;
  }

  /// Write child stdout while no graph update can race it. On a terminal the
  /// scrolling margin keeps exact child bytes above the transient graph; when
  /// redirected, the bytes are written unchanged to the redirected stream.
  fn write_child_stdout(&mut self, chunks: &[Vec<u8>]) -> io::Result<()> {
    if chunks.is_empty() {
      return Ok(());
    }
    execute!(self.stderr, BeginSynchronizedUpdate)?;
    if self.stdout_is_tty {
      let scroll_bottom = self.origin_y.max(1);
      if self.scroll_bottom != Some(scroll_bottom) {
        queue!(
          self.stderr,
          Print(format!("\x1b[1;{scroll_bottom}r")),
          cursor::MoveTo(0, scroll_bottom.saturating_sub(1))
        )?;
        self.scroll_bottom = Some(scroll_bottom);
      }
      self.stderr.flush()?;
    }
    for chunk in chunks {
      self.stdout.write_all(chunk)?;
    }
    self.stdout.flush()?;
    execute!(self.stderr, EndSynchronizedUpdate)
  }

  fn draw(
    &mut self,
    state: &rom_core::state::RenderSnapshot,
    new_logs: &[String],
    config: &rom_core::tui::TuiConfig,
    options: rom_core::RenderOptions,
  ) -> io::Result<()> {
    let (width, terminal_height) = terminal::size()?;
    let minimum_height = u16::try_from(
      rom_core::tui::minimum_required_graph_rows_at_width_with_options(
        width, state, options,
      ),
    )
    .unwrap_or(u16::MAX);
    // Keep at least one physical row outside the graph. Mandatory graph rows
    // are clipped to this budget rather than taking away the streaming region.
    let soft_height = graph_height_budget(terminal_height, minimum_height);
    let screen = rom_core::tui::render_graph_screen_with_options(
      width,
      soft_height,
      state,
      config,
      options,
    );
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

    execute!(self.stderr, BeginSynchronizedUpdate, DisableLineWrap)?;
    let update_result: io::Result<()> = (|| {
      if terminal_resized {
        // Never clear the normal screen: it contains child stdout that is not
        // retained by ROM and therefore cannot be reconstructed. Clear only
        // the union of the old and new transient graph regions.
        let start = self.origin_y.min(origin_y).min(terminal_height);
        for y in start..terminal_height {
          queue!(
            self.stderr,
            cursor::MoveTo(0, y),
            Clear(ClearType::CurrentLine)
          )?;
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

      let scroll_bottom = self.origin_y.max(1);
      if self.scroll_bottom != Some(scroll_bottom) {
        queue!(
          self.stderr,
          Print(format!("\x1b[1;{scroll_bottom}r")),
          cursor::MoveTo(0, scroll_bottom.saturating_sub(1))
        )?;
        self.scroll_bottom = Some(scroll_bottom);
      }
      queue!(self.stderr, cursor::SavePosition)?;

      for line in new_logs {
        let rendered = rom_core::tui::render_streamed_log(width, line);
        for y in 0..rendered.height() {
          queue!(
            self.stderr,
            cursor::MoveTo(0, scroll_bottom.saturating_sub(1)),
            Clear(ClearType::CurrentLine)
          )?;
          rendered.write_ansi_row_trimmed(y, &mut self.stderr)?;
          // A newline at the physical bottom scrolls this row into normal
          // terminal history, leaving the transient region anchored in place.
          queue!(
            self.stderr,
            cursor::MoveTo(0, scroll_bottom.saturating_sub(1)),
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
        queue!(
          self.stderr,
          cursor::MoveTo(0, self.origin_y + y),
          Clear(ClearType::CurrentLine)
        )?;
        screen.write_ansi_row_trimmed(y, &mut self.stderr)?;
      }
      queue!(self.stderr, cursor::RestorePosition)?;
      self.stderr.flush()?;
      self.previous = Some(screen);
      Ok(())
    })();
    let end_result =
      execute!(self.stderr, EnableLineWrap, EndSynchronizedUpdate);

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
      Print("\x1b[r"),
      EnableLineWrap,
      cursor::Show,
      EndSynchronizedUpdate
    );
    if self.raw_mode {
      let _ = terminal::disable_raw_mode();
    }
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
    flush_all_logs: bool,
    config: &rom_core::tui::TuiConfig,
    options: rom_core::RenderOptions,
  ) -> io::Result<()> {
    if shared.screen_dirty.swap(false, Ordering::AcqRel) {
      terminal.invalidate();
    }
    let new_logs = {
      if silent {
        shared.log_store.drain_pending();
        Vec::new()
      } else if flush_all_logs {
        shared.log_store.drain_pending()
      } else {
        shared
          .log_store
          .drain_pending_up_to(MAX_STREAMED_LOG_RECORDS_PER_FRAME)
      }
    };
    let state = shared.state.lock().unwrap().render_snapshot();
    terminal.draw(&state, &new_logs, config, options)
  }
}

pub(super) fn run_tui_render_loop(
  child: &mut Child,
  shared: &MonitorShared,
  cfg: &WrapperConfig,
  stdout_receiver: Receiver<Vec<u8>>,
  stdout_is_tty: bool,
  interactive: bool,
) -> eyre::Result<MonitorOutcome> {
  let Ok(mut terminal) = TerminalSession::enter(stdout_is_tty, interactive)
  else {
    let relay = thread::spawn(move || -> io::Result<()> {
      let stdout = io::stdout();
      let mut stdout = stdout.lock();
      for chunk in stdout_receiver {
        stdout.write_all(&chunk)?;
        stdout.flush()?;
      }
      Ok(())
    });
    let outcome = run_streaming_render_loop(child, shared, cfg);
    relay
      .join()
      .map_err(|_| eyre::eyre!("stdout relay thread panicked"))?
      .map_err(rom_core::error::RomError::Io)?;
    return outcome;
  };

  let tui_config = rom_core::tui::TuiConfig {
    console: console_config(true),
  };
  let mut runtime = TuiRuntime;
  let mut status: Option<ExitStatus> = None;
  let mut requested_outcome = None;

  loop {
    if interactive
      && requested_outcome.is_none()
      && let Some(outcome) = handle_tui_events(child, &mut status)?
    {
      // Keep rendering until both pipe readers have delivered their bounded
      // queues. Cancellation must not turn already-produced records into a
      // best-effort stream.
      requested_outcome = Some(outcome);
    }

    if status.is_none() {
      status = child.try_wait().map_err(rom_core::error::RomError::Io)?;
    }

    populate_pending_dependencies(shared);

    let mut stdout_chunks = stdout_receiver.try_iter().collect::<Vec<_>>();
    let readers_done = shared.stderr_done.load(Ordering::Acquire)
      && shared.stdout_done.load(Ordering::Acquire);
    // The stdout reader publishes `stdout_done` only after its final send.
    // Re-drain after observing that release so a chunk sent between the first
    // drain and the acquire cannot be skipped by the completion break below.
    if readers_done {
      stdout_chunks.extend(stdout_receiver.try_iter());
    }
    terminal
      .write_child_stdout(&stdout_chunks)
      .map_err(rom_core::error::RomError::Io)?;

    runtime
      .draw(
        &mut terminal,
        shared,
        cfg.silent,
        status.is_some(),
        &tui_config,
        rom_core::RenderOptions { style: cfg.style },
      )
      .map_err(rom_core::error::RomError::Io)?;

    let logs_pending = !cfg.silent && shared.log_store.has_pending();
    if status.is_some() && readers_done && !logs_pending {
      break;
    }

    thread::sleep(Duration::from_millis(50));
  }

  drop(terminal);
  if let Some(outcome) = requested_outcome {
    return Ok(outcome);
  }
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
  use super::{graph_height_budget, graph_region_height};

  #[test]
  fn graph_region_targets_one_third_without_exceeding_terminal() {
    assert_eq!(graph_region_height(24), 8);
    assert_eq!(graph_region_height(25), 9);
    assert_eq!(graph_region_height(6), 4);
    assert_eq!(graph_region_height(2), 1);
    assert_eq!(graph_region_height(0), 0);
  }

  #[test]
  fn mandatory_graph_rows_are_clipped_to_reserve_streaming_space() {
    assert_eq!(graph_height_budget(24, 100), 23);
    assert_eq!(graph_height_budget(2, 100), 1);
    assert_eq!(graph_height_budget(1, 100), 0);
  }
}
