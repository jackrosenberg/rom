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
  Screen,
  ScreenCell,
  Style,
};
use self::{
  activity::{
    minimum_required_activity_rows,
    render_activity_graph_lines,
    render_flat_active_lines,
  },
  screen::{Line, Span},
};
use crate::{
  console::{ConsoleConfig, format_duration},
  presentation::{Glyphs, Palette, PresentationStyle, RenderOptions},
  state::{RenderSnapshot, State, current_time},
};

const TEXT_PRIMARY: Color = Palette::TEXT_PRIMARY;
const TEXT_MUTED: Color = Palette::TEXT_MUTED;
const GRAPH_LINE_COLOR: Color = Palette::GRAPH_LINE;
const TABLE_HEADER_COLOR: Color = Palette::TABLE_HEADER;
const MOSS_GREEN: Color = Palette::MOSS_GREEN;
const BUILT_GREEN: Color = Palette::BUILT_GREEN;
const DOWNLOAD_BLUE: Color = Palette::DOWNLOAD_BLUE;
const UPLOAD_PURPLE: Color = Palette::UPLOAD_PURPLE;
const MUTED_RED: Color = Palette::MUTED_RED;
const MUTED_YELLOW: Color = Palette::MUTED_YELLOW;
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
  minimum_required_graph_rows_at_width_with_options(
    width,
    state,
    RenderOptions::default(),
  )
}

/// Minimum live-region height for a selected presentation preset.
#[must_use]
pub fn minimum_required_graph_rows_at_width_with_options(
  width: u16,
  state: &RenderSnapshot,
  options: RenderOptions,
) -> usize {
  match options.style {
    PresentationStyle::Plain => {
      render_flat_active_lines(state, usize::from(width)).len() + 1
    },
    PresentationStyle::Dashboard => {
      if has_presentable_work(state) {
        6
      } else {
        1
      }
    },
    _ => {
      minimum_required_activity_rows(state).saturating_add(
        footer_lines_for_style(state, usize::from(width), options.style).len(),
      )
    },
  }
}

/// Render the compact status and activity graph within a soft height budget.
#[must_use]
pub fn render_graph_screen(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
  config: &TuiConfig,
) -> Screen {
  render_graph_screen_with_options(
    width,
    soft_height,
    state,
    config,
    RenderOptions::default(),
  )
}

/// Render a live graph using an explicit presentation preset.
#[must_use]
pub fn render_graph_screen_with_options(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
  config: &TuiConfig,
  options: RenderOptions,
) -> Screen {
  render_preset_graph_screen(width, soft_height, state, config, options.style)
}

fn render_preset_graph_screen(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
  config: &TuiConfig,
  style: PresentationStyle,
) -> Screen {
  match style {
    PresentationStyle::Plain => render_plain_screen(width, soft_height, state),
    PresentationStyle::Dashboard => {
      render_dashboard_screen(width, soft_height, state)
    },
    PresentationStyle::TableSummary | PresentationStyle::FullSummary => {
      render_graph_screen_inner(
        width,
        soft_height,
        state,
        config,
        true,
        PresentationStyle::Connected,
      )
    },
    _ => {
      render_graph_screen_inner(width, soft_height, state, config, true, style)
    },
  }
}

fn render_graph_screen_inner(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
  config: &TuiConfig,
  show_wait_timer: bool,
  style: PresentationStyle,
) -> Screen {
  if soft_height == 0 {
    return Screen::new(width, 0);
  }
  let footer_height = console_footer_height(width, soft_height, state, style);
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
  draw_console_footer(&mut screen, graph_height, footer_height, state, style);
  screen
}

/// Render the stable post-build graph with the live renderer.
#[must_use]
pub fn render_final_graph_screen(
  width: u16,
  state: &State,
  config: &TuiConfig,
) -> Screen {
  render_final_graph_screen_with_options(
    width,
    state,
    config,
    RenderOptions::default(),
  )
}

/// Render a final graph using an explicit presentation preset.
#[must_use]
pub fn render_final_graph_screen_with_options(
  width: u16,
  state: &State,
  config: &TuiConfig,
  options: RenderOptions,
) -> Screen {
  render_preset_final_graph_screen(width, state, config, options.style)
}

fn render_preset_final_graph_screen(
  width: u16,
  state: &State,
  config: &TuiConfig,
  style: PresentationStyle,
) -> Screen {
  let snapshot = state.render_snapshot();
  let optional_rows = snapshot
    .derivation_infos
    .len()
    .saturating_add(snapshot.full_summary.running_downloads.len())
    .saturating_add(snapshot.full_summary.running_uploads.len())
    .saturating_add(2)
    .min(config.console.max_visible_lines.saturating_add(2));
  let soft_height = u16::try_from(optional_rows.max(
    minimum_required_graph_rows_at_width_with_options(
      width,
      &snapshot,
      style.into(),
    ),
  ))
  .unwrap_or(u16::MAX);
  match style {
    PresentationStyle::Plain => {
      render_plain_screen(width, soft_height, &snapshot)
    },
    PresentationStyle::Dashboard => {
      render_dashboard_screen(width, soft_height, &snapshot)
    },
    PresentationStyle::TableSummary | PresentationStyle::FullSummary => {
      let connected = render_graph_screen_inner(
        width,
        soft_height,
        &snapshot,
        config,
        false,
        PresentationStyle::Connected,
      );
      let summary = render_final_summary_screen(width, state, config, style);
      connected.append_below(summary)
    },
    _ => {
      render_graph_screen_inner(
        width,
        soft_height,
        &snapshot,
        config,
        false,
        style,
      )
    },
  }
}

#[derive(Clone, Debug, Default)]
struct FinalHostSummary {
  built:            usize,
  failed:           usize,
  downloaded:       usize,
  downloaded_bytes: u64,
  uploaded:         usize,
  uploaded_bytes:   u64,
}

fn render_final_summary_screen(
  width: u16,
  state: &State,
  config: &TuiConfig,
  style: PresentationStyle,
) -> Screen {
  let lines = match style {
    PresentationStyle::TableSummary => {
      final_table_summary_lines(width, state, config)
    },
    PresentationStyle::FullSummary => {
      final_full_summary_lines(width, state, config)
    },
    _ => Vec::new(),
  };
  let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
  let mut screen = Screen::new(width, height);
  screen.draw_text(0, 0, width, height, &lines, false);
  screen
}

fn final_host_summaries(state: &State) -> BTreeMap<String, FinalHostSummary> {
  let mut hosts = BTreeMap::<String, FinalHostSummary>::new();
  for build in state.full_summary.completed_builds.values() {
    let summary = hosts.entry(host_label(&build.host)).or_default();
    summary.built = summary.built.saturating_add(1);
  }
  for build in state.full_summary.failed_builds.values() {
    let summary = hosts.entry(host_label(&build.host)).or_default();
    summary.failed = summary.failed.saturating_add(1);
  }
  for transfer in state.full_summary.completed_downloads.values() {
    let summary = hosts.entry(host_label(&transfer.host)).or_default();
    summary.downloaded = summary.downloaded.saturating_add(1);
    summary.downloaded_bytes = summary
      .downloaded_bytes
      .saturating_add(transfer.total_bytes);
  }
  for transfer in state.full_summary.completed_uploads.values() {
    let summary = hosts.entry(host_label(&transfer.host)).or_default();
    summary.uploaded = summary.uploaded.saturating_add(1);
    summary.uploaded_bytes =
      summary.uploaded_bytes.saturating_add(transfer.total_bytes);
  }
  hosts
}

fn final_table_summary_lines(
  width: u16,
  state: &State,
  config: &TuiConfig,
) -> Vec<Line> {
  let hosts = final_host_summaries(state);
  let totals = final_totals(state);
  let mut rows = hosts
    .iter()
    .map(|(host, summary)| final_table_values(host, summary))
    .collect::<Vec<_>>();
  rows.push(final_table_values("Total", &totals));

  let titles = ["HOST", "BUILT", "FAILED", "DOWNLOADED", "UPLOADED"];
  let mut widths = titles.map(|title| display_width(title).saturating_add(2));
  for row in &rows {
    for (column, value) in row.iter().enumerate() {
      widths[column] = widths[column].max(display_width(value));
    }
  }
  shrink_final_table_columns(&mut widths, usize::from(width));

  let mut lines = vec![table_header("┌", &titles, &widths, "┐")];
  for (index, row) in rows.iter().enumerate() {
    let host_style = if index + 1 == rows.len() {
      Style::default()
        .fg(TEXT_PRIMARY)
        .add_attribute(Attribute::Bold)
    } else {
      Style::default().fg(TEXT_PRIMARY)
    };
    lines.push(table_row(row, &widths, &[
      host_style,
      Style::default().fg(BUILT_GREEN),
      Style::default().fg(MUTED_RED),
      Style::default().fg(DOWNLOAD_BLUE),
      Style::default().fg(UPLOAD_PURPLE),
    ]));
  }
  lines.push(final_table_bottom(&widths));
  lines.push(final_outcome_line(state, config));
  lines
}

fn final_table_values(host: &str, summary: &FinalHostSummary) -> Vec<String> {
  vec![
    host.to_string(),
    summary.built.to_string(),
    summary.failed.to_string(),
    transfer_final_value(summary.downloaded, summary.downloaded_bytes),
    transfer_final_value(summary.uploaded, summary.uploaded_bytes),
  ]
}

fn transfer_final_value(count: usize, bytes: u64) -> String {
  if bytes == 0 {
    count.to_string()
  } else {
    format!("{count} · {}", format_bytes(bytes))
  }
}

fn final_totals(state: &State) -> FinalHostSummary {
  FinalHostSummary {
    built:            state.full_summary.completed_builds.len(),
    failed:           state.full_summary.failed_builds.len(),
    downloaded:       state.full_summary.completed_downloads.len(),
    downloaded_bytes: state
      .full_summary
      .completed_downloads
      .values()
      .fold(0_u64, |total, transfer| {
        total.saturating_add(transfer.total_bytes)
      }),
    uploaded:         state.full_summary.completed_uploads.len(),
    uploaded_bytes:   state
      .full_summary
      .completed_uploads
      .values()
      .fold(0_u64, |total, transfer| {
        total.saturating_add(transfer.total_bytes)
      }),
  }
}

fn shrink_final_table_columns(widths: &mut [usize; 5], width: usize) {
  let available = width.saturating_sub(1 + widths.len().saturating_mul(2));
  while widths.iter().sum::<usize>() > available {
    let Some((largest, _)) = widths
      .iter()
      .enumerate()
      .filter(|(_, width)| **width > 1)
      .max_by_key(|(_, width)| **width)
    else {
      break;
    };
    widths[largest] = widths[largest].saturating_sub(1);
  }
}

fn final_table_bottom(widths: &[usize]) -> Line {
  let mut spans = vec![Span::styled("└", hierarchy_style())];
  for (index, width) in widths.iter().enumerate() {
    spans.push(Span::styled(
      "─".repeat(width.saturating_add(2)),
      hierarchy_style(),
    ));
    spans.push(Span::styled(
      if index + 1 == widths.len() {
        "┘"
      } else {
        "┴"
      },
      hierarchy_style(),
    ));
  }
  Line::from(spans)
}

fn final_full_summary_lines(
  width: u16,
  state: &State,
  config: &TuiConfig,
) -> Vec<Line> {
  let totals = final_totals(state);
  let mut lines = vec![
    final_summary_heading(
      usize::from(width).clamp(1, CONNECTED_TABLE_MAX_WIDTH),
    ),
    verbose_detail_line(
      "Built",
      &format!("{} builds", totals.built),
      Style::default().fg(BUILT_GREEN),
    ),
    verbose_detail_line(
      "Failed",
      &format!("{} builds", totals.failed),
      Style::default().fg(MUTED_RED),
    ),
    verbose_detail_line(
      "Downloaded",
      &format!(
        "{} paths · {}",
        totals.downloaded,
        format_bytes(totals.downloaded_bytes)
      ),
      Style::default().fg(DOWNLOAD_BLUE),
    ),
    verbose_detail_line(
      "Uploaded",
      &format!(
        "{} paths · {}",
        totals.uploaded,
        format_bytes(totals.uploaded_bytes)
      ),
      Style::default().fg(UPLOAD_PURPLE),
    ),
    verbose_detail_line(
      "Nix errors",
      &state.nix_errors.len().to_string(),
      if state.nix_errors.is_empty() {
        Style::default().fg(TEXT_PRIMARY)
      } else {
        Style::default().fg(MUTED_RED)
      },
    ),
  ];
  // Full summary is deliberately a strict superset of table summary: retain
  // the per-host diagnostics and append its single outcome line.
  lines.extend(final_table_summary_lines(width, state, config));
  lines
}

fn final_outcome_line(state: &State, config: &TuiConfig) -> Line {
  let failed = state.full_summary.failed_builds.len();
  let nix_errors = state.nix_errors.len();
  let process_failure =
    config.console.process_exit_code.filter(|code| *code != 0);
  let mut reasons = Vec::new();
  if failed > 0 {
    reasons.push(format!(
      "{failed} build {}",
      if failed == 1 { "failure" } else { "failures" }
    ));
  }
  if nix_errors > 0 {
    reasons.push(format!(
      "{nix_errors} nix {}",
      if nix_errors == 1 { "error" } else { "errors" }
    ));
  }
  if let Some(code) = process_failure {
    reasons.push(format!("evaluator status {code}"));
  }
  let failed = !reasons.is_empty();
  let outcome = if failed { "failed" } else { "success" };
  let detail = if reasons.is_empty() {
    String::new()
  } else {
    format!(" · {}", reasons.join(" · "))
  };
  let duration = format_duration(current_time() - state.start_time);
  Line::from(vec![
    Span::styled("└─ ", hierarchy_style()),
    Span::styled("Outcome ", Style::default().fg(TABLE_HEADER_COLOR)),
    Span::styled(outcome, verbose_status_style(outcome)),
    Span::styled(detail, secondary_style()),
    Span::styled(" · Time ", Style::default().fg(TABLE_HEADER_COLOR)),
    Span::styled(duration, Style::default().fg(TEXT_PRIMARY)),
    Span::styled(" ┘", hierarchy_style()),
  ])
}

fn render_plain_screen(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
) -> Screen {
  if soft_height == 0 {
    return Screen::new(width, 0);
  }
  if !has_presentable_work(state) {
    return elapsed_screen(width, state);
  }

  let mut lines = render_flat_active_lines(state, usize::from(width));
  let available = usize::from(soft_height);
  if lines.len() < available {
    lines.push(plain_summary_line(state));
  }
  lines.truncate(available);
  let height = u16::try_from(lines.len()).unwrap_or(soft_height);
  let mut screen = Screen::new(width, height);
  screen.draw_text(0, 0, width, height, &lines, false);
  screen
}

fn render_dashboard_screen(
  width: u16,
  soft_height: u16,
  state: &RenderSnapshot,
) -> Screen {
  if soft_height == 0 {
    return Screen::new(width, 0);
  }
  if !has_presentable_work(state) {
    return elapsed_screen(width, state);
  }

  let lines = dashboard_lines(state);
  let height = soft_height.min(u16::try_from(lines.len()).unwrap_or(u16::MAX));
  let mut screen = Screen::new(width, height);
  screen.draw_text(0, 0, width, height, &lines, false);
  screen
}

fn elapsed_screen(width: u16, state: &RenderSnapshot) -> Screen {
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
  screen
}

fn has_active_work(state: &RenderSnapshot) -> bool {
  !state.full_summary.failed_builds.is_empty()
    || !state.full_summary.running_builds.is_empty()
    || !state.full_summary.running_downloads.is_empty()
    || !state.full_summary.running_uploads.is_empty()
}

fn has_presentable_work(state: &RenderSnapshot) -> bool {
  build_total(state) > 0
    || !state.full_summary.planned_downloads.is_empty()
    || !state.full_summary.running_downloads.is_empty()
    || !state.full_summary.running_uploads.is_empty()
    || !state.completed_download_hosts.is_empty()
    || !state.completed_upload_hosts.is_empty()
}

fn plain_summary_line(state: &RenderSnapshot) -> Line {
  Line::from(vec![
    Span::styled(
      format!(
        "{} builds · {} running · {} waiting · {} done · {} failed",
        build_total(state),
        state.running_build_count,
        state.planned_build_count,
        state.completed_build_count,
        state.failed_build_count,
      ),
      Style::default().fg(MOSS_GREEN),
    ),
    Span::styled(
      format!(" · {}", format_duration(current_time() - state.start_time)),
      secondary_style(),
    ),
  ])
}

fn dashboard_lines(state: &RenderSnapshot) -> Vec<Line> {
  let summary = &state.full_summary;
  let roots = if state.total_root_count == 0 {
    "none".to_string()
  } else {
    let names = state
      .forest_roots
      .iter()
      .filter_map(|id| state.get_derivation_info(*id))
      .map(|info| info.name.name.as_str())
      .take(2)
      .collect::<Vec<_>>()
      .join(", ");
    if names.is_empty() {
      state.total_root_count.to_string()
    } else {
      format!("{} · {names}", state.total_root_count)
    }
  };
  let status = if !summary.failed_builds.is_empty() {
    "failed"
  } else if has_active_work(state) {
    "active"
  } else if !summary.planned_builds.is_empty()
    || !summary.planned_downloads.is_empty()
  {
    "waiting"
  } else {
    "complete"
  };
  let hosts = active_host_labels(state);
  vec![
    dashboard_line("Root", roots, Style::default().fg(TEXT_PRIMARY)),
    dashboard_line(
      "Builds",
      format!(
        "{} total · {} running · {} waiting · {} done · {} failed",
        build_total(state),
        state.running_build_count,
        state.planned_build_count,
        state.completed_build_count,
        state.failed_build_count,
      ),
      Style::default().fg(MOSS_GREEN),
    ),
    dashboard_line(
      "Transfers",
      format!(
        "{} pull · {} push",
        summary.running_downloads.len(),
        summary.running_uploads.len()
      ),
      Style::default().fg(DOWNLOAD_BLUE),
    ),
    dashboard_line(
      "Host",
      if hosts.is_empty() {
        "none".to_string()
      } else {
        hosts.join(", ")
      },
      Style::default().fg(UPLOAD_PURPLE),
    ),
    dashboard_line("Status", status.to_string(), verbose_status_style(status)),
    dashboard_line(
      "Duration",
      format_duration(current_time() - state.start_time),
      Style::default().fg(TEXT_PRIMARY),
    ),
  ]
}

fn dashboard_line(label: &str, value: String, style: Style) -> Line {
  Line::from(vec![
    Span::styled(
      format!("{label:<10}"),
      Style::default()
        .fg(TABLE_HEADER_COLOR)
        .add_attribute(Attribute::Bold),
    ),
    Span::styled(value, style),
  ])
}

fn active_host_labels(state: &RenderSnapshot) -> Vec<String> {
  let mut hosts = state
    .full_summary
    .running_builds
    .values()
    .map(|build| host_label(&build.host))
    .chain(
      state
        .full_summary
        .running_downloads
        .values()
        .map(|transfer| host_label(&transfer.host)),
    )
    .chain(
      state
        .full_summary
        .running_uploads
        .values()
        .map(|transfer| host_label(&transfer.host)),
    )
    .collect::<Vec<_>>();
  hosts.sort();
  hosts.dedup();
  hosts
}

fn host_label(host: &cognos::Host) -> String {
  match host {
    cognos::Host::Localhost => "localhost".to_string(),
    cognos::Host::Remote(_) => {
      cache_host_label(host).unwrap_or_else(|| "remote".to_string())
    },
  }
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
  let frame = ((now * 1000.0) as usize / 80) % Glyphs::SPINNER_FRAMES.len();
  Glyphs::SPINNER_FRAMES[frame]
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
  style: PresentationStyle,
) -> u16 {
  if height == 0 {
    return 0;
  }
  let desired = footer_lines_for_style(state, usize::from(width), style).len();
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
  style: PresentationStyle,
) {
  if height == 0 || y >= screen.height() {
    return;
  }
  let lines = if height == 1 {
    vec![compact_footer_line(state, usize::from(screen.width()))]
  } else {
    footer_lines_for_style(state, usize::from(screen.width()), style)
  };
  screen.draw_text(0, y, screen.width(), height, &lines, false);
}

const CONNECTED_TABLE_MAX_WIDTH: usize = 80;

fn footer_lines_for_style(
  state: &RenderSnapshot,
  width: usize,
  style: PresentationStyle,
) -> Vec<Line> {
  match style {
    PresentationStyle::Compact => vec![compact_footer_line(state, width)],
    PresentationStyle::Verbose => verbose_footer_lines(state, width),
    PresentationStyle::Connected
    | PresentationStyle::Plain
    | PresentationStyle::Dashboard
    | PresentationStyle::TableSummary
    | PresentationStyle::FullSummary => footer_lines(state, width),
  }
}

fn footer_lines(state: &RenderSnapshot, width: usize) -> Vec<Line> {
  let width = width.clamp(1, CONNECTED_TABLE_MAX_WIDTH);
  let (pulls, pushes) = cache_activity(state);
  let mut hosts = pulls
    .keys()
    .chain(pushes.keys())
    .cloned()
    .collect::<Vec<_>>();
  hosts.sort();
  hosts.dedup();
  let longest_host = hosts
    .iter()
    .map(|host| display_width(host))
    .max()
    .unwrap_or(0);
  let columns = FooterColumns::for_width(width, longest_host);

  let mut lines = Vec::new();
  if !hosts.is_empty() {
    lines.push(table_header(
      "├",
      &[columns.host_title, columns.pull_title, columns.push_title],
      &columns.host_widths,
      "┐",
    ));
    for host in hosts {
      let pull_activity = pulls.get(&host);
      let push_activity = pushes.get(&host);
      let pull = pull_activity
        .map(|activity| transfer_cell(activity, columns.detail))
        .unwrap_or_else(|| "—".to_string());
      let push = push_activity
        .map(|activity| transfer_cell(activity, columns.detail))
        .unwrap_or_else(|| "—".to_string());
      lines.push(table_row(&[host, pull, push], &columns.host_widths, &[
        Style::default().fg(TEXT_PRIMARY),
        Style::default().fg(if pull_activity.is_some() {
          DOWNLOAD_BLUE
        } else {
          TEXT_MUTED
        }),
        Style::default().fg(if push_activity.is_some() {
          UPLOAD_PURPLE
        } else {
          TEXT_MUTED
        }),
      ]));
    }
  }

  lines.push(table_header(
    "├",
    &columns.build_titles,
    &columns.build_widths,
    "┤",
  ));
  let total = build_total(state);
  lines.push(table_row(
    &[
      format!("{total} builds"),
      state.running_build_count.to_string(),
      state.planned_build_count.to_string(),
      state.completed_build_count.to_string(),
      state.failed_build_count.to_string(),
    ],
    &columns.build_widths,
    &[
      Style::default().fg(TEXT_PRIMARY),
      Style::default().fg(MOSS_GREEN),
      Style::default().fg(MUTED_YELLOW),
      Style::default().fg(BUILT_GREEN),
      Style::default().fg(MUTED_RED),
    ],
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
  fn for_width(width: usize, longest_host: usize) -> Self {
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
    let host_available = width.saturating_sub(7);
    let pull_min = display_width(&format!(" {pull_title} "));
    let push_min = display_width(&format!(" {push_title} "));
    let host_min = display_width(&format!(" {host_title} "));
    let mut host_widths = if host_available
      >= host_min.saturating_add(pull_min).saturating_add(push_min)
    {
      let host_width = longest_host
        .max(host_min)
        .min(host_available.saturating_sub(pull_min.saturating_add(push_min)));
      vec![host_width, pull_min, push_min]
    } else {
      distribute_columns(host_available, 3, &[2, 1, 1])
    };
    let assigned = host_widths.iter().sum::<usize>();
    let extra = host_available.saturating_sub(assigned);
    for index in 0..extra {
      host_widths[1 + index % 2] += 1;
    }

    Self {
      host_title,
      pull_title,
      push_title,
      build_titles,
      host_widths,
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
  let width = width.clamp(1, CONNECTED_TABLE_MAX_WIDTH);
  let total = build_total(state);
  let elapsed = format_duration(current_time() - state.start_time);
  let status = if width >= 64 {
    format!(
      " {total} builds · {} running · {} waiting · {} done · {} failed ",
      state.running_build_count,
      state.planned_build_count,
      state.completed_build_count,
      state.failed_build_count,
    )
  } else {
    format!(
      " {total} builds · {}/{}/{}/{} ",
      state.running_build_count,
      state.planned_build_count,
      state.completed_build_count,
      state.failed_build_count,
    )
  };
  let notch = format!("┤ {elapsed} ┘");
  let content_width = width.saturating_sub(2 + display_width(&notch));
  let content = fit_text(&status, content_width);
  let rule = "─".repeat(content_width.saturating_sub(display_width(&content)));
  Line::from(vec![
    Span::styled("└─", hierarchy_style()),
    Span::styled(content, Style::default().fg(MOSS_GREEN)),
    Span::styled(rule, hierarchy_style()),
    Span::styled(notch, secondary_style()),
  ])
}

fn verbose_footer_lines(state: &RenderSnapshot, width: usize) -> Vec<Line> {
  let width = width.clamp(1, CONNECTED_TABLE_MAX_WIDTH);
  let summary = &state.full_summary;
  let (pulls, pushes) = cache_activity(state);
  let pull = aggregate_cache_activity(pulls.values());
  let push = aggregate_cache_activity(pushes.values());
  let mut hosts = pulls
    .keys()
    .chain(pushes.keys())
    .cloned()
    .collect::<Vec<_>>();
  hosts.sort();
  hosts.dedup();

  let host_text = if hosts.is_empty() {
    "none".to_string()
  } else {
    hosts.join(", ")
  };
  let status = if !summary.failed_builds.is_empty() {
    "failed"
  } else if !summary.running_builds.is_empty()
    || !summary.running_downloads.is_empty()
    || !summary.running_uploads.is_empty()
  {
    "active"
  } else if !summary.planned_builds.is_empty()
    || !summary.planned_downloads.is_empty()
  {
    "waiting"
  } else {
    "complete"
  };
  let elapsed = format_duration(current_time() - state.start_time);

  vec![
    verbose_heading(width),
    verbose_detail_line(
      "Builds",
      &format!(
        "{} total · {} running · {} waiting · {} done · {} failed",
        build_total(state),
        state.running_build_count,
        state.planned_build_count,
        state.completed_build_count,
        state.failed_build_count,
      ),
      Style::default().fg(MOSS_GREEN),
    ),
    verbose_detail_line(
      "Transfers",
      &format!(
        "pull {} · push {}",
        transfer_cell(&pull, true),
        transfer_cell(&push, true),
      ),
      Style::default().fg(DOWNLOAD_BLUE),
    ),
    verbose_detail_line(
      "Hosts",
      &host_text,
      Style::default().fg(UPLOAD_PURPLE),
    ),
    Line::from(vec![
      Span::styled("└─ ", hierarchy_style()),
      Span::styled("Elapsed ", Style::default().fg(TABLE_HEADER_COLOR)),
      Span::styled(elapsed, Style::default().fg(TEXT_PRIMARY)),
      Span::styled(" · ", secondary_style()),
      Span::styled(status, verbose_status_style(status)),
      Span::styled(" ┘", hierarchy_style()),
    ]),
  ]
}

fn verbose_heading(width: usize) -> Line {
  summary_heading(width, " DETAILS ")
}

fn final_summary_heading(width: usize) -> Line {
  summary_heading(width, " FULL SUMMARY ")
}

fn summary_heading(width: usize, label: &str) -> Line {
  let rule = "─".repeat(width.saturating_sub(2 + display_width(label)));
  Line::from(vec![
    Span::styled("├─", hierarchy_style()),
    Span::styled(
      label,
      Style::default()
        .fg(TABLE_HEADER_COLOR)
        .add_attribute(Attribute::Bold),
    ),
    Span::styled(rule, hierarchy_style()),
  ])
}

fn verbose_detail_line(label: &str, value: &str, value_style: Style) -> Line {
  Line::from(vec![
    Span::styled("│ ", hierarchy_style()),
    Span::styled(
      format!("{label:<10}"),
      Style::default().fg(TABLE_HEADER_COLOR),
    ),
    Span::styled(value, value_style),
  ])
}

fn verbose_status_style(status: &str) -> Style {
  Style::default().fg(match status {
    "failed" => MUTED_RED,
    "waiting" => MUTED_YELLOW,
    "complete" => BUILT_GREEN,
    _ => MOSS_GREEN,
  })
}

fn build_total(state: &RenderSnapshot) -> usize {
  state
    .planned_build_count
    .saturating_add(state.running_build_count)
    .saturating_add(state.completed_build_count)
    .saturating_add(state.failed_build_count)
}

fn aggregate_cache_activity<'a>(
  activities: impl Iterator<Item = &'a CacheActivity>,
) -> CacheActivity {
  activities.fold(CacheActivity::default(), |mut total, activity| {
    total.active = total.active.saturating_add(activity.active);
    total.completed = total.completed.saturating_add(activity.completed);
    total.bytes_done = total.bytes_done.saturating_add(activity.bytes_done);
    total.bytes_total = total.bytes_total.saturating_add(activity.bytes_total);
    total.has_unknown_size |= activity.has_unknown_size;
    total
  })
}

fn table_header(
  left: &str,
  titles: &[&str],
  widths: &[usize],
  right: &str,
) -> Line {
  let mut spans = vec![Span::styled(left, hierarchy_style())];
  for (index, (title, width)) in titles.iter().zip(widths).enumerate() {
    let label = fit_text(&format!(" {title} "), *width);
    spans.push(Span::styled("─", hierarchy_style()));
    spans.push(Span::styled(
      label.clone(),
      Style::default()
        .fg(TABLE_HEADER_COLOR)
        .add_attribute(Attribute::Bold),
    ));
    spans.push(Span::styled(
      "─".repeat(width.saturating_sub(display_width(&label))),
      hierarchy_style(),
    ));
    spans.push(Span::styled(
      if index + 1 == titles.len() {
        right
      } else {
        "┬"
      },
      hierarchy_style(),
    ));
  }
  Line::from(spans)
}

fn table_row(values: &[String], widths: &[usize], styles: &[Style]) -> Line {
  let mut spans = vec![Span::styled("│", hierarchy_style())];
  for ((value, width), style) in values.iter().zip(widths).zip(styles) {
    let value = fit_text(value, *width);
    let padding = width.saturating_sub(display_width(&value));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(value, *style));
    spans.push(Span::raw(" ".repeat(padding)));
    spans.push(Span::styled("│", hierarchy_style()));
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
  Line::from(vec![
    Span::styled(format!("└{rule}"), hierarchy_style()),
    Span::styled(notch, secondary_style()),
  ])
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
