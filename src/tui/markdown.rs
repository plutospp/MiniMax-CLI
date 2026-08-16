//! Markdown-to-styled-rows renderer for assistant messages.
//!
//! Parses the markdown constructs models actually emit (headings, bold,
//! italic, inline code, links, bullet/ordered lists, blockquotes, rules) and
//! returns word-wrapped rows of styled spans ready for the transcript.
//! Fenced code blocks are split out earlier by `parse_message_segments` and
//! keep their dedicated syntax-highlighted path.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::palette;

/// One rendered, word-wrapped line: a sequence of styled spans.
pub type MdRow = Vec<Span<'static>>;

/// Render markdown text into wrapped, styled rows at the given width.
#[must_use]
pub fn markdown_rows(text: &str, base: Style, width: usize) -> Vec<MdRow> {
    let mut renderer = Renderer::new(base, width.max(4));
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    for event in Parser::new_ext(text, options) {
        renderer.push(event);
    }
    renderer.finish()
}

#[derive(Debug, Clone, Copy)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
}

impl InlineStyle {
    const PLAIN: Self = Self {
        bold: false,
        italic: false,
        strike: false,
        code: false,
        link: false,
    };

    fn apply(self, base: Style) -> Style {
        let mut style = if self.code {
            Style::default().fg(palette::MINIMAX_GREEN)
        } else {
            base
        };
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strike {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link {
            style = style
                .fg(palette::MINIMAX_BLUE)
                .add_modifier(Modifier::UNDERLINED);
        }
        style
    }
}

/// Accumulates text runs into a paragraph before wrapping.
#[derive(Default)]
struct Paragraph {
    spans: Vec<Span<'static>>,
}

impl Paragraph {
    fn push(&mut self, text: &str, style: Style) {
        if !text.is_empty() {
            self.spans.push(Span::styled(text.to_string(), style));
        }
    }

    fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn take(&mut self) -> Vec<Span<'static>> {
        std::mem::take(&mut self.spans)
    }
}

struct Renderer {
    base: Style,
    width: usize,
    rows: Vec<MdRow>,
    paragraph: Paragraph,
    /// Stack of active inline styles (innermost last).
    inline: Vec<InlineStyle>,
    /// Current block context.
    in_heading: Option<HeadingLevel>,
    in_quote: bool,
    list_stack: Vec<Option<u64>>, // Some(n) = ordered list, next item number
    item_pending: bool,
    /// In-progress markdown table, if any.
    table: Option<TableBuild>,
}

/// Accumulates rows while a markdown table is being parsed.
#[derive(Default)]
struct TableBuild {
    alignments: Vec<pulldown_cmark::Alignment>,
    /// Completed rows; row 0 is the header.
    rows: Vec<Vec<Vec<Span<'static>>>>,
    /// Cells of the row currently being parsed.
    current_row: Vec<Vec<Span<'static>>>,
    /// Spans of the cell currently being parsed.
    current_cell: Paragraph,
}

impl Renderer {
    fn new(base: Style, width: usize) -> Self {
        Self {
            base,
            width,
            rows: Vec::new(),
            paragraph: Paragraph::default(),
            inline: vec![InlineStyle::PLAIN],
            in_heading: None,
            in_quote: false,
            list_stack: Vec::new(),
            item_pending: false,
            table: None,
        }
    }

    fn current_style(&self) -> InlineStyle {
        *self.inline.last().unwrap_or(&InlineStyle::PLAIN)
    }

    /// Route inline text to the open table cell, or the paragraph otherwise.
    fn push_inline(&mut self, text: &str, style: Style) {
        if let Some(table) = self.table.as_mut() {
            table.current_cell.push(text, style);
        } else {
            self.paragraph.push(text, style);
        }
    }

    fn compose_prefix(&self) -> String {
        let mut prefix = String::new();
        if self.in_quote {
            prefix.push_str("> ");
        }
        let depth = self.list_stack.len().saturating_sub(1);
        if depth > 0 {
            prefix.push_str(&"  ".repeat(depth));
        }
        if let Some(list) = self.list_stack.last() {
            match list {
                Some(n) => prefix.push_str(&format!("{n}. ")),
                None => prefix.push_str("• "),
            }
        }
        prefix
    }

    fn flush_paragraph(&mut self) {
        if self.paragraph.is_empty() {
            return;
        }
        let spans = self.paragraph.take();
        let prefix = self.compose_prefix();
        self.item_pending = false;
        // Continuation lines hang to align with the item text.
        let hang = " ".repeat(UnicodeWidthStr::width(prefix.as_str()));
        for (index, row) in wrap_spans(&spans, self.width.saturating_sub(hang.width()))
            .into_iter()
            .enumerate()
        {
            let mut line = Vec::with_capacity(row.len() + 2);
            if index == 0 {
                if !prefix.is_empty() {
                    line.push(Span::styled(prefix.clone(), self.base));
                }
            } else if !hang.is_empty() {
                line.push(Span::raw(hang.clone()));
            }
            line.extend(row);
            self.rows.push(line);
        }
    }

    /// Emit a parsed markdown table as aligned, width-fitted rows.
    ///
    /// Columns use natural widths when the table fits; otherwise the widest
    /// columns shrink (floor 3) until it does, wrapping cell text within its
    /// column. The header row is bold with a rule line beneath.
    fn flush_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        if table.rows.is_empty() {
            return;
        }
        let ncols = table
            .alignments
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        if ncols == 0 {
            return;
        }

        let separator_width = 3 * (ncols - 1);
        let available = self.width.saturating_sub(separator_width).max(ncols * 3);

        let mut widths = vec![0usize; ncols];
        for row in &table.rows {
            for (index, cell) in row.iter().enumerate().take(ncols) {
                let cell_width = cell.iter().map(|s| s.content.width()).sum();
                widths[index] = widths[index].max(cell_width);
            }
        }
        while widths.iter().sum::<usize>() > available {
            let Some(widest) = widths
                .iter()
                .enumerate()
                .filter(|&(_, width)| *width > 3)
                .max_by_key(|&(index, width)| (width, std::cmp::Reverse(index)))
                .map(|(index, _)| index)
            else {
                break;
            };
            widths[widest] -= 1;
        }

        let header_style = self
            .base
            .fg(palette::MINIMAX_BLUE)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(palette::TEXT_DIM);

        for (row_index, row) in table.rows.iter().enumerate() {
            // Wrap each cell into lines within its column width.
            let wrapped: Vec<Vec<MdRow>> = (0..ncols)
                .map(|index| {
                    let cell = row.get(index);
                    if cell.is_none_or(|spans| spans.is_empty()) {
                        return vec![Vec::new()];
                    }
                    wrap_cell_spans(&row[index], widths[index])
                })
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);

            for (line_index, _) in (0..height).enumerate() {
                let mut line = Vec::new();
                for index in 0..ncols {
                    if index > 0 {
                        line.push(Span::styled(" │ ", dim));
                    }
                    let cell_line = wrapped[index].get(line_index);
                    let alignment = table
                        .alignments
                        .get(index)
                        .copied()
                        .unwrap_or(pulldown_cmark::Alignment::None);
                    let padded = pad_cell(cell_line, widths[index], alignment, header_style);
                    if row_index == 0 {
                        line.extend(padded.into_iter().map(|span| {
                            let mut style = span.style;
                            style = style.patch(header_style);
                            Span::styled(span.content, style)
                        }));
                    } else {
                        line.extend(padded);
                    }
                }
                self.rows.push(line);
            }

            // Rule under the header row.
            if row_index == 0 {
                let mut rule = Vec::new();
                for (index, width) in widths.iter().enumerate() {
                    if index > 0 {
                        rule.push(Span::styled("─┼─", dim));
                    }
                    rule.push(Span::styled("─".repeat(*width), dim));
                }
                self.rows.push(rule);
            }
        }
        self.blank();
    }

    fn blank(&mut self) {
        if self.rows.last().is_some_and(|last| !last.is_empty()) {
            self.rows.push(Vec::new());
        }
    }

    fn heading_style(&self, level: HeadingLevel) -> Style {
        match level {
            HeadingLevel::H1 => self
                .base
                .fg(palette::MINIMAX_ORANGE)
                .add_modifier(Modifier::BOLD),
            HeadingLevel::H2 => self
                .base
                .fg(palette::MINIMAX_BLUE)
                .add_modifier(Modifier::BOLD),
            _ => self
                .base
                .fg(palette::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        }
    }

    fn flush_heading(&mut self) {
        let Some(level) = self.in_heading.take() else {
            return;
        };
        let style = self.heading_style(level);
        let spans = self.paragraph.take();
        if spans.is_empty() {
            return;
        }
        self.blank();
        // Headings never wrap; content is short by construction.
        self.rows.push(
            spans
                .iter()
                .map(|span| Span::styled(span.content.to_string(), style))
                .collect(),
        );
        self.blank();
    }

    fn push(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => {
                let style = self.current_style().apply(self.base);
                self.push_inline(&text, style);
            }
            Event::Code(code) => {
                let style = InlineStyle {
                    code: true,
                    ..self.current_style()
                }
                .apply(self.base);
                self.push_inline(&code, style);
            }
            Event::SoftBreak => {
                let style = self.base;
                self.push_inline(" ", style);
            }
            Event::HardBreak => {
                // Split the current paragraph row without ending the block.
                let spans = self.paragraph.take();
                for row in wrap_spans(&spans, self.width) {
                    self.rows.push(row);
                }
            }
            Event::Rule => {
                self.blank();
                let bar = "─".repeat(self.width / 3);
                self.rows.push(vec![Span::styled(
                    bar,
                    Style::default().fg(palette::TEXT_DIM),
                )]);
                self.blank();
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.paragraph.push(marker, self.base);
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_paragraph();
                self.in_heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush_paragraph();
                self.blank();
                self.in_quote = true;
            }
            Tag::List(start) => {
                self.list_stack.push(start);
            }
            Tag::Item => {
                self.flush_paragraph();
                self.item_pending = true;
            }
            Tag::Table(alignments) => {
                self.flush_paragraph();
                self.blank();
                self.table = Some(TableBuild {
                    alignments,
                    ..Default::default()
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row = Vec::new();
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.current_cell = Paragraph::default();
                }
            }
            Tag::Strong => {
                let style = InlineStyle {
                    bold: true,
                    ..self.current_style()
                };
                self.inline.push(style);
            }
            Tag::Emphasis => {
                let style = InlineStyle {
                    italic: true,
                    ..self.current_style()
                };
                self.inline.push(style);
            }
            Tag::Strikethrough => {
                let style = InlineStyle {
                    strike: true,
                    ..self.current_style()
                };
                self.inline.push(style);
            }
            Tag::Link { .. } => {
                let style = InlineStyle {
                    link: true,
                    ..self.current_style()
                };
                self.inline.push(style);
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_paragraph();
                self.blank();
            }
            TagEnd::Heading(_) => self.flush_heading(),
            TagEnd::BlockQuote(_) => {
                self.flush_paragraph();
                self.in_quote = false;
                self.blank();
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    let spans = table.current_cell.take();
                    table.current_row.push(spans);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.current_row);
                    if !row.is_empty() {
                        table.rows.push(row);
                    }
                }
            }
            TagEnd::Table => self.flush_table(),
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush_paragraph();
                if let Some(Some(n)) = self.list_stack.last_mut() {
                    *n += 1;
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<MdRow> {
        self.flush_paragraph();
        while self.rows.last().is_some_and(Vec::is_empty) {
            self.rows.pop();
        }
        self.rows
    }
}

/// Word-wrap styled spans to `width`, preserving per-span styling.
fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<MdRow> {
    let width = width.max(1);
    let mut rows: Vec<MdRow> = Vec::new();
    let mut current: MdRow = Vec::new();
    let mut current_width = 0usize;

    for span in spans {
        let style = span.style;
        for (index, word) in split_keep_trailing_space(&span.content)
            .into_iter()
            .enumerate()
        {
            let word_width = UnicodeWidthStr::width(word.as_str());
            let is_space = word.trim().is_empty();

            if current_width + word_width > width && !current.is_empty() && !is_space {
                rows.push(std::mem::take(&mut current));
                current_width = 0;
                if index == 0 || !is_space {
                    // Trim the leading space of the wrapped line.
                    let trimmed = word.trim_start();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let leading = word.len() - trimmed.len();
                    let text = word[leading..].to_string();
                    let w = UnicodeWidthStr::width(text.as_str());
                    current.push(Span::styled(text, style));
                    current_width = w;
                    continue;
                }
            }

            current_width += word_width;
            current.push(Span::styled(word, style));
        }
    }

    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// Split text into words that keep their trailing whitespace attached.
fn split_keep_trailing_space(text: &str) -> Vec<String> {
    text.split_inclusive(' ')
        .flat_map(|chunk| chunk.split_inclusive('\t'))
        .map(str::to_string)
        .collect()
}

/// Pad a table cell's spans to the column width, honoring column alignment.
fn pad_cell(
    cell_line: Option<&MdRow>,
    width: usize,
    alignment: pulldown_cmark::Alignment,
    base: Style,
) -> MdRow {
    let mut spans: MdRow = cell_line.map(Vec::to_owned).unwrap_or_default();
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let padding = width.saturating_sub(used);
    let (left, right) = match alignment {
        pulldown_cmark::Alignment::Right => (padding, 0),
        pulldown_cmark::Alignment::Center => (padding / 2, padding - padding / 2),
        _ => (0, padding),
    };
    let mut padded = Vec::with_capacity(spans.len() + 2);
    if left > 0 {
        padded.push(Span::styled(" ".repeat(left), base));
    }
    padded.append(&mut spans);
    if right > 0 {
        padded.push(Span::styled(" ".repeat(right), base));
    }
    padded
}

/// Wrap spans to an exact width, splitting words that cannot fit on a line.
///
/// Used for table cells, where breaking a long word is preferable to letting
/// the aligned row exceed the terminal width.
fn wrap_cell_spans(spans: &[Span<'static>], width: usize) -> Vec<MdRow> {
    let width = width.max(1);
    let mut rows: Vec<MdRow> = Vec::new();
    let mut current: MdRow = Vec::new();
    let mut current_width = 0usize;

    /// Split `text` at the char boundary covering `limit` display columns.
    fn split_at_width(text: &str, limit: usize) -> (String, String) {
        use unicode_width::UnicodeWidthChar;
        let mut used = 0usize;
        for (index, ch) in text.char_indices() {
            if used + ch.width().unwrap_or(0) > limit {
                return (text[..index].to_string(), text[index..].to_string());
            }
            used += ch.width().unwrap_or(0);
        }
        (text.to_string(), String::new())
    }

    for span in spans {
        let style = span.style;
        for word in split_keep_trailing_space(&span.content) {
            let mut piece: String = word;
            // Hard-split pieces wider than a full column line.
            while UnicodeWidthStr::width(piece.as_str()) > width {
                // The split head is a full-width chunk; never append it to a
                // row that already has content.
                if !current.is_empty() {
                    rows.push(std::mem::take(&mut current));
                }
                let (head, rest) = split_at_width(&piece, width);
                if head.is_empty() {
                    // First glyph wider than the column (e.g. CJK in a narrow
                    // column): take it unconditionally to guarantee progress.
                    let rest_start = piece
                        .char_indices()
                        .nth(1)
                        .map_or(piece.len(), |(index, _)| index);
                    current.push(Span::styled(piece[..rest_start].to_string(), style));
                    rows.push(std::mem::take(&mut current));
                    current_width = 0;
                    piece = piece[rest_start..].to_string();
                    continue;
                }
                current.push(Span::styled(head, style));
                rows.push(std::mem::take(&mut current));
                current_width = 0;
                piece = rest.trim_start().to_string();
            }
            // Soft-wrap: start a new row rather than splitting a word that
            // fits a full line on its own.
            if current_width + UnicodeWidthStr::width(piece.as_str()) > width && !current.is_empty()
            {
                rows.push(std::mem::take(&mut current));
                current_width = 0;
                piece = piece.trim_start().to_string();
            }
            let piece_width = UnicodeWidthStr::width(piece.as_str());
            if !piece.is_empty() {
                current.push(Span::styled(piece, style));
                current_width += piece_width;
            }
        }
    }

    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}
#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(row: &MdRow) -> String {
        row.iter().map(|s| s.content.as_ref()).collect()
    }

    fn rows_text(rows: &[MdRow]) -> String {
        rows.iter().map(row_text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn headings_render_styled_without_hashes() {
        let rows = markdown_rows("# Title\n\nBody text", Style::default(), 60);
        assert_eq!(rows.len(), 3); // heading, blank, body
        assert_eq!(row_text(&rows[0]), "Title");
        assert_eq!(row_text(&rows[2]), "Body text");
    }

    #[test]
    fn bold_and_inline_code_lose_markdown_syntax() {
        let rows = markdown_rows(
            "Uses **tiny11maker.ps1** and `oscdimg.exe` here",
            Style::default(),
            60,
        );
        let text = rows_text(&rows);
        assert!(text.contains("Uses tiny11maker.ps1 and"));
        assert!(!text.contains("**"));
        assert!(!text.contains('`'));
        // The code span carries the code color
        let code_span = rows[0]
            .iter()
            .find(|s| s.content.contains("oscdimg.exe"))
            .expect("code span present");
        assert_eq!(code_span.style.fg, Some(palette::MINIMAX_GREEN));
        // The bold span carries BOLD
        let bold_span = rows[0]
            .iter()
            .find(|s| s.content.contains("tiny11maker.ps1"))
            .expect("bold span present");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn strikethrough_renders_styled_without_tildes() {
        let rows = markdown_rows("this is ~~removed~~ text", Style::default(), 60);
        let text = rows_text(&rows);
        assert!(text.contains("removed"));
        assert!(!text.contains('~'));
        let strike_span = rows[0]
            .iter()
            .find(|s| s.content.contains("removed"))
            .expect("strike span present");
        assert!(
            strike_span
                .style
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        );
    }

    #[test]
    fn tables_render_aligned_columns_within_width() {
        let md = "| Script | Purpose |\n|---|---|\n| tiny11maker.ps1 | Removes bloat but keeps the image serviceable |\n| core.ps1 | Strips everything |";
        let rows = markdown_rows(md, Style::default(), 40);

        // No raw pipe syntax remains.
        for row in &rows {
            let text = row_text(row);
            assert!(!text.starts_with('|'), "raw table row: {text}");
        }

        // Header present and styled bold.
        let header = rows
            .iter()
            .find(|r| row_text(r).contains("Script"))
            .expect("header row");
        assert!(
            header
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        );
        // Rule line under the header.
        assert!(rows.iter().any(|r| row_text(r).contains('┼')));
        // Cell content preserved.
        let text = rows_text(&rows);
        assert!(text.contains("tiny11maker.ps1"));
        assert!(text.contains("serviceable"));
        // Every row fits the width.
        for row in &rows {
            let width = row.iter().map(|s| s.content.width()).sum::<usize>();
            assert!(width <= 40, "row width {width} > 40: {}", row_text(row));
        }
    }

    #[test]
    fn tables_hard_wrap_unbreakable_words_to_fit_width() {
        let md = "| A | B |\n|---|---|\n| supercalifragilistic | x |";
        let rows = markdown_rows(md, Style::default(), 20);
        for row in &rows {
            let width = row.iter().map(|s| s.content.width()).sum::<usize>();
            assert!(width <= 20, "row width {width} > 20: {}", row_text(row));
        }
        let text = rows_text(&rows);
        assert!(text.contains("supercalifragili"), "head lost: {text}");
        assert!(text.contains("stic"), "tail lost: {text}");
    }

    #[test]
    fn tables_hard_wrap_with_leading_words_on_the_row() {
        // Regression: a long word hard-split after "ab " on the same row must
        // not push the 16-wide head onto the occupied row.
        let md = "| A | B |\n|---|---|\n| ab supercalifragilistic | x |";
        let rows = markdown_rows(md, Style::default(), 20);
        for row in &rows {
            let width = row.iter().map(|s| s.content.width()).sum::<usize>();
            assert!(width <= 20, "row width {width} > 20: {}", row_text(row));
        }
        let text = rows_text(&rows);
        assert!(text.contains("ab"), "leading word lost: {text}");
        assert!(text.contains("stic"), "word tail lost: {text}");
    }

    #[test]
    fn tables_wide_glyphs_in_narrow_columns_make_progress() {
        // CJK glyphs are double-width; a column shrunk below one glyph must
        // still emit rows (taking single glyphs) rather than looping forever.
        let md = "| 中文字 | x |\n|---|---|\n| 测试数据内容 | y |";
        let rows = markdown_rows(md, Style::default(), 10);
        assert!(!rows.is_empty());
        for row in &rows {
            let width = row.iter().map(|s| s.content.width()).sum::<usize>();
            assert!(width <= 10, "row width {width} > 10: {}", row_text(row));
        }
        let text = rows_text(&rows);
        assert!(text.contains('测'), "glyph content lost: {text}");
        assert!(text.contains('内'), "glyph content lost: {text}");
    }

    #[test]
    fn bullet_lists_get_markers_and_hanging_indent() {
        let rows = markdown_rows("- first item\n- second item", Style::default(), 40);
        assert_eq!(row_text(&rows[0]), "• first item");
        assert_eq!(row_text(&rows[1]), "• second item");
    }

    #[test]
    fn ordered_lists_number_items() {
        let rows = markdown_rows("1. one\n2. two", Style::default(), 40);
        assert!(rows_text(&rows).contains("1. one"));
        assert!(rows_text(&rows).contains("2. two"));
    }

    #[test]
    fn link_text_is_kept_without_url_syntax() {
        let rows = markdown_rows(
            "See [the docs](https://example.com) now",
            Style::default(),
            60,
        );
        let text = rows_text(&rows);
        assert!(text.contains("the docs"));
        assert!(!text.contains("](https"));
        assert!(!text.contains('['));
    }

    #[test]
    fn wrapping_respects_width() {
        let rows = markdown_rows("word ".repeat(30).trim(), Style::default(), 20);
        assert!(rows.len() > 1);
        for row in &rows {
            let width = row.iter().map(|s| s.content.width()).sum::<usize>();
            assert!(width <= 20, "row width {width} > 20: {}", row_text(row));
        }
    }

    #[test]
    fn blockquote_and_rule() {
        let rows = markdown_rows("> quoted line\n\n---\n", Style::default(), 40);
        let text = rows_text(&rows);
        assert!(text.contains("> quoted line"));
        assert!(text.contains('─'));
    }

    #[test]
    fn plain_text_passes_through() {
        let rows = markdown_rows("just a plain sentence", Style::default(), 60);
        assert_eq!(rows_text(&rows), "just a plain sentence");
    }
}
