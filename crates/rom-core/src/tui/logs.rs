use std::borrow::Cow;

use super::screen::{
  Attribute,
  Color,
  Line,
  Screen,
  Span,
  Style,
  terminal_text_height,
};

const MAX_RENDERED_LOG_LINE_CHARS: usize = 2_000;

pub(super) fn render_streamed_log(width: u16, text: &str) -> Screen {
  if width == 0 {
    return Screen::new(0, 0);
  }
  let lines = parse_ansi_text(text);
  let height = terminal_text_height(&lines, width).max(1);
  let mut screen = Screen::new(width, height);
  screen.draw_terminal_text(&lines);
  screen
}

pub(super) fn render_retained_log_tail(
  width: u16,
  height: u16,
  logs: &[String],
  excluded_tail: usize,
) -> Screen {
  let mut screen = Screen::new(width, height);
  if width == 0 || height == 0 {
    return screen;
  }

  let end = logs.len().saturating_sub(excluded_tail);
  let mut start = end.saturating_sub(usize::from(height));
  let lines = loop {
    let lines = logs[start..end]
      .iter()
      .flat_map(|line| parse_ansi_line(line))
      .collect::<Vec<_>>();
    if terminal_text_height(&lines, width) <= height
      || start.saturating_add(1) >= end
    {
      break lines;
    }
    start = start.saturating_add(1);
  };
  let rendered_height = terminal_text_height(&lines, width).min(height);
  screen.draw_text(
    0,
    height.saturating_sub(rendered_height),
    width,
    rendered_height,
    &lines,
    true,
  );
  screen
}

fn parse_ansi_line(line: &str) -> Vec<Line> {
  parse_ansi_text(bounded_log_line(line).as_ref())
}

fn parse_ansi_text(text: &str) -> Vec<Line> {
  let bytes = text.as_bytes();
  let mut lines = Vec::new();
  let mut spans = Vec::new();
  let mut style = Style::default();
  let mut start = 0;
  let mut index = 0;
  let mut ended_with_newline = false;

  while index < bytes.len() {
    match bytes[index] {
      b'\x1b' => {
        push_text_span(&mut spans, &text[start..index], style);
        let next = skip_escape_sequence(text, index, &mut style);
        index = next.max(index + 1);
        start = index;
        ended_with_newline = false;
      },
      b'\n' | b'\r' => {
        push_text_span(&mut spans, &text[start..index], style);
        lines.push(Line::from_spans(std::mem::take(&mut spans)));
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
          index += 1;
        }
        index += 1;
        start = index;
        ended_with_newline = true;
      },
      _ => {
        let character = text[index..]
          .chars()
          .next()
          .expect("index remains on a UTF-8 boundary");
        index += character.len_utf8();
        ended_with_newline = false;
      },
    }
  }

  push_text_span(&mut spans, &text[start..], style);
  if !ended_with_newline || lines.is_empty() {
    lines.push(Line::from_spans(spans));
  }
  lines
}

fn push_text_span(spans: &mut Vec<Span>, text: &str, style: Style) {
  if text.is_empty() {
    return;
  }
  if let Some(last) = spans.last_mut()
    && last.style == style
  {
    last.content.push_str(text);
  } else {
    spans.push(Span::styled(text, style));
  }
}

fn skip_escape_sequence(
  text: &str,
  escape_index: usize,
  style: &mut Style,
) -> usize {
  let bytes = text.as_bytes();
  match bytes.get(escape_index + 1) {
    Some(b'[') => {
      let parameter_start = escape_index + 2;
      let mut end = parameter_start;
      while let Some(byte) = bytes.get(end) {
        if (b'@'..=b'~').contains(byte) {
          if *byte == b'm' {
            apply_sgr(&text[parameter_start..end], style);
          }
          return end + 1;
        }
        end += 1;
      }
      bytes.len()
    },
    Some(b']') => {
      let mut end = escape_index + 2;
      while end < bytes.len() {
        if bytes[end] == b'\x07' {
          return end + 1;
        }
        if bytes[end] == b'\x1b' && bytes.get(end + 1) == Some(&b'\\') {
          return end + 2;
        }
        end += 1;
      }
      bytes.len()
    },
    Some(_) => (escape_index + 2).min(bytes.len()),
    None => bytes.len(),
  }
}

fn apply_sgr(parameters: &str, style: &mut Style) {
  if parameters.is_empty() {
    *style = Style::default();
    return;
  }

  let parts = parameters.split(';').collect::<Vec<_>>();
  let mut index = 0;
  while index < parts.len() {
    if parts[index].contains(':') {
      apply_colon_sgr(parts[index], style);
      index += 1;
      continue;
    }
    let code = parts[index].parse::<u16>().unwrap_or(0);
    if matches!(code, 38 | 48 | 58) {
      let (color, consumed) = extended_color(&parts[index + 1..]);
      if let Some(color) = color {
        set_extended_color(style, code, color);
      }
      index += consumed + 1;
      continue;
    }
    apply_sgr_code(code, style);
    index += 1;
  }
}

fn apply_colon_sgr(parameter: &str, style: &mut Style) {
  let values = parameter.split(':').collect::<Vec<_>>();
  let Some(code) = values.first().and_then(|value| value.parse::<u16>().ok())
  else {
    return;
  };
  if !matches!(code, 38 | 48 | 58) {
    apply_sgr_code(code, style);
    return;
  }
  let mode = values.get(1).and_then(|value| value.parse::<u8>().ok());
  let color = match mode {
    Some(5) => {
      values
        .get(2)
        .and_then(|value| value.parse::<u8>().ok())
        .map(Color::AnsiValue)
    },
    Some(2) => {
      let components = values[2..]
        .iter()
        .filter(|value| !value.is_empty())
        .filter_map(|value| value.parse::<u8>().ok())
        .collect::<Vec<_>>();
      (components.len() >= 3).then(|| {
        Color::Rgb {
          r: components[components.len() - 3],
          g: components[components.len() - 2],
          b: components[components.len() - 1],
        }
      })
    },
    _ => None,
  };
  if let Some(color) = color {
    set_extended_color(style, code, color);
  }
}

fn extended_color(parts: &[&str]) -> (Option<Color>, usize) {
  match parts.first().and_then(|value| value.parse::<u8>().ok()) {
    Some(5) => {
      (
        parts
          .get(1)
          .and_then(|value| value.parse::<u8>().ok())
          .map(Color::AnsiValue),
        parts.len().min(2),
      )
    },
    Some(2) => {
      let color = match (parts.get(1), parts.get(2), parts.get(3)) {
        (Some(r), Some(g), Some(b)) => {
          match (r.parse::<u8>(), g.parse::<u8>(), b.parse::<u8>()) {
            (Ok(r), Ok(g), Ok(b)) => Some(Color::Rgb { r, g, b }),
            _ => None,
          }
        },
        _ => None,
      };
      (color, parts.len().min(4))
    },
    _ => (None, parts.len().min(1)),
  }
}

fn set_extended_color(style: &mut Style, code: u16, color: Color) {
  match code {
    38 => style.foreground = Some(color),
    48 => style.background = Some(color),
    58 => style.underline_color = Some(color),
    _ => {},
  }
}

fn apply_sgr_code(code: u16, style: &mut Style) {
  match code {
    0 => *style = Style::default(),
    1 => style.attributes.set(Attribute::Bold),
    2 => style.attributes.set(Attribute::Dim),
    3 => style.attributes.set(Attribute::Italic),
    4 => style.attributes.set(Attribute::Underlined),
    5 => style.attributes.set(Attribute::SlowBlink),
    6 => style.attributes.set(Attribute::RapidBlink),
    7 => style.attributes.set(Attribute::Reverse),
    8 => style.attributes.set(Attribute::Hidden),
    9 => style.attributes.set(Attribute::CrossedOut),
    20 => style.attributes.set(Attribute::Fraktur),
    21 => style.attributes.unset(Attribute::Bold),
    22 => {
      style.attributes.unset(Attribute::Bold);
      style.attributes.unset(Attribute::Dim);
    },
    23 => {
      style.attributes.unset(Attribute::Italic);
      style.attributes.unset(Attribute::Fraktur);
    },
    24 => {
      for attribute in [
        Attribute::Underlined,
        Attribute::DoubleUnderlined,
        Attribute::Undercurled,
        Attribute::Underdotted,
        Attribute::Underdashed,
      ] {
        style.attributes.unset(attribute);
      }
    },
    25 => {
      style.attributes.unset(Attribute::SlowBlink);
      style.attributes.unset(Attribute::RapidBlink);
    },
    27 => style.attributes.unset(Attribute::Reverse),
    28 => style.attributes.unset(Attribute::Hidden),
    29 => style.attributes.unset(Attribute::CrossedOut),
    30..=37 => style.foreground = Some(ansi_color((code - 30) as u8, false)),
    39 => style.foreground = None,
    40..=47 => style.background = Some(ansi_color((code - 40) as u8, false)),
    49 => style.background = None,
    51 => style.attributes.set(Attribute::Framed),
    52 => style.attributes.set(Attribute::Encircled),
    53 => style.attributes.set(Attribute::OverLined),
    54 => {
      style.attributes.unset(Attribute::Framed);
      style.attributes.unset(Attribute::Encircled);
    },
    55 => style.attributes.unset(Attribute::OverLined),
    59 => style.underline_color = None,
    90..=97 => style.foreground = Some(ansi_color((code - 90) as u8, true)),
    100..=107 => style.background = Some(ansi_color((code - 100) as u8, true)),
    _ => {},
  }
}

fn ansi_color(index: u8, bright: bool) -> Color {
  match (index, bright) {
    (0, false) => Color::Black,
    (1, false) => Color::DarkRed,
    (2, false) => Color::DarkGreen,
    (3, false) => Color::DarkYellow,
    (4, false) => Color::DarkBlue,
    (5, false) => Color::DarkMagenta,
    (6, false) => Color::DarkCyan,
    (7, false) => Color::Grey,
    (0, true) => Color::DarkGrey,
    (1, true) => Color::Red,
    (2, true) => Color::Green,
    (3, true) => Color::Yellow,
    (4, true) => Color::Blue,
    (5, true) => Color::Magenta,
    (6, true) => Color::Cyan,
    (7, true) => Color::White,
    _ => Color::Reset,
  }
}

fn bounded_log_line(line: &str) -> Cow<'_, str> {
  if line.chars().count() <= MAX_RENDERED_LOG_LINE_CHARS {
    return Cow::Borrowed(line);
  }

  let mut truncated = line
    .chars()
    .take(MAX_RENDERED_LOG_LINE_CHARS)
    .collect::<String>();
  truncated.push('…');
  Cow::Owned(truncated)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_basic_indexed_and_rgb_sgr_styles() {
    let lines = parse_ansi_text(
      "\x1b[35;1mbold\x1b[0m \x1b[38;5;42mindexed\x1b[48;2;1;2;3mrgb",
    );

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].spans[0].style.foreground, Some(Color::DarkMagenta));
    assert!(lines[0].spans[0].style.attributes.has(Attribute::Bold));
    assert!(
      lines[0]
        .spans
        .iter()
        .any(|span| span.style.foreground == Some(Color::AnsiValue(42)))
    );
    assert!(lines[0].spans.iter().any(|span| {
      span.style.background == Some(Color::Rgb { r: 1, g: 2, b: 3 })
    }));
  }

  #[test]
  fn retained_tail_repaints_logs_above_resized_graph_without_duplicates() {
    let logs = vec![
      "old".to_string(),
      "recent".to_string(),
      "pending".to_string(),
    ];

    let screen = render_retained_log_tail(12, 3, &logs, 1);

    assert_eq!(screen.row_text(0).as_deref(), Some("            "));
    assert_eq!(screen.row_text(1).as_deref(), Some("old         "));
    assert_eq!(screen.row_text(2).as_deref(), Some("recent      "));
    assert!(!screen.plain_text().contains("pending"));
  }

  #[test]
  fn streamed_logs_wrap_by_unicode_width_and_discard_cursor_controls() {
    let screen = render_streamed_log(4, "界界x\x1b[2J");

    assert_eq!(screen.height(), 2);
    assert_eq!(screen.row_text(0).as_deref(), Some("界界"));
    assert_eq!(screen.row_text(1).as_deref(), Some("x   "));
    assert!(!screen.plain_text().contains("2J"));
  }
}
