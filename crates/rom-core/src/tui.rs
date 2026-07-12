//! Full-screen styled framebuffer renderer for ROM.
use std::collections::BTreeMap;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod activity;
mod logs;
mod screen;

pub use self::screen::{
  Attribute,
  Attributes,
  Color,
  ContentStyle,
  Screen,
  ScreenCell,
  Style,
};
use self::{
  activity::{minimum_required_activity_rows, render_activity_graph_lines},
  screen::{Line, Span},
};
use crate::{
  console::{ConsoleConfig, format_duration},
  state::{RenderSnapshot, State, current_time},
};

const TEXT_MUTED: Color = Color::Rgb {
  r: 122,
  g: 164,
  b: 179,
};
const GRAPH_LINE_COLOR: Color = Color::Rgb {
  r: 47,
  g: 104,
  b: 126,
};
const MOSS_GREEN: Color = Color::Rgb {
  r: 63,
  g: 236,
  b: 208,
};
const BUILT_GREEN: Color = Color::Rgb {
  r: 74,
  g: 158,
  b: 139,
};
const DOWNLOAD_BLUE: Color = Color::Rgb {
  r: 73,
  g: 147,
  b: 255,
};
const UPLOAD_PURPLE: Color = Color::Rgb {
  r: 193,
  g: 96,
  b: 232,
};
const MUTED_RED: Color = Color::Rgb {
  r: 234,
  g: 65,
  b: 83,
};
const MUTED_YELLOW: Color = Color::Rgb {
  r: 255,
  g: 179,
  b: 76,
};
const SPINNER_FRAMES: &[&str] = &["⢄", "⢂", "⢁", "⡁", "⡈", "⡐", "⡠"];
#[derive(Clone, Copy, Default)]
pub struct TuiConfig {
  pub console: ConsoleConfig,
}

/// Minimum live-region height needed for mandatory activity rows and footer.
#[must_use]
pub fn minimum_required_graph_rows_at_width(
  width: u16,
  state: &RenderSnapshot,
) -> usize {
  minimum_required_activity_rows(state)
    .saturating_add(footer_lines(state, usize::from(width)).len())
}

/// Render the compact status and activity graph within a soft height budget.
#[must_use]
pub fn render_graph_screen(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
  config: &TuiConfig,
) -> Screen {
  render_graph_screen_inner(width, soft_height, state, config, true)
}

fn render_graph_screen_inner(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
  config: &TuiConfig,
  show_wait_timer: bool,
) -> Screen {
  if soft_height == 0 {
    return Screen::new(width, 0);
  }
  let footer_height = console_footer_height(width, soft_height, state);
  let graph_budget = soft_height.saturating_sub(footer_height);
  let lines = render_activity_graph_lines(
    state,
    config.console,
    usize::from(graph_budget),
    usize::from(width),
  )
  .lines;
  if lines.is_empty() {
    if !show_wait_timer {
      return Screen::new(width, 0);
    }
    let mut screen = Screen::new(width, 1);
    screen.draw_text(
      0,
      0,
      width,
      1,
      &[Line::from(Span::styled(
        format_duration(current_time() - state.start_time),
        secondary_style(),
      ))],
      false,
    );
    return screen;
  }
  let graph_height =
    graph_budget.min(u16::try_from(lines.len()).unwrap_or(u16::MAX));
  let height = graph_height.saturating_add(footer_height);
  let mut screen = Screen::new(width, height);
  screen.draw_text(0, 0, width, graph_height, &lines, true);
  draw_console_footer(&mut screen, graph_height, footer_height, state);
  screen
}

/// Render the stable post-build graph with the live renderer.
#[must_use]
pub fn render_final_graph_screen(
  width: u16,
  state: &State,
  config: &TuiConfig,
) -> Screen {
  let snapshot = state.render_snapshot();
  let optional_rows = snapshot
    .derivation_infos
    .len()
    .saturating_add(snapshot.full_summary.running_downloads.len())
    .saturating_add(snapshot.full_summary.running_uploads.len())
    .saturating_add(2)
    .min(config.console.max_visible_lines.saturating_add(2));
  let soft_height = u16::try_from(
    optional_rows.max(minimum_required_graph_rows_at_width(width, &snapshot)),
  )
  .unwrap_or(u16::MAX);
  render_graph_screen_inner(width, soft_height, &snapshot, config, false)
}

/// Parse one stored log record into safe, styled terminal rows.
#[must_use]
pub fn render_streamed_log(width: u16, line: &str) -> Screen {
  logs::render_streamed_log(width, line)
}

/// Repaint the retained log tail after a resize without duplicating records.
#[must_use]
pub fn render_retained_log_tail(
  width: u16,
  height: u16,
  logs: &[String],
  excluded_tail: usize,
) -> Screen {
  logs::render_retained_log_tail(width, height, logs, excluded_tail)
}

pub(super) fn spinner_frame(now: f64) -> &'static str {
  let frame = ((now * 1000.0) as usize / 80) % SPINNER_FRAMES.len();
  SPINNER_FRAMES[frame]
}

fn secondary_style() -> Style {
  Style::default().fg(TEXT_MUTED)
}

pub(super) fn hierarchy_style() -> Style {
  Style::default().fg(GRAPH_LINE_COLOR)
}

#[derive(Clone, Debug, Default)]
struct CacheActivity {
  active:           usize,
  completed:        usize,
  bytes_done:       u64,
  bytes_total:      u64,
  has_unknown_size: bool,
}

fn console_footer_height(
  width: u16,
  height: u16,
  state: &RenderSnapshot,
) -> u16 {
  if height == 0 {
    return 0;
  }
  let desired = footer_lines(state, usize::from(width)).len();
  let mandatory = minimum_required_activity_rows(state);
  if usize::from(height) >= mandatory.saturating_add(desired) {
    u16::try_from(desired).unwrap_or(u16::MAX)
  } else {
    1
  }
}

fn draw_console_footer(
  screen: &mut Screen,
  y: u16,
  height: u16,
  state: &RenderSnapshot,
) {
  if height == 0 || y >= screen.height() {
    return;
  }
  let lines = if height == 1 {
    vec![compact_footer_line(state, usize::from(screen.width()))]
  } else {
    footer_lines(state, usize::from(screen.width()))
  };
  screen.draw_text(0, y, screen.width(), height, &lines, false);
}

const CONNECTED_TABLE_MAX_WIDTH: usize = 80;

fn footer_lines(state: &RenderSnapshot, width: usize) -> Vec<Line> {
  let width = width.clamp(1, CONNECTED_TABLE_MAX_WIDTH);
  let columns = FooterColumns::for_width(width);
  let (pulls, pushes) = cache_activity(state);
  let mut hosts = pulls
    .keys()
    .chain(pushes.keys())
    .cloned()
    .collect::<Vec<_>>();
  hosts.sort();
  hosts.dedup();

  let mut lines = vec![table_header(
    "├",
    &[columns.host_title, columns.pull_title, columns.push_title],
    &columns.host_widths,
    "┐",
  )];
  if hosts.is_empty() {
    lines.push(table_row(
      &["—".to_string(), "—".to_string(), "—".to_string()],
      &columns.host_widths,
    ));
  } else {
    for host in hosts {
      let pull = pulls
        .get(&host)
        .map(|activity| transfer_cell(activity, columns.detail))
        .unwrap_or_else(|| "—".to_string());
      let push = pushes
        .get(&host)
        .map(|activity| transfer_cell(activity, columns.detail))
        .unwrap_or_else(|| "—".to_string());
      lines.push(table_row(&[host, pull, push], &columns.host_widths));
    }
  }

  lines.push(table_header(
    "├",
    &columns.build_titles,
    &columns.build_widths,
    "┤",
  ));
  let summary = &state.full_summary;
  let total = summary
    .planned_builds
    .len()
    .saturating_add(summary.running_builds.len())
    .saturating_add(summary.completed_builds.len())
    .saturating_add(summary.failed_builds.len());
  lines.push(table_row(
    &[
      format!("{total} builds"),
      summary.running_builds.len().to_string(),
      summary.planned_builds.len().to_string(),
      summary.completed_builds.len().to_string(),
      summary.failed_builds.len().to_string(),
    ],
    &columns.build_widths,
  ));
  lines.push(table_bottom(
    width,
    &format_duration(current_time() - state.start_time),
    &columns.build_widths,
  ));
  lines
}

#[derive(Clone, Debug)]
struct FooterColumns {
  host_title:   &'static str,
  pull_title:   &'static str,
  push_title:   &'static str,
  build_titles: [&'static str; 5],
  host_widths:  Vec<usize>,
  build_widths: Vec<usize>,
  detail:       bool,
}

impl FooterColumns {
  fn for_width(width: usize) -> Self {
    let (host_title, pull_title, push_title, build_titles, detail) =
      if width >= 72 {
        (
          "HOSTS",
          "PULL",
          "PUSH",
          ["BUILDS", "RUNNING", "WAITING", "DONE", "FAILED"],
          true,
        )
      } else if width >= 44 {
        (
          "HOST",
          "PULL",
          "PUSH",
          ["BUILDS", "RUN", "WAIT", "DONE", "FAIL"],
          false,
        )
      } else {
        (
          "H",
          "PULL",
          "PUSH",
          ["B", "RUN", "WAIT", "DONE", "FAIL"],
          false,
        )
      };
    Self {
      host_title,
      pull_title,
      push_title,
      build_titles,
      host_widths: distribute_columns(width.saturating_sub(7), 3, &[2, 1, 1]),
      build_widths: distribute_columns(width.saturating_sub(11), 5, &[
        2, 1, 1, 1, 1,
      ]),
      detail,
    }
  }
}

fn distribute_columns(
  available: usize,
  count: usize,
  weights: &[usize],
) -> Vec<usize> {
  let minimum = usize::from(available >= count);
  let mut widths = vec![minimum; count];
  let remaining = available.saturating_sub(minimum.saturating_mul(count));
  let weight_total = weights.iter().sum::<usize>().max(1);
  for index in 0..remaining {
    let point = index % weight_total;
    let mut accumulated = 0;
    let column = weights
      .iter()
      .position(|weight| {
        accumulated += *weight;
        point < accumulated
      })
      .unwrap_or(count - 1);
    widths[column] += 1;
  }
  widths
}

fn compact_footer_line(state: &RenderSnapshot, width: usize) -> Line {
  let width = width.min(CONNECTED_TABLE_MAX_WIDTH);
  let summary = &state.full_summary;
  let total = summary
    .planned_builds
    .len()
    .saturating_add(summary.running_builds.len())
    .saturating_add(summary.completed_builds.len())
    .saturating_add(summary.failed_builds.len());
  let elapsed = format_duration(current_time() - state.start_time);
  let status = format!(
    "{total} builds · {}/{}/{}/{}",
    summary.running_builds.len(),
    summary.planned_builds.len(),
    summary.completed_builds.len(),
    summary.failed_builds.len(),
  );
  let notch = format!("┤ {elapsed} ┘");
  let content_width = width.saturating_sub(2 + display_width(&notch));
  let content = fit_text(&status, content_width);
  let rule = "─".repeat(content_width.saturating_sub(display_width(&content)));
  Line::from(format!("└─{content}{rule}{notch}"))
}

fn table_header(
  left: &str,
  titles: &[&str],
  widths: &[usize],
  right: &str,
) -> Line {
  let mut spans = vec![Span::raw(left)];
  for (index, (title, width)) in titles.iter().zip(widths).enumerate() {
    let label = fit_text(&format!(" {title} "), *width);
    spans.push(Span::raw("─"));
    spans.push(Span::raw(label.clone()));
    spans.push(Span::raw(
      "─".repeat(width.saturating_sub(display_width(&label))),
    ));
    spans.push(Span::raw(if index + 1 == titles.len() {
      right
    } else {
      "┬"
    }));
  }
  Line::from(spans)
}

fn table_row(values: &[String], widths: &[usize]) -> Line {
  let mut spans = vec![Span::raw("│")];
  for (value, width) in values.iter().zip(widths) {
    let value = fit_text(value, *width);
    let padding = width.saturating_sub(display_width(&value));
    spans.push(Span::raw(" "));
    spans.push(Span::raw(value));
    spans.push(Span::raw(" ".repeat(padding)));
    spans.push(Span::raw("│"));
  }
  Line::from(spans)
}

fn table_bottom(width: usize, elapsed: &str, columns: &[usize]) -> Line {
  let notch = format!("┤ {elapsed} ┘");
  let rule_width = width.saturating_sub(1 + display_width(&notch));
  let mut rule = String::new();
  for (index, column_width) in columns.iter().enumerate() {
    rule.push_str(&"─".repeat(column_width.saturating_add(1)));
    if index + 1 < columns.len() {
      rule.push('┴');
    }
  }
  rule = fit_rule(&rule, rule_width);
  Line::from(format!("└{rule}{notch}"))
}

fn fit_rule(rule: &str, width: usize) -> String {
  let mut fitted = rule.chars().take(width).collect::<String>();
  fitted.push_str(&"─".repeat(width.saturating_sub(display_width(&fitted))));
  fitted
}

fn transfer_cell(activity: &CacheActivity, detail: bool) -> String {
  let paths = activity.active.saturating_add(activity.completed);
  let prefix = format!("{}/{}", activity.completed, paths);
  if !activity.has_unknown_size && activity.bytes_total > 0 {
    let percent = activity
      .bytes_done
      .saturating_mul(100)
      .checked_div(activity.bytes_total)
      .unwrap_or(0);
    if detail {
      format!(
        "{prefix} · {percent}% {}/{}",
        format_bytes(activity.bytes_done),
        format_bytes(activity.bytes_total)
      )
    } else {
      format!("{prefix} · {percent}%")
    }
  } else if detail && activity.bytes_done > 0 {
    format!("{prefix} · {}", format_bytes(activity.bytes_done))
  } else {
    prefix
  }
}

fn display_width(value: &str) -> usize {
  UnicodeWidthStr::width(value)
}

fn fit_text(value: &str, max_width: usize) -> String {
  if display_width(value) <= max_width {
    return value.to_string();
  }
  if max_width == 0 {
    return String::new();
  }
  if max_width == 1 {
    return "…".to_string();
  }
  let mut output = String::new();
  for ch in value.chars() {
    if display_width(&output).saturating_add(ch.width().unwrap_or(0))
      >= max_width
    {
      break;
    }
    output.push(ch);
  }
  output.push('…');
  output
}

fn cache_activity(
  state: &RenderSnapshot,
) -> (
  BTreeMap<String, CacheActivity>,
  BTreeMap<String, CacheActivity>,
) {
  let mut from = BTreeMap::new();
  let mut to = BTreeMap::new();
  for transfer in state.full_summary.running_downloads.values() {
    add_running_cache(&mut from, transfer);
  }
  for transfer in state.full_summary.running_uploads.values() {
    add_running_cache(&mut to, transfer);
  }
  for transfer in &state.completed_download_hosts {
    add_completed_cache(
      &mut from,
      &transfer.host,
      transfer.completed,
      transfer.total_bytes,
    );
  }
  for transfer in &state.completed_upload_hosts {
    add_completed_cache(
      &mut to,
      &transfer.host,
      transfer.completed,
      transfer.total_bytes,
    );
  }
  (from, to)
}

fn add_running_cache(
  caches: &mut BTreeMap<String, CacheActivity>,
  transfer: &crate::state::TransferInfo,
) {
  let Some(host) = cache_host_label(&transfer.host) else {
    return;
  };
  let cache = caches.entry(host).or_default();
  cache.active += 1;
  cache.bytes_done =
    cache.bytes_done.saturating_add(transfer.bytes_transferred);
  if let Some(total) = transfer.total_bytes {
    cache.bytes_total = cache.bytes_total.saturating_add(total);
  } else {
    cache.has_unknown_size = true;
  }
}

fn add_completed_cache(
  caches: &mut BTreeMap<String, CacheActivity>,
  host: &cognos::Host,
  completed: usize,
  bytes: u64,
) {
  let Some(host) = cache_host_label(host) else {
    return;
  };
  let cache = caches.entry(host).or_default();
  cache.completed = cache.completed.saturating_add(completed);
  cache.bytes_done = cache.bytes_done.saturating_add(bytes);
  cache.bytes_total = cache.bytes_total.saturating_add(bytes);
}

fn format_bytes(bytes: u64) -> String {
  const KIB: f64 = 1024.0;
  const MIB: f64 = KIB * 1024.0;
  const GIB: f64 = MIB * 1024.0;
  let bytes = bytes as f64;
  if bytes >= GIB {
    format!("{:.1}G", bytes / GIB)
  } else if bytes >= MIB {
    format!("{:.1}M", bytes / MIB)
  } else if bytes >= KIB {
    format!("{:.1}K", bytes / KIB)
  } else {
    format!("{bytes:.0}B")
  }
}

fn cache_host_label(host: &cognos::Host) -> Option<String> {
  let cognos::Host::Remote(host) = host else {
    return None;
  };
  let without_scheme = host
    .split_once("://")
    .map_or(host.as_str(), |(_, rest)| rest);
  let without_user = without_scheme
    .rsplit_once('@')
    .map_or(without_scheme, |(_, rest)| rest);
  let authority = without_user.split('/').next().unwrap_or(without_user);
  if let Some(bracket) = authority.strip_prefix('[')
    && let Some((address, _)) = bracket.split_once(']')
  {
    return Some(address.to_string());
  }
  let without_port = authority
    .rsplit_once(':')
    .filter(|(_, port)| port.chars().all(|ch| ch.is_ascii_digit()))
    .map_or(authority, |(name, _)| name);
  Some(without_port.to_string())
}
