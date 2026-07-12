use std::{
  collections::VecDeque,
  fmt,
  io::{self, Write},
  ops::Index,
};

pub use crossterm::style::{Attribute, Attributes, Color, ContentStyle};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Terminal styling attached to a screen cell.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Style {
  pub foreground:      Option<Color>,
  pub background:      Option<Color>,
  pub underline_color: Option<Color>,
  pub attributes:      Attributes,
}

impl Style {
  #[must_use]
  pub fn fg(mut self, color: Color) -> Self {
    self.foreground = color_option(color);
    self
  }

  #[must_use]
  pub fn bg(mut self, color: Color) -> Self {
    self.background = color_option(color);
    self
  }

  #[must_use]
  pub fn underline_color(mut self, color: Color) -> Self {
    self.underline_color = color_option(color);
    self
  }

  #[must_use]
  pub fn add_attribute(mut self, attribute: Attribute) -> Self {
    self.attributes.set(attribute);
    self
  }

  #[must_use]
  pub fn remove_attribute(mut self, attribute: Attribute) -> Self {
    self.attributes.unset(attribute);
    self
  }

  #[must_use]
  pub fn patch(mut self, other: Self) -> Self {
    if other.foreground.is_some() {
      self.foreground = other.foreground;
    }
    if other.background.is_some() {
      self.background = other.background;
    }
    if other.underline_color.is_some() {
      self.underline_color = other.underline_color;
    }
    self.attributes.extend(other.attributes);
    self
  }

  #[must_use]
  pub const fn content_style(self) -> ContentStyle {
    ContentStyle {
      foreground_color: self.foreground,
      background_color: self.background,
      underline_color:  self.underline_color,
      attributes:       self.attributes,
    }
  }
}

fn color_option(color: Color) -> Option<Color> {
  if color == Color::Reset {
    None
  } else {
    Some(color)
  }
}

/// One terminal cell in a rendered [`Screen`].
///
/// A zero `width` marks a continuation cell occupied by the preceding wide
/// grapheme. Consumers must not print continuation cells independently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenCell {
  pub symbol: String,
  pub style:  Style,
  pub width:  u16,
}

impl Default for ScreenCell {
  fn default() -> Self {
    Self {
      symbol: " ".to_string(),
      style:  Style::default(),
      width:  1,
    }
  }
}

impl ScreenCell {
  #[must_use]
  pub const fn is_continuation(&self) -> bool {
    self.width == 0
  }
}

/// A fixed-size styled terminal framebuffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Screen {
  width:  u16,
  height: u16,
  cells:  Vec<ScreenCell>,
}

impl Screen {
  #[must_use]
  pub fn new(width: u16, height: u16) -> Self {
    Self {
      width,
      height,
      cells: vec![
        ScreenCell::default();
        usize::from(width) * usize::from(height)
      ],
    }
  }

  #[must_use]
  pub const fn width(&self) -> u16 {
    self.width
  }

  #[must_use]
  pub const fn height(&self) -> u16 {
    self.height
  }

  #[must_use]
  pub const fn dimensions(&self) -> (u16, u16) {
    (self.width, self.height)
  }

  #[must_use]
  pub fn cells(&self) -> &[ScreenCell] {
    &self.cells
  }

  #[must_use]
  pub fn row(&self, y: u16) -> Option<&[ScreenCell]> {
    if y >= self.height {
      return None;
    }
    let start = usize::from(y) * usize::from(self.width);
    Some(&self.cells[start..start + usize::from(self.width)])
  }

  #[must_use]
  pub fn cell(&self, x: u16, y: u16) -> Option<&ScreenCell> {
    self.index_of(x, y).map(|index| &self.cells[index])
  }

  #[must_use]
  pub fn row_text(&self, y: u16) -> Option<String> {
    self.row(y).map(|row| {
      row
        .iter()
        .filter(|cell| !cell.is_continuation())
        .map(|cell| cell.symbol.as_str())
        .collect()
    })
  }

  #[must_use]
  pub fn plain_text(&self) -> String {
    (0..self.height)
      .filter_map(|y| self.row_text(y))
      .collect::<Vec<_>>()
      .join("\n")
  }

  /// Write one complete row with ANSI SGR styling and no trailing newline.
  pub fn write_ansi_row<W: Write>(
    &self,
    y: u16,
    writer: &mut W,
  ) -> io::Result<()> {
    let Some(row) = self.row(y) else {
      return Ok(());
    };
    let mut active = Style::default();

    for cell in row.iter().filter(|cell| !cell.is_continuation()) {
      if cell.style != active {
        writer.write_all(b"\x1b[0m")?;
        write_style(writer, cell.style)?;
        active = cell.style;
      }
      writer.write_all(cell.symbol.as_bytes())?;
    }

    if active != Style::default() {
      writer.write_all(b"\x1b[0m")?;
    }
    Ok(())
  }

  /// Write a row without the framebuffer's trailing default cells.
  pub fn write_ansi_row_trimmed<W: Write>(
    &self,
    y: u16,
    writer: &mut W,
  ) -> io::Result<()> {
    let Some(row) = self.row(y) else {
      return Ok(());
    };
    let end = row
      .iter()
      .rposition(|cell| {
        !cell.is_continuation()
          && (cell.symbol != " " || cell.style != Style::default())
      })
      .map_or(0, |index| index.saturating_add(1));
    let mut active = Style::default();
    for cell in row[..end].iter().filter(|cell| !cell.is_continuation()) {
      if cell.style != active {
        writer.write_all(b"\x1b[0m")?;
        write_style(writer, cell.style)?;
        active = cell.style;
      }
      writer.write_all(cell.symbol.as_bytes())?;
    }
    if active != Style::default() {
      writer.write_all(b"\x1b[0m")?;
    }
    Ok(())
  }

  /// Serialize the complete screen as ANSI-styled rows separated by CRLF.
  pub fn write_ansi<W: Write>(&self, writer: &mut W) -> io::Result<()> {
    for y in 0..self.height {
      if y > 0 {
        writer.write_all(b"\r\n")?;
      }
      self.write_ansi_row(y, writer)?;
    }
    Ok(())
  }

  pub(super) fn draw_text(
    &mut self,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    lines: &[Line],
    wrap: bool,
  ) {
    if width == 0 || height == 0 {
      return;
    }

    let mut target_y = y;
    let end_y = y.saturating_add(height).min(self.height);
    for line in lines {
      let graphemes = styled_graphemes(line);
      let rendered_lines = if wrap {
        wrap_graphemes(graphemes, width)
      } else {
        vec![truncate_graphemes(graphemes, width)]
      };
      for rendered in rendered_lines {
        if target_y >= end_y {
          return;
        }
        self.draw_graphemes(x, target_y, width, &rendered);
        target_y = target_y.saturating_add(1);
      }
    }
  }

  pub(super) fn draw_terminal_text(&mut self, lines: &[Line]) {
    let mut y = 0;
    for line in lines {
      for row in hard_wrap_graphemes(styled_graphemes(line), self.width) {
        if y >= self.height {
          return;
        }
        self.draw_graphemes(0, y, self.width, &row);
        y = y.saturating_add(1);
      }
    }
  }

  fn draw_graphemes(
    &mut self,
    x: u16,
    y: u16,
    width: u16,
    graphemes: &[StyledGrapheme],
  ) {
    let mut target_x = x;
    let end_x = x.saturating_add(width).min(self.width);
    for grapheme in graphemes {
      if grapheme.width == 0 {
        if target_x > x
          && let Some(cell) = self.cell_mut(target_x - 1, y)
          && !cell.is_continuation()
        {
          cell.symbol.push_str(&grapheme.symbol);
        }
        continue;
      }
      if target_x.saturating_add(grapheme.width) > end_x {
        break;
      }
      self.put_grapheme(
        target_x,
        y,
        &grapheme.symbol,
        grapheme.width,
        grapheme.style,
      );
      target_x = target_x.saturating_add(grapheme.width);
    }
  }

  fn put_grapheme(
    &mut self,
    x: u16,
    y: u16,
    symbol: &str,
    width: u16,
    style: Style,
  ) {
    if width == 0 || x >= self.width || y >= self.height {
      return;
    }
    let Some(index) = self.index_of(x, y) else {
      return;
    };
    self.cells[index] = ScreenCell {
      symbol: symbol.to_string(),
      style,
      width,
    };
    for offset in 1..width {
      let continuation_x = x.saturating_add(offset);
      let Some(index) = self.index_of(continuation_x, y) else {
        break;
      };
      self.cells[index] = ScreenCell {
        symbol: String::new(),
        style,
        width: 0,
      };
    }
  }

  fn cell_mut(&mut self, x: u16, y: u16) -> Option<&mut ScreenCell> {
    self.index_of(x, y).map(|index| &mut self.cells[index])
  }

  fn index_of(&self, x: u16, y: u16) -> Option<usize> {
    if x >= self.width || y >= self.height {
      return None;
    }
    Some(usize::from(y) * usize::from(self.width) + usize::from(x))
  }
}

impl Index<(u16, u16)> for Screen {
  type Output = ScreenCell;

  fn index(&self, (x, y): (u16, u16)) -> &Self::Output {
    self
      .cell(x, y)
      .expect("screen coordinates must be in bounds")
  }
}

impl fmt::Display for Screen {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(&self.plain_text())
  }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Line {
  pub(super) spans: Vec<Span>,
}

impl Line {
  pub(super) fn from_spans(spans: Vec<Span>) -> Self {
    Self { spans }
  }
}

impl From<&str> for Line {
  fn from(value: &str) -> Self {
    Self::from(Span::raw(value))
  }
}

impl From<String> for Line {
  fn from(value: String) -> Self {
    Self::from(Span::raw(value))
  }
}

impl From<Span> for Line {
  fn from(span: Span) -> Self {
    Self { spans: vec![span] }
  }
}

impl From<Vec<Span>> for Line {
  fn from(spans: Vec<Span>) -> Self {
    Self::from_spans(spans)
  }
}

#[derive(Clone, Debug)]
pub(super) struct Span {
  pub(super) content: String,
  pub(super) style:   Style,
}

impl Span {
  pub(super) fn raw(content: impl Into<String>) -> Self {
    Self {
      content: content.into(),
      style:   Style::default(),
    }
  }

  pub(super) fn styled(content: impl Into<String>, style: Style) -> Self {
    Self {
      content: content.into(),
      style,
    }
  }
}

#[derive(Clone, Debug)]
struct StyledGrapheme {
  symbol: String,
  style:  Style,
  width:  u16,
}

impl StyledGrapheme {
  fn is_whitespace(&self) -> bool {
    self.symbol.chars().all(char::is_whitespace)
  }
}

pub(super) fn terminal_text_height(lines: &[Line], width: u16) -> u16 {
  lines
    .iter()
    .map(|line| hard_wrap_graphemes(styled_graphemes(line), width).len())
    .sum::<usize>()
    .min(usize::from(u16::MAX)) as u16
}

fn styled_graphemes(line: &Line) -> Vec<StyledGrapheme> {
  line
    .spans
    .iter()
    .flat_map(|span| {
      span.content.graphemes(true).map(|symbol| {
        StyledGrapheme {
          symbol: symbol.to_string(),
          style:  span.style,
          width:  UnicodeWidthStr::width(symbol) as u16,
        }
      })
    })
    .collect()
}

fn hard_wrap_graphemes(
  graphemes: Vec<StyledGrapheme>,
  max_width: u16,
) -> Vec<Vec<StyledGrapheme>> {
  if max_width == 0 {
    return Vec::new();
  }
  let mut rows = Vec::new();
  let mut row = Vec::new();
  let mut width = 0_u16;
  for grapheme in graphemes {
    if grapheme.width > max_width {
      continue;
    }
    if grapheme.width > 0 && width.saturating_add(grapheme.width) > max_width {
      rows.push(std::mem::take(&mut row));
      width = 0;
    }
    width = width.saturating_add(grapheme.width);
    row.push(grapheme);
  }
  rows.push(row);
  rows
}

fn truncate_graphemes(
  graphemes: Vec<StyledGrapheme>,
  max_width: u16,
) -> Vec<StyledGrapheme> {
  let mut width = 0_u16;
  graphemes
    .into_iter()
    .filter(|grapheme| grapheme.width <= max_width)
    .take_while(|grapheme| {
      let fits = width.saturating_add(grapheme.width) <= max_width;
      if fits {
        width = width.saturating_add(grapheme.width);
      }
      fits
    })
    .collect()
}

fn wrap_graphemes(
  graphemes: Vec<StyledGrapheme>,
  max_width: u16,
) -> Vec<Vec<StyledGrapheme>> {
  if max_width == 0 {
    return Vec::new();
  }

  let mut wrapped = Vec::new();
  let mut line = Vec::new();
  let mut pending_word = Vec::new();
  let mut pending_whitespace: VecDeque<StyledGrapheme> = VecDeque::new();
  let mut line_width = 0_u16;
  let mut word_width = 0_u16;
  let mut whitespace_width = 0_u16;
  let mut previous_was_non_whitespace = false;

  for grapheme in graphemes {
    if grapheme.width > max_width {
      continue;
    }
    let is_whitespace = grapheme.is_whitespace();
    let word_finished = previous_was_non_whitespace && is_whitespace;
    let segment_overflows = line.is_empty()
      && word_width
        .saturating_add(whitespace_width)
        .saturating_add(grapheme.width)
        > max_width;

    if word_finished || segment_overflows {
      line.extend(pending_whitespace.drain(..));
      line_width = line_width.saturating_add(whitespace_width);
      line.append(&mut pending_word);
      line_width = line_width.saturating_add(word_width);
      whitespace_width = 0;
      word_width = 0;
    }

    let line_full = line_width >= max_width;
    let pending_overflows = grapheme.width > 0
      && line_width
        .saturating_add(whitespace_width)
        .saturating_add(word_width)
        >= max_width;
    if line_full || pending_overflows {
      let mut remaining = max_width.saturating_sub(line_width);
      wrapped.push(std::mem::take(&mut line));
      line_width = 0;
      while let Some(space) = pending_whitespace.front() {
        if space.width > remaining {
          break;
        }
        whitespace_width = whitespace_width.saturating_sub(space.width);
        remaining = remaining.saturating_sub(space.width);
        pending_whitespace.pop_front();
      }
      if is_whitespace && pending_whitespace.is_empty() {
        previous_was_non_whitespace = false;
        continue;
      }
    }

    if is_whitespace {
      whitespace_width = whitespace_width.saturating_add(grapheme.width);
      pending_whitespace.push_back(grapheme);
    } else {
      word_width = word_width.saturating_add(grapheme.width);
      pending_word.push(grapheme);
    }
    previous_was_non_whitespace = !is_whitespace;
  }

  line.extend(pending_whitespace);
  line.append(&mut pending_word);
  if !line.is_empty() {
    wrapped.push(line);
  }
  if wrapped.is_empty() {
    wrapped.push(Vec::new());
  }
  wrapped
}

fn write_style<W: Write>(writer: &mut W, style: Style) -> io::Result<()> {
  for attribute in Attribute::iterator() {
    if attribute != Attribute::Reset && style.attributes.has(attribute) {
      write!(writer, "\x1b[{}m", attribute.sgr())?;
    }
  }
  if let Some(color) = style.foreground {
    write_color(writer, 38, color)?;
  }
  if let Some(color) = style.background {
    write_color(writer, 48, color)?;
  }
  if let Some(color) = style.underline_color {
    write_color(writer, 58, color)?;
  }
  Ok(())
}

fn write_color<W: Write>(
  writer: &mut W,
  extended_prefix: u8,
  color: Color,
) -> io::Result<()> {
  if extended_prefix == 58 {
    return match color {
      Color::Reset => writer.write_all(b"\x1b[59m"),
      Color::Rgb { r, g, b } => {
        write!(writer, "\x1b[58;2;{r};{g};{b}m")
      },
      Color::AnsiValue(value) => write!(writer, "\x1b[58;5;{value}m"),
      color => write!(writer, "\x1b[58;5;{}m", base_color_index(color)),
    };
  }

  let base = if extended_prefix == 48 { 40 } else { 30 };
  let bright_base = if extended_prefix == 48 { 100 } else { 90 };
  let code = match color {
    Color::Reset => {
      return write!(
        writer,
        "\x1b[{}m",
        if extended_prefix == 48 { 49 } else { 39 }
      );
    },
    Color::Black => Some(base),
    Color::DarkRed => Some(base + 1),
    Color::DarkGreen => Some(base + 2),
    Color::DarkYellow => Some(base + 3),
    Color::DarkBlue => Some(base + 4),
    Color::DarkMagenta => Some(base + 5),
    Color::DarkCyan => Some(base + 6),
    Color::Grey => Some(base + 7),
    Color::DarkGrey => Some(bright_base),
    Color::Red => Some(bright_base + 1),
    Color::Green => Some(bright_base + 2),
    Color::Yellow => Some(bright_base + 3),
    Color::Blue => Some(bright_base + 4),
    Color::Magenta => Some(bright_base + 5),
    Color::Cyan => Some(bright_base + 6),
    Color::White => Some(bright_base + 7),
    Color::AnsiValue(value) => {
      return write!(writer, "\x1b[{extended_prefix};5;{value}m");
    },
    Color::Rgb { r, g, b } => {
      return write!(writer, "\x1b[{extended_prefix};2;{r};{g};{b}m");
    },
  };
  write!(
    writer,
    "\x1b[{}m",
    code.expect("base ANSI color has a code")
  )
}

fn base_color_index(color: Color) -> u8 {
  match color {
    Color::Black => 0,
    Color::DarkRed => 1,
    Color::DarkGreen => 2,
    Color::DarkYellow => 3,
    Color::DarkBlue => 4,
    Color::DarkMagenta => 5,
    Color::DarkCyan => 6,
    Color::Grey => 7,
    Color::DarkGrey => 8,
    Color::Red => 9,
    Color::Green => 10,
    Color::Yellow => 11,
    Color::Blue => 12,
    Color::Magenta => 13,
    Color::Cyan => 14,
    Color::White => 15,
    Color::Reset | Color::Rgb { .. } | Color::AnsiValue(_) => 0,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn wide_graphemes_occupy_continuation_cells() {
    let mut screen = Screen::new(4, 1);
    screen.draw_text(0, 0, 4, 1, &[Line::from("界x")], false);

    assert_eq!(screen[(0, 0)].symbol, "界");
    assert_eq!(screen[(0, 0)].width, 2);
    assert!(screen[(1, 0)].is_continuation());
    assert_eq!(screen.row_text(0).as_deref(), Some("界x "));
  }

  #[test]
  fn ansi_rows_include_cell_styles() {
    let mut screen = Screen::new(2, 1);
    screen.draw_text(
      0,
      0,
      2,
      1,
      &[Line::from(Span::styled(
        "x",
        Style::default()
          .fg(Color::DarkRed)
          .add_attribute(Attribute::Bold),
      ))],
      false,
    );
    let mut output = Vec::new();
    screen.write_ansi_row(0, &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();

    assert!(output.contains("\x1b[1m"));
    assert!(output.contains("\x1b[31m"));
    assert!(output.contains('x'));
    assert!(output.ends_with("\x1b[0m "));
  }
}
