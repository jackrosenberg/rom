use std::collections::HashSet;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{
  CollapsedDependencies,
  TransferActivity,
  TransferKind,
  TransferLookup,
  active_activity_status,
  derivation_transfer_activity,
};
use crate::{
  console::format_duration,
  state::{BuildInfo, BuildStatus, DerivationId, RenderSnapshot},
  tui::{
    BUILT_GREEN,
    DOWNLOAD_BLUE,
    MOSS_GREEN,
    MUTED_RED,
    MUTED_YELLOW,
    UPLOAD_PURPLE,
    hierarchy_style,
    screen::{Line, Span, Style},
    secondary_style,
    spinner_frame,
  },
};

const MAX_ACTIVITY_NAME_CHARS: usize = 56;

#[derive(Clone)]
enum RowActivity {
  Build,
  Transfer(TransferActivity),
}

fn row_activity(
  transfer_lookup: &TransferLookup,
  drv_id: DerivationId,
  info: &crate::state::RenderDerivationInfo,
) -> RowActivity {
  match derivation_transfer_activity(transfer_lookup, drv_id) {
    Some(transfer @ TransferActivity::Running { .. }) => {
      RowActivity::Transfer(transfer)
    },
    _ if active_activity_status(&info.build_status) => RowActivity::Build,
    Some(transfer) => RowActivity::Transfer(transfer),
    None => RowActivity::Build,
  }
}

pub(super) struct ActivityLine<'a> {
  pub(super) state:           &'a RenderSnapshot,
  pub(super) transfer_lookup: &'a TransferLookup,
  pub(super) drv_id:          DerivationId,
  pub(super) info:            &'a crate::state::RenderDerivationInfo,
  pub(super) collapsed_deps:  CollapsedDependencies,
  pub(super) depth:           usize,
  pub(super) now:             f64,
  pub(super) width:           usize,
}

#[derive(Clone)]
pub(super) struct RenderedActivityLine {
  pub(super) line: Line,
}

impl RenderedActivityLine {
  pub(super) fn with_prefix(mut self, prefix: &str) -> Self {
    if !prefix.is_empty() {
      self
        .line
        .spans
        .insert(0, Span::styled(prefix, hierarchy_style()));
    }
    self
  }

  pub(super) fn to_line(&self) -> Line {
    self.line.clone()
  }
}

pub(super) fn activity_line(args: ActivityLine<'_>) -> RenderedActivityLine {
  let ActivityLine {
    state,
    transfer_lookup,
    drv_id,
    info,
    collapsed_deps,
    depth,
    now,
    width,
  } = args;
  let prefix_width = depth.saturating_mul(3);
  let row_activity = row_activity(transfer_lookup, drv_id, info);
  let (status, status_style) =
    status_indicator(&row_activity, &info.build_status, now);
  let status_prefix_width = if status.is_empty() {
    0
  } else {
    UnicodeWidthStr::width(status.as_str()) + 1
  };
  let suffix = activity_suffix(state, info, collapsed_deps, &row_activity);
  let elapsed = activity_elapsed(&row_activity, &info.build_status, now);
  let display_name = disambiguated_name(state, info);
  let body = activity_spans(
    status,
    status_style,
    &display_name,
    name_style(&row_activity, &info.build_status, depth),
    suffix.as_deref(),
    &elapsed,
    width,
    prefix_width,
    status_prefix_width,
  );

  RenderedActivityLine {
    line: Line::from(body),
  }
}

pub(super) fn transfer_activity_line(
  state: &RenderSnapshot,
  transfer: &TransferActivity,
  now: f64,
  width: usize,
) -> Option<Line> {
  let name = state
    .get_store_path_info(transfer.path_id())?
    .name
    .name
    .clone();
  let row_activity = RowActivity::Transfer(transfer.clone());
  let (status, status_style) =
    status_indicator(&row_activity, &BuildStatus::Unknown, now);
  let status_prefix_width = if status.is_empty() {
    0
  } else {
    UnicodeWidthStr::width(status.as_str()) + 1
  };
  let suffix = transfer_suffix(state, transfer);
  let elapsed = activity_elapsed(&row_activity, &BuildStatus::Unknown, now);
  Some(Line::from(activity_spans(
    status,
    status_style,
    &name,
    name_style(&row_activity, &BuildStatus::Unknown, 0),
    suffix.as_deref(),
    &elapsed,
    width,
    0,
    status_prefix_width,
  )))
}

fn status_indicator(
  row_activity: &RowActivity,
  status: &BuildStatus,
  now: f64,
) -> (String, Style) {
  match row_activity {
    RowActivity::Transfer(TransferActivity::Running { kind, .. }) => {
      let (direction, color) = match kind {
        TransferKind::Download => ("↓", DOWNLOAD_BLUE),
        TransferKind::Upload => ("↑", UPLOAD_PURPLE),
      };
      (
        format!("{direction} {}", spinner_frame(now)),
        Style::default().fg(color),
      )
    },
    RowActivity::Transfer(TransferActivity::PlannedDownload { .. }) => {
      ("↓".to_string(), Style::default().fg(DOWNLOAD_BLUE))
    },
    RowActivity::Build => {
      match status {
        BuildStatus::Building(_) => {
          (
            spinner_frame(now).to_string(),
            Style::default().fg(MOSS_GREEN),
          )
        },
        BuildStatus::Built { .. } => {
          ("✓".to_string(), Style::default().fg(BUILT_GREEN))
        },
        BuildStatus::Failed { .. } => {
          ("✗".to_string(), Style::default().fg(MUTED_RED))
        },
        BuildStatus::Planned => {
          ("".to_string(), Style::default().fg(MUTED_YELLOW))
        },
        BuildStatus::Unknown => (" ".to_string(), Style::default()),
      }
    },
  }
}

fn activity_suffix(
  state: &RenderSnapshot,
  info: &crate::state::RenderDerivationInfo,
  collapsed_deps: CollapsedDependencies,
  row_activity: &RowActivity,
) -> Option<String> {
  let status_suffix = match row_activity {
    RowActivity::Transfer(transfer) => transfer_suffix(state, transfer),
    RowActivity::Build => {
      match &info.build_status {
        BuildStatus::Building(build) => running_suffix(state, build),
        BuildStatus::Failed { info: build, fail } => {
          Some(failed_suffix(state, build, fail))
        },
        BuildStatus::Built { .. } => None,
        BuildStatus::Planned => None,
        BuildStatus::Unknown => None,
      }
    },
  };
  combine_suffixes(status_suffix, collapsed_deps_suffix(collapsed_deps))
}

fn transfer_suffix(
  state: &RenderSnapshot,
  activity: &TransferActivity,
) -> Option<String> {
  match activity {
    TransferActivity::Running { kind, transfer, .. } => {
      let mut parts = Vec::new();
      if let Some(total) = transfer.total_bytes {
        parts.push(format!(
          "{} / {}",
          format_bytes(transfer.bytes_transferred),
          format_bytes(total)
        ));
      }
      if matches!(kind, TransferKind::Upload)
        && let Some(host) = remote_host_label(state, &transfer.host)
      {
        parts.push(format!("to {host}"));
      }
      (!parts.is_empty()).then(|| parts.join(" "))
    },
    TransferActivity::PlannedDownload { .. } => None,
  }
}

fn format_bytes(bytes: u64) -> String {
  const KIB: f64 = 1024.0;
  const MIB: f64 = KIB * 1024.0;
  const GIB: f64 = MIB * 1024.0;

  let bytes = bytes as f64;
  if bytes >= GIB {
    format!("{:.1} GiB", bytes / GIB)
  } else if bytes >= MIB {
    format!("{:.1} MiB", bytes / MIB)
  } else if bytes >= KIB {
    format!("{:.1} KiB", bytes / KIB)
  } else {
    format!("{bytes:.0} B")
  }
}

fn combine_suffixes(
  first: Option<String>,
  second: Option<String>,
) -> Option<String> {
  match (first, second) {
    (Some(first), Some(second)) => Some(format!("{first} · {second}")),
    (Some(first), None) => Some(first),
    (None, Some(second)) => Some(second),
    (None, None) => None,
  }
}

fn collapsed_deps_suffix(deps: CollapsedDependencies) -> Option<String> {
  let mut parts = Vec::new();
  if deps.built > 0 {
    parts.push(format!("built {}", deps.built));
  }
  if deps.waiting > 0 {
    parts.push(format!("waiting {}", deps.waiting));
  }
  if deps.shared > 0 {
    parts.push(format!("shared {}", deps.shared));
  }

  if parts.is_empty() {
    None
  } else {
    Some(parts.join(" · "))
  }
}

fn running_suffix(state: &RenderSnapshot, build: &BuildInfo) -> Option<String> {
  let phase = build
    .activity_id
    .and_then(|id| state.activities.get(&id))
    .and_then(|activity| activity.phase.as_deref());
  let host = remote_host_label(state, &build.host);
  let mut parts = Vec::new();
  if let Some(phase) = phase {
    parts.push(phase.to_string());
  }
  if let Some(host) = host {
    parts.push(format!("on {host}"));
  }
  (!parts.is_empty()).then(|| parts.join(" "))
}

fn failed_suffix(
  state: &RenderSnapshot,
  build: &BuildInfo,
  fail: &crate::state::BuildFail,
) -> String {
  let mut suffix = match &fail.fail_type {
    crate::state::FailType::BuildFailed(code) => {
      format!("failed with exit code {code}")
    },
    crate::state::FailType::Timeout => "timed out".to_string(),
    crate::state::FailType::HashMismatch => "hash mismatch".to_string(),
    crate::state::FailType::DependencyFailed => "dependency failed".to_string(),
    crate::state::FailType::Unknown => "failed".to_string(),
  };

  if let Some(phase) = build
    .activity_id
    .and_then(|id| state.activities.get(&id))
    .and_then(|activity| activity.phase.as_deref())
  {
    suffix.push_str(" in ");
    suffix.push_str(phase);
  }

  if let Some(host) = remote_host_label(state, &build.host) {
    suffix.push_str(" on ");
    suffix.push_str(&host);
  }

  suffix
}

fn remote_host_label(
  state: &RenderSnapshot,
  host: &cognos::Host,
) -> Option<String> {
  let cognos::Host::Remote(raw) = host else {
    return None;
  };
  let short = short_host(raw);
  let collides = remote_hosts(state)
    .into_iter()
    .any(|other| other != *raw && short_host(&other) == short);
  Some(if collides {
    raw.clone()
  } else {
    short.to_string()
  })
}

fn short_host(host: &str) -> &str {
  let without_scheme = host.split_once("://").map_or(host, |(_, rest)| rest);
  let without_user = without_scheme
    .rsplit_once('@')
    .map_or(without_scheme, |(_, rest)| rest);
  let without_port = without_user.split(':').next().unwrap_or(without_user);
  without_port.split('.').next().unwrap_or(without_port)
}

fn remote_hosts(state: &RenderSnapshot) -> HashSet<String> {
  let mut hosts = HashSet::new();
  let mut insert = |host: &cognos::Host| {
    if let cognos::Host::Remote(host) = host {
      hosts.insert(host.clone());
    }
  };
  for build in state.full_summary.running_builds.values() {
    insert(&build.host);
  }
  for build in state.full_summary.completed_builds.values() {
    insert(&build.host);
  }
  for build in state.full_summary.failed_builds.values() {
    insert(&build.host);
  }
  for transfer in state
    .full_summary
    .running_downloads
    .values()
    .chain(state.full_summary.running_uploads.values())
  {
    insert(&transfer.host);
  }
  hosts
}

fn disambiguated_name(
  state: &RenderSnapshot,
  info: &crate::state::RenderDerivationInfo,
) -> String {
  let Some(platform) = info.platform.as_deref() else {
    return info.name.name.clone();
  };
  let differs = state
    .derivation_ids_with_name(&info.name.name)
    .into_iter()
    .filter_map(|id| state.get_derivation_info(id))
    .any(|other| {
      other
        .platform
        .as_deref()
        .is_some_and(|other_platform| other_platform != platform)
    });
  if differs {
    format!("{} [{platform}]", info.name.name)
  } else {
    info.name.name.clone()
  }
}

fn activity_elapsed(
  row_activity: &RowActivity,
  status: &BuildStatus,
  now: f64,
) -> String {
  match row_activity {
    RowActivity::Transfer(TransferActivity::Running { transfer, .. }) => {
      elapsed_since(transfer.start, now)
    },
    RowActivity::Transfer(TransferActivity::PlannedDownload { .. }) => {
      String::new()
    },
    RowActivity::Build => {
      match status {
        BuildStatus::Building(build) => elapsed_since(build.start, now),
        BuildStatus::Built { info, end } => elapsed_since(info.start, *end),
        BuildStatus::Failed { info, fail } => {
          elapsed_since(info.start, fail.at)
        },
        BuildStatus::Planned | BuildStatus::Unknown => String::new(),
      }
    },
  }
}

fn elapsed_since(start: f64, end: f64) -> String {
  let elapsed = (end - start).max(0.0);
  if elapsed < 0.3 {
    String::new()
  } else {
    format_duration(elapsed)
  }
}

#[allow(clippy::too_many_arguments)]
fn activity_spans(
  status: String,
  status_style: Style,
  name: &str,
  name_style: Style,
  suffix: Option<&str>,
  elapsed: &str,
  width: usize,
  prefix_width: usize,
  status_prefix_width: usize,
) -> Vec<Span> {
  let name = fit_activity_name(
    name,
    width,
    prefix_width,
    status_prefix_width,
    suffix,
    elapsed,
  );
  let mut spans = Vec::new();
  if !status.is_empty() {
    spans.push(Span::styled(status, status_style));
    spans.push(Span::raw(" "));
  }
  spans.push(Span::styled(name, name_style));
  if let Some(suffix) = suffix {
    spans.push(Span::styled(" · ", secondary_style()));
    spans.push(Span::styled(suffix, secondary_style()));
  }
  if !elapsed.is_empty() {
    spans.push(Span::styled(" · ", secondary_style()));
    spans.push(Span::styled(elapsed, secondary_style()));
  }
  spans
}

fn fit_activity_name(
  name: &str,
  width: usize,
  prefix_width: usize,
  status_prefix_width: usize,
  suffix: Option<&str>,
  elapsed: &str,
) -> String {
  let suffix_width = suffix
    .map(|suffix| UnicodeWidthStr::width(suffix) + 3)
    .unwrap_or(0);
  let elapsed_width = if elapsed.is_empty() {
    0
  } else {
    UnicodeWidthStr::width(elapsed) + 3
  };
  let fixed_width =
    prefix_width + status_prefix_width + suffix_width + elapsed_width;
  let available = width
    .saturating_sub(fixed_width)
    .clamp(1, MAX_ACTIVITY_NAME_CHARS);

  truncate_start_width(name, available)
}

fn truncate_start_width(text: &str, max_width: usize) -> String {
  if UnicodeWidthStr::width(text) <= max_width {
    return text.to_string();
  }
  if max_width <= 1 {
    return "…".to_string();
  }

  let mut tail = Vec::new();
  let mut width = 1;
  for grapheme in text.graphemes(true).rev() {
    let grapheme_width = UnicodeWidthStr::width(grapheme);
    if width + grapheme_width > max_width {
      break;
    }
    width += grapheme_width;
    tail.push(grapheme);
  }
  tail.reverse();
  format!("…{}", tail.concat())
}

fn name_style(
  row_activity: &RowActivity,
  status: &BuildStatus,
  depth: usize,
) -> Style {
  match row_activity {
    RowActivity::Transfer(TransferActivity::Running { kind, .. }) => {
      Style::default().fg(match kind {
        TransferKind::Download => DOWNLOAD_BLUE,
        TransferKind::Upload => UPLOAD_PURPLE,
      })
    },
    RowActivity::Transfer(TransferActivity::PlannedDownload { .. }) => {
      Style::default().fg(MUTED_YELLOW)
    },
    RowActivity::Build => {
      match status {
        BuildStatus::Failed { .. } => Style::default().fg(MUTED_RED),
        BuildStatus::Planned | BuildStatus::Unknown => {
          Style::default().fg(MUTED_YELLOW)
        },
        BuildStatus::Built { .. } => Style::default().fg(BUILT_GREEN),
        BuildStatus::Building(_) if depth == 0 => {
          Style::default().fg(MOSS_GREEN)
        },
        BuildStatus::Building(_) => Style::default().fg(MOSS_GREEN),
      }
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn truncation_uses_display_width_and_preserves_graphemes() {
    let wide = truncate_start_width("prefix-日本語", 7);
    assert!(UnicodeWidthStr::width(wide.as_str()) <= 7);
    assert!(wide.ends_with("本語"));

    let combining = truncate_start_width("prefix-e\u{301}nd", 5);
    assert!(UnicodeWidthStr::width(combining.as_str()) <= 5);
    assert!(combining.contains("e\u{301}"));
  }
}
