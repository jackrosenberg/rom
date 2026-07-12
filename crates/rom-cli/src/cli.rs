//! CLI interface for ROM
mod tui_runtime;

use std::{
  collections::HashMap,
  io::{self, BufRead, BufReader, IsTerminal, Read, Write},
  path::PathBuf,
  process::{Child, Command, Stdio},
  sync::{
    Arc,
    Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
  },
  thread,
  time::Duration,
};

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::log_store::{LogStore, post_tui_failure_error_lines};

pub(super) const DEPENDENCY_POPULATE_BUDGET_PER_FRAME: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorOutcome {
  Completed(i32),
  Cancelled,
}

impl MonitorOutcome {
  fn exit_code(self) -> i32 {
    match self {
      Self::Completed(code) => code,
      Self::Cancelled => 130,
    }
  }

  fn is_cancelled(self) -> bool {
    matches!(self, Self::Cancelled)
  }
}

#[derive(Debug, Parser)]
#[command(
  name = "rom",
  version,
  about = "ROM - A Nix build output monitor",
  after_help = "With no COMMAND, reads Nix output from standard input."
)]
pub struct Cli {
  #[command(subcommand)]
  pub command: Option<Commands>,

  /// Parse unprefixed Nix internal-json records from stdin
  #[arg(long, global = true)]
  pub json: bool,

  /// Minimal output
  #[arg(long, global = true)]
  pub silent: bool,

  /// Log prefix style: short, full, none
  #[arg(long, global = true, default_value = "short")]
  pub log_prefix: String,

  /// Nix-family evaluator to use. Auto-detected by default
  #[arg(long, global = true)]
  pub platform: Option<String>,

  /// Increase verbosity; controls nix log level and rom diagnostic output.
  /// Repeatable: -v (info), -vv (debug), -vvv (trace)
  #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
  pub verbose: u8,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
  /// Run nix build with monitoring
  Build {
    /// Packages or flake expressions to build
    packages: Vec<String>,

    /// Extra flags to pass directly to nix
    #[arg(last = true)]
    nix_flags: Vec<String>,
  },

  /// Run nix shell with monitoring
  Shell {
    /// Packages or flake expressions
    packages: Vec<String>,

    /// Extra flags to pass directly to nix
    #[arg(last = true)]
    nix_flags: Vec<String>,
  },

  /// Run nix develop with monitoring
  Develop {
    /// Packages or flake expressions
    packages: Vec<String>,

    /// Extra flags to pass directly to nix
    #[arg(last = true)]
    nix_flags: Vec<String>,
  },
}

pub(super) struct WrapperConfig {
  platform:         cognos::Platform,
  silent:           bool,
  verbose:          u8,
  log_prefix_style: rom_core::types::LogPrefixStyle,
}

/// Run the CLI application
pub fn run() -> eyre::Result<()> {
  let cli = Cli::parse();

  // Initialize tracing based on verbosity level; RUST_LOG overrides
  let default_filter = match cli.verbose {
    0 => "rom=warn",
    1 => "rom=info",
    2 => "rom=debug",
    _ => "rom=trace",
  };
  tracing_subscriber::fmt()
    .with_env_filter(
      EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter)),
    )
    .with_target(false)
    .with_writer(std::io::stderr)
    .init();

  let log_prefix_style: rom_core::types::LogPrefixStyle =
    cli.log_prefix.parse()?;
  let silent = cli.silent;
  let verbose = cli.verbose;
  let platform = cli
    .platform
    .as_deref()
    .and_then(|platform| platform.parse().ok())
    .unwrap_or_else(cognos::Platform::detect);

  // Check if we're being called as a symlink (rom-build, rom-shell)
  let program_name = std::env::args()
    .next()
    .and_then(|path| {
      PathBuf::from(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(std::string::ToString::to_string)
    })
    .unwrap_or_else(|| "rom".to_string());

  let cfg = WrapperConfig {
    platform,
    silent,
    verbose,
    log_prefix_style,
  };

  match (&program_name[..], cli.command) {
    // rom-build symlink
    ("rom-build", _) => {
      let args: Vec<String> = std::env::args().skip(1).collect();
      let (packages, nix_flags) = parse_args_with_separator(&args);
      run_build_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom-shell symlink
    ("rom-shell", _) => {
      let args: Vec<String> = std::env::args().skip(1).collect();
      let (packages, nix_flags) = parse_args_with_separator(&args);
      run_shell_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom build command
    (
      _,
      Some(Commands::Build {
        packages,
        nix_flags,
      }),
    ) => {
      if packages.is_empty() {
        eyre::bail!(
          "No package or flake specified for build\nUsage: rom build \
           <package> [-- <flags>]\nExample: rom build nixpkgs#hello -- \
           --rebuild"
        );
      }
      run_build_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom shell command
    (
      _,
      Some(Commands::Shell {
        packages,
        nix_flags,
      }),
    ) => {
      if packages.is_empty() {
        eyre::bail!(
          "No package or flake specified for shell\nUsage: rom shell \
           <package> [-- <flags>]\nExample: rom shell nixpkgs#python3 -- \
           --pure"
        );
      }
      run_shell_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom develop command
    (
      _,
      Some(Commands::Develop {
        packages,
        nix_flags,
      }),
    ) => {
      if packages.is_empty() {
        eyre::bail!(
          "No package or flake specified for develop\nUsage: rom develop \
           <package> [-- <flags>]\nExample: rom develop nixpkgs#hello -- \
           --impure"
        );
      }
      run_develop_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    (_, None) => {
      let stdin = io::stdin();
      let stdout = io::stdout();
      let use_color = stdout.is_terminal();
      let width = if use_color {
        crossterm::terminal::size().map_or(100, |(width, _)| width)
      } else {
        100
      };
      let config = rom_core::Config {
        input_mode: if cli.json {
          rom_core::InputMode::Json
        } else {
          rom_core::InputMode::Human
        },
        use_color,
        width,
        silent,
      };
      rom_core::monitor_stream(config, stdin.lock(), stdout.lock())?;
      Ok(())
    },
  }
}

/// Parse arguments, separating those before and after `--`
/// Returns (`args_before_separator`, `args_after_separator`)
///
/// Everything before `--` is for the package name and rom arguments.
/// Everything after `--` goes directly to nix.
#[must_use]
pub fn parse_args_with_separator(
  args: &[String],
) -> (Vec<String>, Vec<String>) {
  if let Some(pos) = args.iter().position(|arg| arg == "--") {
    // Arguments before -- are package/rom args
    let before = args[..pos].to_vec();

    // Arguments after -- go to nix
    let after = args[pos + 1..].to_vec();
    (before, after)
  } else {
    // No separator found - all args are package/rom args for backward
    // compatibility
    (args.to_vec(), Vec::new())
  }
}

/// Returns the nix verbosity flag for the given level.
/// Always produces at least `-v` so build events are emitted via
/// `--log-format internal-json`.
fn nix_verbosity_flag(verbose: u8) -> String {
  format!("-{}", "v".repeat(verbose.max(1) as usize))
}

fn run_build_wrapper(
  packages: Vec<String>,
  nix_flags: Vec<String>,
  cfg: &WrapperConfig,
) -> eyre::Result<()> {
  if packages.is_empty() {
    eyre::bail!(
      "No package or flake specified for build\nUsage: rom build <package> \
       [-- <flags>]\nExample: rom build nixpkgs#hello -- --rebuild"
    );
  }

  let mut cmd_args = vec![
    "build".to_string(),
    nix_verbosity_flag(cfg.verbose),
    "--log-format".to_string(),
    "internal-json".to_string(),
  ];
  cmd_args.extend(packages);
  cmd_args.extend(nix_flags);

  let exit_code = run_monitored_command(cfg.platform.binary(), cmd_args, cfg)?;
  if exit_code != 0 {
    std::process::exit(exit_code);
  }
  Ok(())
}

fn run_shell_wrapper(
  packages: Vec<String>,
  nix_flags: Vec<String>,
  cfg: &WrapperConfig,
) -> eyre::Result<()> {
  if packages.is_empty() {
    eyre::bail!(
      "No package or flake specified for shell\nUsage: rom shell <package> \
       [-- <flags>]\nExample: rom shell nixpkgs#python3 -- --pure"
    );
  }

  // First pass: monitor the build phase with --command exit
  let mut monitor_args = vec![
    "shell".to_string(),
    nix_verbosity_flag(cfg.verbose),
    "--log-format".to_string(),
    "internal-json".to_string(),
  ];
  let shell_args: Vec<String> =
    packages.iter().chain(nix_flags.iter()).cloned().collect();
  monitor_args.extend(replace_command_with_exit(&shell_args));

  let exit_code =
    run_monitored_command(cfg.platform.binary(), monitor_args, cfg)?;

  if exit_code != 0 {
    std::process::exit(exit_code);
  }

  // Second pass: enter the actual shell
  if !cfg.silent {
    let mut shell_args = vec!["shell".to_string()];
    shell_args.extend(packages);
    shell_args.extend(nix_flags);

    let status = Command::new(cfg.platform.binary())
      .args(&shell_args)
      .status()
      .map_err(rom_core::error::RomError::Io)?;

    std::process::exit(status.code().unwrap_or(1));
  }

  Ok(())
}

fn run_develop_wrapper(
  packages: Vec<String>,
  nix_flags: Vec<String>,
  cfg: &WrapperConfig,
) -> eyre::Result<()> {
  // First pass: monitor with --command true
  let mut monitor_args = vec![
    "develop".to_string(),
    nix_verbosity_flag(cfg.verbose),
    "--log-format".to_string(),
    "internal-json".to_string(),
    "--command".to_string(),
    "true".to_string(),
  ];
  monitor_args.extend(packages.clone());
  monitor_args.extend(nix_flags.clone());

  let exit_code =
    run_monitored_command(cfg.platform.binary(), monitor_args, cfg)?;

  if exit_code != 0 {
    std::process::exit(exit_code);
  }

  // Second pass: enter the actual dev shell
  if !cfg.silent {
    let mut develop_args = vec!["develop".to_string()];
    develop_args.extend(packages);
    develop_args.extend(nix_flags);

    let status = Command::new(cfg.platform.binary())
      .args(&develop_args)
      .status()
      .map_err(rom_core::error::RomError::Io)?;

    std::process::exit(status.code().unwrap_or(1));
  }

  Ok(())
}

pub(super) struct MonitorShared {
  pub(super) state:        Arc<Mutex<rom_core::state::State>>,
  pub(super) graph:        Arc<Mutex<rom_core::graph::GraphIndexer>>,
  pub(super) log_store:    Arc<LogStore>,
  pub(super) stderr_done:  Arc<AtomicBool>,
  pub(super) stdout_done:  Arc<AtomicBool>,
  pub(super) screen_dirty: Arc<AtomicBool>,
}

impl MonitorShared {
  fn new() -> Self {
    Self {
      state:        Arc::new(Mutex::new(rom_core::state::State::new())),
      graph:        Arc::new(Mutex::new(rom_core::graph::GraphIndexer::new())),
      log_store:    Arc::new(LogStore::new()),
      stderr_done:  Arc::new(AtomicBool::new(false)),
      stdout_done:  Arc::new(AtomicBool::new(false)),
      screen_dirty: Arc::new(AtomicBool::new(false)),
    }
  }
}

fn run_monitored_command(
  command: &str,
  args: Vec<String>,
  cfg: &WrapperConfig,
) -> eyre::Result<i32> {
  let mut child = Command::new(command)
    .args(&args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(rom_core::error::RomError::Io)?;

  let stderr = child.stderr.take().expect("Failed to capture stderr");
  let stdout = child.stdout.take().expect("Failed to capture stdout");
  let use_tui = io::stdin().is_terminal() && io::stderr().is_terminal();
  let shared = MonitorShared::new();
  let stderr_thread = spawn_stderr_reader(stderr, &shared, cfg, use_tui);
  let (stdout_thread, stdout_receiver) = if use_tui {
    let (sender, receiver) = std::sync::mpsc::sync_channel(32);
    (
      spawn_queued_stdout_reader(stdout, sender, shared.stdout_done.clone()),
      Some(receiver),
    )
  } else {
    (
      spawn_stdout_reader(
        stdout,
        shared.screen_dirty.clone(),
        shared.stdout_done.clone(),
      ),
      None,
    )
  };

  let outcome = if let Some(stdout_receiver) = stdout_receiver {
    tui_runtime::run_tui_render_loop(
      &mut child,
      &shared,
      cfg,
      stdout_receiver,
      io::stdout().is_terminal(),
    )
  } else {
    run_streaming_render_loop(&mut child, &shared, cfg)
  };

  // A monitor failure must not detach a still-running evaluator or its pipe
  // readers. Reap the child before joining the workers on every exit path.
  if outcome.is_err() {
    let _ = child.kill();
    let _ = child.wait();
  }
  // Cancellation or a rendering error can otherwise leave the parser blocked
  // behind the bounded log hand-off while this thread waits to join it.
  shared.log_store.close();

  let stderr_result = stderr_thread
    .join()
    .map_err(|_| eyre::eyre!("stderr reader thread panicked"));
  let stdout_result = stdout_thread
    .join()
    .map_err(|_| eyre::eyre!("stdout reader thread panicked"));

  let outcome = outcome?;
  stderr_result?.map_err(rom_core::error::RomError::Io)?;
  stdout_result?.map_err(rom_core::error::RomError::Io)?;
  // Live TUI logs now remain in normal scrollback, so replaying selected
  // failure lines after teardown would print parsed stderr records twice.
  finish_monitored_command(&shared, cfg, outcome, false)?;

  Ok(outcome.exit_code())
}

fn spawn_stderr_reader<R: Read + Send + 'static>(
  stderr: R,
  shared: &MonitorShared,
  cfg: &WrapperConfig,
  preserve_log_ansi: bool,
) -> thread::JoinHandle<io::Result<()>> {
  let state = shared.state.clone();
  let graph = shared.graph.clone();
  let log_store = shared.log_store.clone();
  let stderr_done = shared.stderr_done.clone();
  let log_prefix_style = cfg.log_prefix_style;
  let silent = cfg.silent;

  thread::spawn(move || {
    use tracing::debug;

    struct MarkDone(Arc<AtomicBool>);

    impl Drop for MarkDone {
      fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
      }
    }

    let _mark_done = MarkDone(stderr_done);
    let reader = BufReader::new(stderr);
    let mut json_count = 0;
    let mut non_json_count = 0;
    let mut log_prefixes = HashMap::new();

    for line in reader.lines() {
      let line = line?;
      if let Some(json_line) = line.strip_prefix("@nix ") {
        json_count += 1;
        if let Ok(action) = serde_json::from_str::<cognos::Actions>(json_line) {
          debug!("Parsed JSON message #{}: {:?}", json_count, action);

          if let Some((id, line)) = build_log_line(&action) {
            let prefix = log_prefixes.get(&id).cloned().unwrap_or_default();
            push_log(&log_store, format!("{prefix}{line}"));
            continue;
          }
          if let Some(line) = post_build_log_line(&action) {
            push_log(&log_store, format!("[post-build] {line}"));
            continue;
          }

          if let Some((id, prefix)) =
            build_log_prefix(&action, log_prefix_style, !silent)
          {
            log_prefixes.insert(id, prefix);
          }

          let log_line = message_log_line(&action, preserve_log_ansi);
          if !rom_core::update::action_may_update_state(&action) {
            if let Some(line) = log_line {
              push_log(&log_store, line);
            }
            continue;
          }

          let (derivation_count_before, derivation_count_after) = {
            let mut state = state.lock().unwrap();
            let derivation_count_before = state.derivation_infos.len();
            rom_core::update::process_message(&mut state, action.clone());
            let mut graph = graph.lock().unwrap();
            graph.observe_action(&mut state, &action);
            if let cognos::Actions::Message { msg, raw_msg, .. } = &action {
              graph.observe_plan_line(
                &mut state,
                raw_msg.as_deref().unwrap_or(msg.as_str()),
              );
            }
            let derivation_count_after = state.derivation_infos.len();

            (derivation_count_before, derivation_count_after)
          };

          if let Some(line) = log_line {
            push_log(&log_store, line);
          }

          if let cognos::Actions::Stop { id } = action {
            log_prefixes.remove(&id);
          }

          if derivation_count_after != derivation_count_before {
            debug!(
              "Derivation count changed: {} -> {}",
              derivation_count_before, derivation_count_after
            );
          }
        } else {
          debug!("Failed to parse JSON: {}", json_line);
          push_log(&log_store, line);
        }
      } else {
        non_json_count += 1;
        push_log(&log_store, line);
      }
    }

    debug!(
      "Stderr thread finished: {} JSON messages, {} non-JSON lines",
      json_count, non_json_count
    );
    Ok(())
  })
}

fn spawn_queued_stdout_reader<R: Read + Send + 'static>(
  mut child_stdout: R,
  sender: SyncSender<Vec<u8>>,
  stdout_done: Arc<AtomicBool>,
) -> thread::JoinHandle<io::Result<()>> {
  thread::spawn(move || {
    struct MarkDone(Arc<AtomicBool>);
    impl Drop for MarkDone {
      fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
      }
    }
    let _mark_done = MarkDone(stdout_done);
    let mut buffer = [0_u8; 8192];
    loop {
      let read = child_stdout.read(&mut buffer)?;
      if read == 0 {
        break;
      }
      if sender.send(buffer[..read].to_vec()).is_err() {
        break;
      }
    }
    Ok(())
  })
}

fn spawn_stdout_reader<R: Read + Send + 'static>(
  mut child_stdout: R,
  screen_dirty: Arc<AtomicBool>,
  stdout_done: Arc<AtomicBool>,
) -> thread::JoinHandle<io::Result<()>> {
  thread::spawn(move || {
    struct MarkDone(Arc<AtomicBool>);
    impl Drop for MarkDone {
      fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
      }
    }
    let _mark_done = MarkDone(stdout_done);
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut buffer = [0_u8; 8192];
    loop {
      let read = child_stdout.read(&mut buffer)?;
      if read == 0 {
        break;
      }
      stdout.write_all(&buffer[..read])?;
      stdout.flush()?;
      screen_dirty.store(true, Ordering::Release);
    }
    Ok(())
  })
}

fn build_log_line(action: &cognos::Actions) -> Option<(cognos::Id, &str)> {
  let cognos::Actions::Result {
    fields,
    id,
    result_type,
  } = action
  else {
    return None;
  };
  if !matches!(result_type, cognos::ResultType::BuildLogLine) {
    return None;
  }

  fields
    .first()
    .and_then(|field| field.as_str())
    .map(|line| (*id, line))
}

fn post_build_log_line(action: &cognos::Actions) -> Option<&str> {
  let cognos::Actions::Result {
    fields,
    result_type,
    ..
  } = action
  else {
    return None;
  };
  if !matches!(result_type, cognos::ResultType::PostBuildLogLine) {
    return None;
  }

  fields.first().and_then(|field| field.as_str())
}

fn message_log_line(
  action: &cognos::Actions,
  preserve_log_ansi: bool,
) -> Option<String> {
  let cognos::Actions::Message { msg, raw_msg, .. } = action else {
    return None;
  };

  let display = if preserve_log_ansi {
    msg.as_str()
  } else {
    raw_msg.as_deref().unwrap_or(msg.as_str())
  };
  Some(display.to_string())
}

fn build_log_prefix(
  action: &cognos::Actions,
  style: rom_core::types::LogPrefixStyle,
  use_color: bool,
) -> Option<(cognos::Id, String)> {
  let cognos::Actions::Start {
    id,
    text,
    activity,
    fields,
    ..
  } = action
  else {
    return None;
  };
  if *activity != cognos::Activities::Build {
    return None;
  }

  let name = fields
    .first()
    .and_then(|value| value.as_str())
    .and_then(rom_core::state::Derivation::parse)
    .or_else(|| {
      text
        .split_whitespace()
        .map(|part| {
          part.trim_matches(|ch| ch == '\'' || ch == '"' || ch == ',')
        })
        .find_map(rom_core::state::Derivation::parse)
    })
    .map(|drv| drv.name)
    .unwrap_or_default();

  Some((*id, format_log_prefix(&name, style, use_color)))
}

fn format_log_prefix(
  name: &str,
  style: rom_core::types::LogPrefixStyle,
  use_color: bool,
) -> String {
  if matches!(style, rom_core::types::LogPrefixStyle::None) || name.is_empty() {
    return String::new();
  }

  let name = if use_color && std::io::stderr().is_terminal() {
    format!("\x1b[34m{name}\x1b[0m")
  } else {
    name.to_string()
  };
  format!("{name}> ")
}

fn push_log(log_store: &Arc<LogStore>, line: String) {
  log_store.push(line);
}

pub(super) fn snapshot_logs(
  shared: &MonitorShared,
  silent: bool,
) -> Vec<String> {
  if silent {
    Vec::new()
  } else {
    shared.log_store.snapshot()
  }
}

pub(super) fn console_config(
  use_color: bool,
) -> rom_core::console::ConsoleConfig {
  rom_core::console::ConsoleConfig {
    use_color,
    width: crossterm::terminal::size().map_or(100, |(width, _)| width),
    ..rom_core::console::ConsoleConfig::default()
  }
}

pub(super) fn run_streaming_render_loop(
  child: &mut Child,
  shared: &MonitorShared,
  cfg: &WrapperConfig,
) -> eyre::Result<MonitorOutcome> {
  let render_state = shared.state.clone();
  let render_graph = shared.graph.clone();
  let log_store = shared.log_store.clone();
  let stderr_done = shared.stderr_done.clone();
  let silent = cfg.silent;

  // Redirected stderr must be append-only: stream each log once and leave the
  // stable summary to `render_final_after_monitor`.
  let render_thread = thread::spawn(move || -> io::Result<()> {
    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    loop {
      let done = stderr_done.load(Ordering::Acquire);
      {
        let mut state = render_state.lock().unwrap();
        let mut graph = render_graph.lock().unwrap();
        graph
          .populate_pending(&mut state, DEPENDENCY_POPULATE_BUDGET_PER_FRAME);
      }

      let lines = log_store.drain_pending();
      if !silent && !lines.is_empty() {
        for line in lines {
          writeln!(stderr, "{line}")?;
        }
        stderr.flush()?;
      }

      if done {
        break;
      }
      thread::sleep(Duration::from_millis(25));
    }
    Ok(())
  });

  let status = child.wait().map_err(rom_core::error::RomError::Io)?;
  render_thread
    .join()
    .map_err(|_| eyre::eyre!("stderr render thread panicked"))?
    .map_err(rom_core::error::RomError::Io)?;
  Ok(MonitorOutcome::Completed(status.code().unwrap_or(1)))
}

fn finish_monitored_command(
  shared: &MonitorShared,
  cfg: &WrapperConfig,
  outcome: MonitorOutcome,
  show_failure_errors: bool,
) -> eyre::Result<()> {
  if outcome.is_cancelled() {
    let _ = writeln!(io::stderr(), "rom: build cancelled");
    return Ok(());
  }

  finish_monitor_state(shared);
  render_final_after_monitor(
    shared,
    cfg,
    outcome.exit_code(),
    show_failure_errors,
  )
}

fn finish_monitor_state(shared: &MonitorShared) {
  let mut state = shared.state.lock().unwrap();
  let mut graph = shared.graph.lock().unwrap();
  graph.drain_pending(&mut state, Duration::from_secs(2));
  rom_core::update::finish_state(&mut state);
}

fn render_final_after_monitor(
  shared: &MonitorShared,
  _cfg: &WrapperConfig,
  exit_code: i32,
  show_failure_errors: bool,
) -> eyre::Result<()> {
  let state = shared.state.lock().unwrap();
  rom_core::console::write_final_graph(
    io::stderr(),
    &state,
    console_config(io::stderr().is_terminal()),
  )
  .map_err(rom_core::error::RomError::Io)?;
  if show_failure_errors && exit_code != 0 {
    let logs = shared.log_store.snapshot();
    write_post_tui_failure_errors(io::stderr(), &state, &logs)
      .map_err(rom_core::error::RomError::Io)?;
  }
  Ok(())
}

fn write_post_tui_failure_errors<W: Write>(
  mut writer: W,
  state: &rom_core::state::State,
  logs: &[String],
) -> io::Result<()> {
  let lines = post_tui_failure_error_lines(state, logs);
  if lines.is_empty() {
    return Ok(());
  }

  writeln!(writer, "Build errors:")?;
  for line in lines {
    writeln!(writer, "{line}")?;
  }
  writeln!(writer)?;
  writer.flush()
}

/// Replace --command/-c arguments with "sh -c exit" for monitoring pass
pub fn replace_command_with_exit(args: &[String]) -> Vec<String> {
  let mut result = Vec::new();
  let mut skip_next = false;

  for arg in args {
    if skip_next {
      skip_next = false;
      continue;
    }

    if arg == "--command" || arg == "-c" {
      // Skip this and the next argument
      skip_next = true;
      continue;
    }

    result.push(arg.clone());
  }

  // Add our exit command
  result.push("--command".to_string());
  result.push("sh".to_string());
  result.push("-c".to_string());
  result.push("exit".to_string());

  result
}
