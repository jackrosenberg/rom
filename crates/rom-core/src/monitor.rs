//! Append-only monitoring for Nix output streams.
use std::{
  io::{BufRead, Write},
  time::Duration,
};

use cognos::{Actions, Host, ResultType};

use crate::{
  console::{ConsoleConfig, write_final_graph},
  error::{Result, RomError},
  graph::GraphIndexer,
  state::{
    BuildFail,
    BuildInfo,
    BuildStatus,
    Derivation,
    FailType,
    State,
    StorePath,
    TransferInfo,
    current_time,
  },
  types::{Config, InputMode},
  update,
};

const DEPENDENCY_POPULATE_BUDGET_PER_LINE: usize = 1;

#[derive(Clone, Copy, Default)]
enum HumanParserState {
  #[default]
  Idle,
  PlanBuilds,
  PlanDownloads,
}

/// Incremental, append-only Nix stream monitor.
///
/// `Monitor` never takes ownership of a terminal or rewrites prior output. Log
/// records are passed to the supplied writer once, and [`Self::finish`] appends
/// the final graph rendered by the operations-console seam.
pub struct Monitor<W: Write> {
  state:       State,
  graph:       GraphIndexer,
  writer:      W,
  config:      Config,
  human_state: HumanParserState,
  finished:    bool,
}

impl<W: Write> Monitor<W> {
  /// Create a monitor writing to `writer`.
  pub fn new(config: Config, writer: W) -> Result<Self> {
    if config.width == 0 {
      return Err(RomError::config("output width must be greater than zero"));
    }
    Ok(Self {
      state: State::new(),
      graph: GraphIndexer::new(),
      writer,
      config,
      human_state: HumanParserState::Idle,
      finished: false,
    })
  }

  /// Process all lines and append the final graph.
  pub fn process_stream<R: BufRead>(&mut self, reader: R) -> Result<()> {
    for line in reader.lines() {
      self.process_line(&line.map_err(RomError::Io)?)?;
    }
    self.finish()
  }

  /// Process one line, returning whether it changed monitored state.
  pub fn process_line(&mut self, line: &str) -> Result<bool> {
    if self.finished {
      return Err(RomError::other("monitor is already finished"));
    }

    let changed = if let Some(json) = line.strip_prefix("@nix ") {
      self.process_json(json, line)?
    } else if self.config.input_mode == InputMode::Json {
      self.process_json(line, line)?
    } else {
      self.process_human(line)?
    };

    let populated = self
      .graph
      .populate_pending(&mut self.state, DEPENDENCY_POPULATE_BUDGET_PER_LINE);
    Ok(changed || populated)
  }

  /// Apply a decoded internal-json action.
  pub fn process_action(&mut self, action: Actions) -> Result<bool> {
    if self.finished {
      return Err(RomError::other("monitor is already finished"));
    }
    let changed = self.apply_action(action);
    let populated = self
      .graph
      .populate_pending(&mut self.state, DEPENDENCY_POPULATE_BUDGET_PER_LINE);
    Ok(changed || populated)
  }

  /// Drain graph indexing, append the final graph, and report build failures.
  pub fn finish(&mut self) -> Result<()> {
    if !self.finished {
      self.finished = true;
      self
        .graph
        .drain_pending(&mut self.state, Duration::from_secs(2));
      update::finish_state(&mut self.state);
      write_final_graph(&mut self.writer, &self.state, ConsoleConfig {
        use_color: self.config.use_color,
        width: self.config.width,
        ..ConsoleConfig::default()
      })
      .map_err(RomError::Io)?;
    }

    if self.state.has_errors() {
      Err(RomError::BuildFailed)
    } else {
      Ok(())
    }
  }

  /// Current state accumulated from input processed so far.
  #[must_use]
  pub const fn state(&self) -> &State {
    &self.state
  }

  /// Mutable access to current state for embedding applications.
  pub const fn state_mut(&mut self) -> &mut State {
    &mut self.state
  }

  /// Current dependency indexer.
  #[must_use]
  pub const fn graph(&self) -> &GraphIndexer {
    &self.graph
  }

  /// Access the underlying output writer.
  pub const fn writer_mut(&mut self) -> &mut W {
    &mut self.writer
  }

  /// Consume the monitor and return its writer without implicitly finishing.
  pub fn into_writer(self) -> W {
    self.writer
  }

  fn process_json(&mut self, json: &str, original: &str) -> Result<bool> {
    let action = match serde_json::from_str::<Actions>(json) {
      Ok(action) => action,
      Err(error) => {
        tracing::debug!(%error, "failed to parse internal-json record");
        self.write_log(original)?;
        return Ok(false);
      },
    };

    match &action {
      Actions::Message { msg, raw_msg, .. } => {
        let line = if self.config.use_color {
          msg.as_str()
        } else {
          raw_msg.as_deref().unwrap_or(msg.as_str())
        };
        self.write_log(line)?;
      },
      Actions::Result {
        fields,
        result_type: ResultType::BuildLogLine | ResultType::PostBuildLogLine,
        ..
      } => {
        if let Some(line) = fields.first().and_then(serde_json::Value::as_str) {
          self.write_log(line)?;
        }
      },
      Actions::Start { .. } | Actions::Stop { .. } | Actions::Result { .. } => {
      },
    }

    Ok(self.apply_action(action))
  }

  fn apply_action(&mut self, action: Actions) -> bool {
    let changed = update::process_message(&mut self.state, action.clone());
    let observed = self.graph.observe_action(&mut self.state, &action);
    let planned = if let Actions::Message { msg, raw_msg, .. } = &action {
      self.graph.observe_plan_line(
        &mut self.state,
        raw_msg.as_deref().unwrap_or(msg.as_str()),
      )
    } else {
      false
    };
    changed || observed || planned
  }

  fn process_human(&mut self, line: &str) -> Result<bool> {
    self.write_log(line)?;
    let trimmed = line.trim();

    if matches!(
      self.human_state,
      HumanParserState::PlanBuilds | HumanParserState::PlanDownloads
    ) {
      if line.starts_with("  /nix/store/") || line.starts_with("\t/nix/store/")
      {
        let changed = match self.human_state {
          HumanParserState::PlanBuilds => {
            Derivation::parse(trimmed)
              .map(|drv| {
                self.graph.plan_derivation(&mut self.state, drv);
                true
              })
              .unwrap_or(false)
          },
          HumanParserState::PlanDownloads => {
            StorePath::parse(trimmed)
              .map(|path| {
                let id = self.state.get_or_create_store_path_id(path);
                self.state.full_summary.planned_downloads.insert(id)
              })
              .unwrap_or(false)
          },
          HumanParserState::Idle => false,
        };
        return Ok(changed);
      }
      self.human_state = HumanParserState::Idle;
    }

    if trimmed.ends_with("derivations will be built:")
      || trimmed.ends_with("derivation will be built:")
    {
      self.human_state = HumanParserState::PlanBuilds;
      return Ok(true);
    }
    if trimmed.contains("paths will be fetched")
      || trimmed.contains("path will be fetched")
    {
      self.human_state = HumanParserState::PlanDownloads;
      return Ok(true);
    }

    if trimmed.starts_with("building '") && trimmed.contains(".drv'") {
      return Ok(self.start_human_build(trimmed));
    }

    let reports_build_failure = (trimmed.contains("builder for '")
      && trimmed.contains("failed"))
      || trimmed.contains("Cannot build '");
    if reports_build_failure && let Some(drv) = extract_derivation(trimmed) {
      let id = self.state.get_or_create_derivation_id(drv);
      let now = current_time();
      let info = current_build_info(&self.state, id).unwrap_or(BuildInfo {
        start:       now,
        host:        Host::Localhost,
        activity_id: None,
      });
      let code = trimmed
        .split_once("exit code")
        .and_then(|(_, tail)| {
          tail.trim().split(|c: char| !c.is_ascii_digit()).next()
        })
        .and_then(|code| code.parse().ok());
      self.state.update_build_status(id, BuildStatus::Failed {
        info,
        fail: BuildFail {
          at:        now,
          fail_type: code.map_or(FailType::Unknown, FailType::BuildFailed),
        },
      });
      return Ok(true);
    }

    if trimmed.starts_with("copying path '")
      && let Some(path) = extract_store_path(trimmed)
    {
      let id = self.state.get_or_create_store_path_id(path);
      let host = if let Some((_, destination)) = trimmed
        .split_once(" from '")
        .or_else(|| trimmed.split_once(" to '"))
      {
        parse_host(destination.split('\'').next().unwrap_or_default())
      } else {
        Host::Localhost
      };
      let transfer = TransferInfo {
        start: current_time(),
        host,
        activity_id: 0,
        bytes_transferred: 0,
        total_bytes: None,
      };
      if trimmed.contains(" to '") {
        self.state.full_summary.running_uploads.insert(id, transfer);
      } else {
        self
          .state
          .full_summary
          .running_downloads
          .insert(id, transfer);
      }
      return Ok(true);
    }

    if trimmed.starts_with("error:") || trimmed.contains(" error:") {
      self.state.nix_errors.push(trimmed.to_string());
      return Ok(true);
    }

    Ok(false)
  }

  fn start_human_build(&mut self, line: &str) -> bool {
    let Some(drv) = extract_derivation(line) else {
      return false;
    };
    let id = self.graph.plan_derivation(&mut self.state, drv);
    let host = line
      .split_once(" on '")
      .map_or(Host::Localhost, |(_, tail)| {
        parse_host(tail.split('\'').next().unwrap_or_default())
      });
    self.state.update_build_status(
      id,
      BuildStatus::Building(BuildInfo {
        start: current_time(),
        host,
        activity_id: None,
      }),
    );
    true
  }

  fn write_log(&mut self, line: &str) -> Result<()> {
    if !self.config.silent {
      writeln!(self.writer, "{line}").map_err(RomError::Io)?;
    }
    Ok(())
  }
}

fn current_build_info(
  state: &State,
  id: crate::state::DerivationId,
) -> Option<BuildInfo> {
  state.get_derivation_info(id).and_then(|info| {
    if let BuildStatus::Building(build) = &info.build_status {
      Some(build.clone())
    } else {
      None
    }
  })
}

fn extract_quoted_path(line: &str) -> Option<&str> {
  let (_, rest) = line.split_once('\'')?;
  rest.split_once('\'').map(|(path, _)| path)
}

fn extract_derivation(line: &str) -> Option<Derivation> {
  extract_quoted_path(line).and_then(Derivation::parse)
}

fn extract_store_path(line: &str) -> Option<StorePath> {
  extract_quoted_path(line).and_then(StorePath::parse)
}

fn parse_host(value: &str) -> Host {
  let name = value
    .strip_prefix("ssh://")
    .or_else(|| value.strip_prefix("https://"))
    .or_else(|| value.strip_prefix("http://"))
    .unwrap_or(value)
    .trim_end_matches('/');
  if name.is_empty() || name == "localhost" {
    Host::Localhost
  } else {
    Host::Remote(name.to_string())
  }
}
