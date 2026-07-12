//! Full-screen styled framebuffer renderer for ROM.
use std::collections::BTreeMap;

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

const TEXT_PRIMARY: Color = Color::Rgb {
  r: 224,
  g: 241,
  b: 247,
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
  if soft_height == 0 {
    return Screen::new(width, 0);
  }
  let footer_height = console_footer_height(width, soft_height, state);
  let graph_budget = soft_height.saturating_sub(footer_height);
  let mut lines = render_activity_graph_lines(
    state,
    config.console,
    usize::from(graph_budget),
    usize::from(width),
  )
  .lines;
  if lines.is_empty() {
    lines.push(idle_graph_line(state, usize::from(width)));
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
  render_graph_screen(width, soft_height, &snapshot, config)
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

fn idle_graph_line(state: &RenderSnapshot, width: usize) -> Line {
  if let Some(line) = evaluation_graph_line(state, width) {
    return line;
  }

  Line::from(Span::styled(
    "Waiting for Nix activity...",
    secondary_style(),
  ))
}

fn evaluation_graph_line(state: &RenderSnapshot, width: usize) -> Option<Line> {
  let eval = &state.evaluation_state;
  if eval.count == 0 && eval.last_file_name.is_none() {
    return None;
  }

  let count_label = if eval.count == 1 {
    "1 file".to_string()
  } else {
    format!("{} files", eval.count)
  };
  let name_budget = width
    .saturating_sub(" Evaluating ".len() + count_label.len() + 4)
    .clamp(12, 72);
  let file = eval.last_file_name.as_deref().map_or_else(
    || "Nix expression".to_string(),
    |path| compact_eval_path(path, name_budget),
  );

  Some(Line::from(vec![
    Span::styled(
      spinner_frame(current_time()),
      Style::default().fg(MUTED_YELLOW),
    ),
    Span::raw(" "),
    Span::styled("Evaluating", Style::default().fg(TEXT_PRIMARY)),
    Span::raw(" "),
    Span::styled(file, secondary_style()),
    Span::raw(" "),
    Span::styled(count_label, secondary_style()),
  ]))
}

fn spinner_frame(now: f64) -> &'static str {
  let frame = ((now * 1000.0) as usize / 80) % SPINNER_FRAMES.len();
  SPINNER_FRAMES[frame]
}

fn compact_eval_path(path: &str, max_chars: usize) -> String {
  let path = path.trim();
  let components: Vec<&str> = path
    .split('/')
    .filter(|component| !component.is_empty())
    .collect();

  if components.len() >= 3 {
    let tail = components[components.len() - 3..].join("/");
    if tail.chars().count() <= max_chars {
      return tail;
    }
  }

  truncate_start(path, max_chars)
}

fn truncate_start(value: &str, max_chars: usize) -> String {
  let len = value.chars().count();
  if len <= max_chars {
    return value.to_string();
  }
  if max_chars <= 3 {
    return ".".repeat(max_chars);
  }

  let tail: String = value.chars().skip(len - (max_chars - 3)).collect();
  format!("...{tail}")
}

fn secondary_style() -> Style {
  Style::default().fg(TEXT_MUTED)
}

pub(super) fn hierarchy_style() -> Style {
  Style::default().fg(GRAPH_LINE_COLOR)
}

const BUILD_PANEL_WIDTH: usize = 28;
const BUILD_PANEL_HEIGHT: usize = 8;
const PANEL_GAP: usize = 2;
const WIDE_PANEL_MIN_WIDTH: usize = 94;
const PROGRESS_START: Color = Color::Rgb {
  r: 73,
  g: 115,
  b: 255,
};

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
    vec![compact_build_status(state)]
  } else {
    footer_lines(state, usize::from(screen.width()))
  };
  screen.draw_text(0, y, screen.width(), height, &lines, false);
}

fn footer_lines(state: &RenderSnapshot, width: usize) -> Vec<Line> {
  let cache = cache_panel_lines(
    state,
    width.saturating_sub(BUILD_PANEL_WIDTH + PANEL_GAP),
  );
  if width >= WIDE_PANEL_MIN_WIDTH && !cache.is_empty() {
    return join_panels(build_panel_lines(state, BUILD_PANEL_WIDTH), cache);
  }

  let panel_width = width.max(BUILD_PANEL_WIDTH);
  let mut lines = compact_build_panel_lines(state, panel_width);
  if !cache.is_empty() {
    lines.extend(cache_panel_lines(state, width));
  }
  lines
}

fn join_panels(left: Vec<Line>, right: Vec<Line>) -> Vec<Line> {
  let height = left.len().max(right.len());
  (0..height)
    .map(|index| {
      let mut spans = left
        .get(index)
        .cloned()
        .unwrap_or_else(|| Line::from(" ".repeat(BUILD_PANEL_WIDTH)))
        .spans;
      spans.push(Span::raw(" ".repeat(PANEL_GAP)));
      if let Some(line) = right.get(index) {
        spans.extend(line.spans.clone());
      }
      Line::from(spans)
    })
    .collect()
}

fn compact_build_status(state: &RenderSnapshot) -> Line {
  let summary = &state.full_summary;
  let elapsed = format_duration(current_time() - state.start_time);
  Line::from(vec![
    Span::styled(
      format!("{} building", summary.running_builds.len()),
      Style::default().fg(MOSS_GREEN),
    ),
    Span::styled(
      format!("  {} waiting", summary.planned_builds.len()),
      Style::default().fg(MUTED_YELLOW),
    ),
    Span::styled(
      format!("  {} built", summary.completed_builds.len()),
      Style::default().fg(BUILT_GREEN),
    ),
    Span::styled(
      format!("  {} failed", summary.failed_builds.len()),
      Style::default().fg(MUTED_RED),
    ),
    Span::styled(format!("  {elapsed}"), secondary_style()),
  ])
}

fn compact_build_panel_lines(
  state: &RenderSnapshot,
  width: usize,
) -> Vec<Line> {
  let summary = &state.full_summary;
  let elapsed = format_duration(current_time() - state.start_time);
  let content = format!(
    "Building {} · Waiting {} · Built {} · Failed {} · Elapsed {elapsed}",
    summary.running_builds.len(),
    summary.planned_builds.len(),
    summary.completed_builds.len(),
    summary.failed_builds.len(),
  );
  vec![
    build_panel_border(state, width, 3, 0),
    build_panel_content(state, width, 3, 1, &content, TEXT_PRIMARY),
    build_panel_border(state, width, 3, 2),
  ]
}

fn build_panel_lines(state: &RenderSnapshot, width: usize) -> Vec<Line> {
  let width = width.max(BUILD_PANEL_WIDTH);
  let summary = &state.full_summary;
  let completed = summary.completed_builds.len();
  let failed = summary.failed_builds.len();
  let finished = completed.saturating_add(failed);
  let total = finished
    .saturating_add(summary.running_builds.len())
    .saturating_add(summary.planned_builds.len());
  let percent = finished.saturating_mul(100).checked_div(total).unwrap_or(0);
  let title = format!("BUILD {finished}/{total} · {percent}%");
  let rows = [
    (title, TEXT_PRIMARY),
    (
      format!("{:>3}  BUILDING", summary.running_builds.len()),
      MOSS_GREEN,
    ),
    (
      format!("{:>3}  WAITING", summary.planned_builds.len()),
      MUTED_YELLOW,
    ),
    (format!("{:>3}  BUILT", completed), BUILT_GREEN),
    (format!("{:>3}  FAILED", failed), MUTED_RED),
    (
      format!(
        "{} ELAPSED",
        format_duration(current_time() - state.start_time)
      ),
      TEXT_MUTED,
    ),
  ];
  let mut lines = Vec::with_capacity(BUILD_PANEL_HEIGHT);
  lines.push(build_panel_border(state, width, BUILD_PANEL_HEIGHT, 0));
  for (index, (content, color)) in rows.into_iter().enumerate() {
    lines.push(build_panel_content(
      state,
      width,
      BUILD_PANEL_HEIGHT,
      index + 1,
      &content,
      color,
    ));
  }
  lines.push(build_panel_border(
    state,
    width,
    BUILD_PANEL_HEIGHT,
    BUILD_PANEL_HEIGHT - 1,
  ));
  lines
}

fn build_panel_content(
  state: &RenderSnapshot,
  width: usize,
  height: usize,
  y: usize,
  content: &str,
  color: Color,
) -> Line {
  let inner = width.saturating_sub(2);
  let content = truncate_end_chars(content, inner);
  let mut spans = vec![build_border_span(state, width, height, y, 0)];
  spans.push(Span::styled(
    format!("{content:<inner$}"),
    Style::default().fg(color),
  ));
  spans.push(build_border_span(state, width, height, y, width - 1));
  Line::from(spans)
}

fn build_panel_border(
  state: &RenderSnapshot,
  width: usize,
  height: usize,
  y: usize,
) -> Line {
  Line::from(
    (0..width)
      .map(|x| build_border_span(state, width, height, y, x))
      .collect::<Vec<_>>(),
  )
}

fn build_border_span(
  state: &RenderSnapshot,
  width: usize,
  height: usize,
  y: usize,
  x: usize,
) -> Span {
  let (position, light, heavy, frontier) = border_position(width, height, x, y);
  let perimeter = width
    .saturating_mul(2)
    .saturating_add(height * 2)
    .saturating_sub(4);
  let summary = &state.full_summary;
  let built = summary.completed_builds.len();
  let failed = summary.failed_builds.len();
  let finished = built.saturating_add(failed);
  let total = finished
    .saturating_add(summary.running_builds.len())
    .saturating_add(summary.planned_builds.len());
  let filled = perimeter
    .saturating_mul(finished)
    .checked_div(total)
    .unwrap_or(0);
  let failed_cells = if failed == 0 {
    0
  } else {
    perimeter
      .saturating_mul(failed)
      .checked_div(total)
      .unwrap_or(0)
      .max(1)
      .min(filled)
  };

  if position < filled {
    let color =
      if failed_cells > 0 && position >= filled.saturating_sub(failed_cells) {
        MUTED_RED
      } else {
        progress_gradient(position, perimeter)
      };
    Span::styled(heavy, Style::default().fg(color))
  } else if position == filled && filled < perimeter && finished > 0 {
    Span::styled(
      frontier,
      Style::default()
        .fg(TEXT_PRIMARY)
        .add_attribute(Attribute::Bold),
    )
  } else {
    Span::styled(light, hierarchy_style())
  }
}

fn border_position(
  width: usize,
  height: usize,
  x: usize,
  y: usize,
) -> (usize, &'static str, &'static str, &'static str) {
  if y == 0 {
    let glyphs = if x == 0 {
      ("┌", "┏", "┏")
    } else if x + 1 == width {
      ("┐", "┓", "┓")
    } else {
      ("─", "━", "╸")
    };
    return (x, glyphs.0, glyphs.1, glyphs.2);
  }
  if x + 1 == width {
    let position = width - 1 + y;
    let glyphs = if y + 1 == height {
      ("┘", "┛", "┛")
    } else {
      ("│", "┃", "╵")
    };
    return (position, glyphs.0, glyphs.1, glyphs.2);
  }
  if y + 1 == height {
    let position = width + height - 2 + (width - 1 - x);
    let glyphs = if x == 0 {
      ("└", "┗", "┗")
    } else {
      ("─", "━", "╺")
    };
    return (position, glyphs.0, glyphs.1, glyphs.2);
  }
  let position = 2 * width + height - 3 + (height - 1 - y);
  (position, "│", "┃", "╷")
}

fn progress_gradient(position: usize, perimeter: usize) -> Color {
  let denominator = perimeter.saturating_sub(1).max(1);
  let interpolate = |start: u8, end: u8| {
    let start = usize::from(start);
    let end = usize::from(end);
    if end >= start {
      u8::try_from(start + (end - start) * position / denominator)
        .unwrap_or(end as u8)
    } else {
      u8::try_from(start - (start - end) * position / denominator)
        .unwrap_or(end as u8)
    }
  };
  let Color::Rgb {
    r: sr,
    g: sg,
    b: sb,
  } = PROGRESS_START
  else {
    return MOSS_GREEN;
  };
  let Color::Rgb {
    r: er,
    g: eg,
    b: eb,
  } = MOSS_GREEN
  else {
    return MOSS_GREEN;
  };
  Color::Rgb {
    r: interpolate(sr, er),
    g: interpolate(sg, eg),
    b: interpolate(sb, eb),
  }
}

fn cache_panel_lines(
  state: &RenderSnapshot,
  available_width: usize,
) -> Vec<Line> {
  let (from, to) = cache_activity(state);
  if from.is_empty() && to.is_empty() {
    return Vec::new();
  }
  let width = available_width.clamp(48, 64);
  let paths_width = 9;
  let progress_width = 20;
  let cache_width = width.saturating_sub(paths_width + progress_width + 4);
  let mut lines = Vec::new();
  let first_label = if from.is_empty() { "TO" } else { "FROM" };
  let first_color = if from.is_empty() {
    UPLOAD_PURPLE
  } else {
    DOWNLOAD_BLUE
  };
  lines.push(cache_header_line(
    first_label,
    first_color,
    cache_width,
    paths_width,
    progress_width,
    true,
  ));
  if !from.is_empty() {
    lines.extend(cache_rows(&from, cache_width, paths_width, progress_width));
  }
  if !to.is_empty() {
    if !from.is_empty() {
      lines.push(cache_header_line(
        "TO",
        UPLOAD_PURPLE,
        cache_width,
        paths_width,
        progress_width,
        false,
      ));
    }
    lines.extend(cache_rows(&to, cache_width, paths_width, progress_width));
  }
  lines.push(cache_bottom_line(cache_width, paths_width, progress_width));
  lines
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

fn cache_header_line(
  label: &str,
  label_color: Color,
  cache_width: usize,
  paths_width: usize,
  progress_width: usize,
  top: bool,
) -> Line {
  let left = if top { "┌" } else { "├" };
  let joint = if top { "┬" } else { "┼" };
  let right = if top { "┐" } else { "┤" };
  let label_width = label.chars().count();
  let mut spans = vec![Span::styled(left, hierarchy_style())];
  spans.push(Span::styled("─ ", hierarchy_style()));
  spans.push(Span::styled(
    label,
    Style::default()
      .fg(label_color)
      .add_attribute(Attribute::Bold),
  ));
  spans.push(Span::styled(
    format!(
      " {}",
      "─".repeat(cache_width.saturating_sub(label_width + 3))
    ),
    hierarchy_style(),
  ));
  spans.push(Span::styled(joint, hierarchy_style()));
  let paths_label = if top { " PATHS " } else { "" };
  spans.push(Span::styled(
    centered_rule(paths_label, paths_width),
    hierarchy_style(),
  ));
  spans.push(Span::styled(joint, hierarchy_style()));
  let progress_label = if top { " PROGRESS " } else { "" };
  spans.push(Span::styled(
    centered_rule(progress_label, progress_width),
    hierarchy_style(),
  ));
  spans.push(Span::styled(right, hierarchy_style()));
  Line::from(spans)
}

fn cache_rows(
  caches: &BTreeMap<String, CacheActivity>,
  cache_width: usize,
  paths_width: usize,
  progress_width: usize,
) -> Vec<Line> {
  caches
    .iter()
    .map(|(host, activity)| {
      let host = truncate_end_chars(host, cache_width.saturating_sub(2));
      let total_paths = activity.active.saturating_add(activity.completed);
      let paths = truncate_start_chars(
        &format!("{} / {total_paths}", activity.completed),
        paths_width,
      );
      let progress = truncate_end_chars(
        &cache_progress(activity),
        progress_width.saturating_sub(2),
      );
      Line::from(vec![
        Span::styled("│", hierarchy_style()),
        Span::styled(
          format!(" {host:<width$}", width = cache_width - 1),
          Style::default().fg(TEXT_PRIMARY),
        ),
        Span::styled("│", hierarchy_style()),
        Span::styled(
          format!("{paths:>width$}", width = paths_width),
          secondary_style(),
        ),
        Span::styled("│", hierarchy_style()),
        Span::styled(
          format!(" {progress:<width$}", width = progress_width - 1),
          secondary_style(),
        ),
        Span::styled("│", hierarchy_style()),
      ])
    })
    .collect()
}

fn cache_progress(activity: &CacheActivity) -> String {
  if !activity.has_unknown_size && activity.bytes_total > 0 {
    let percent = activity
      .bytes_done
      .saturating_mul(100)
      .checked_div(activity.bytes_total)
      .unwrap_or(0);
    format!(
      "{percent}% · {}/{}",
      format_bytes(activity.bytes_done),
      format_bytes(activity.bytes_total)
    )
  } else if activity.bytes_done > 0 {
    format_bytes(activity.bytes_done)
  } else {
    "—".to_string()
  }
}

fn cache_bottom_line(
  cache_width: usize,
  paths_width: usize,
  progress_width: usize,
) -> Line {
  Line::from(Span::styled(
    format!(
      "└{}┴{}┴{}┘",
      "─".repeat(cache_width),
      "─".repeat(paths_width),
      "─".repeat(progress_width)
    ),
    hierarchy_style(),
  ))
}

fn centered_rule(label: &str, width: usize) -> String {
  if label.is_empty() {
    return "─".repeat(width);
  }
  let label_width = label.chars().count().min(width);
  let left = width.saturating_sub(label_width) / 2;
  let right = width.saturating_sub(label_width + left);
  format!("{}{}{}", "─".repeat(left), label, "─".repeat(right))
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

fn truncate_start_chars(value: &str, max_chars: usize) -> String {
  let length = value.chars().count();
  if length <= max_chars {
    return value.to_string();
  }
  if max_chars <= 1 {
    return "…".to_string();
  }
  format!(
    "…{}",
    value
      .chars()
      .skip(length - (max_chars - 1))
      .collect::<String>()
  )
}

fn truncate_end_chars(value: &str, max_chars: usize) -> String {
  if value.chars().count() <= max_chars {
    return value.to_string();
  }
  if max_chars <= 1 {
    return "…".to_string();
  }
  let mut result = value.chars().take(max_chars - 1).collect::<String>();
  result.push('…');
  result
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
