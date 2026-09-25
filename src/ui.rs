//! Terminal rendering with Ratatui. The UI only presents state: it reads the
//! [`App`] and turns [`Document`]s into styled lines. The only state it
//! writes back is presentational (scroll offsets, viewport height).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::assistant::{Mode, SuggestionKind};
use crate::document::{self, Document, Row, Tone};
use crate::regex::{RegexAnalyzer, RegexTokenKind, StandardRegexAnalyzer};
use crate::search::tokenizer::{TokenKind, tokenize};

// Palette (16-color safe).
const ACCENT: Color = Color::Cyan;
const HEADING: Color = Color::Yellow;
const CODE: Color = Color::Green;
const MUTED: Color = Color::DarkGray;
const SELECTED_BG: Color = Color::DarkGray;

/// Width at which results and preview are shown side by side.
const SIDE_BY_SIDE: u16 = 100;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [input, main, help] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_input(frame, input, app);
    app.viewport = main.height;
    if app.details.is_empty() {
        draw_search(frame, main, app);
    } else {
        draw_detail(frame, main, app);
    }
    draw_help(frame, help, app);
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let right = format!(
        " {} · {} entradas ",
        app.mode().label(),
        app.knowledge_size()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(MUTED))
        .title(Span::styled(
            " TermSense ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_top(Line::from(Span::styled(right, Style::new().fg(MUTED))).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = app.editor.text();
    let prompt_width = 2u16;
    let available = inner.width.saturating_sub(prompt_width + 1) as usize;
    let cursor_col = app.editor.cursor_column();
    // Horizontal scroll: keep the cursor visible on long input.
    let skip = cursor_col.saturating_sub(available);
    let mut spans = vec![Span::styled(
        "> ",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    spans.extend(skip_columns(highlight_input(text), skip));
    if text.is_empty() {
        spans.push(Span::styled(
            "digite um comando, conceito ou pergunta…",
            Style::new().fg(MUTED).add_modifier(Modifier::ITALIC),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
    if app.details.is_empty() {
        let x = inner.x + prompt_width + (cursor_col - skip) as u16;
        frame.set_cursor_position(Position::new(
            x.min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}

/// Colors the query using the shell tokenizer (or the regex parser).
fn highlight_input(text: &str) -> Vec<Span<'static>> {
    let lead = text.len() - text.trim_start().len();
    let rest = &text[lead..];
    let is_regex = rest
        .split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case("regex") || w.eq_ignore_ascii_case("regexp"));
    let mut pieces: Vec<(usize, usize, Style)> = Vec::new();
    if is_regex {
        let kw_end = lead + rest.find(char::is_whitespace).unwrap_or(rest.len());
        pieces.push((
            lead,
            kw_end,
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        let pattern_start = (kw_end + 1).min(text.len());
        for t in StandardRegexAnalyzer.parse(&text[pattern_start..]) {
            pieces.push((
                pattern_start + t.start,
                pattern_start + t.end,
                regex_style(t.kind),
            ));
        }
    } else {
        for t in tokenize(text) {
            pieces.push((t.start, t.end, token_style(t.kind)));
        }
    }
    let mut spans = Vec::new();
    let mut pos = 0;
    for (start, end, style) in pieces {
        if start < pos || end > text.len() {
            continue;
        }
        if start > pos {
            spans.push(Span::raw(text[pos..start].to_string()));
        }
        spans.push(Span::styled(text[start..end].to_string(), style));
        pos = end;
    }
    if pos < text.len() {
        spans.push(Span::raw(text[pos..].to_string()));
    }
    spans
}

fn token_style(kind: TokenKind) -> Style {
    let s = Style::new();
    match kind {
        TokenKind::Command => s.fg(ACCENT).add_modifier(Modifier::BOLD),
        TokenKind::Option | TokenKind::EndOfOptions => s.fg(Color::Yellow),
        TokenKind::String => s.fg(Color::Green),
        TokenKind::Path | TokenKind::Glob => s.fg(Color::Blue),
        TokenKind::Url | TokenKind::Host | TokenKind::Number => s.fg(Color::LightBlue),
        TokenKind::Variable | TokenKind::Substitution | TokenKind::Assignment => {
            s.fg(Color::Magenta)
        }
        TokenKind::Pipe | TokenKind::Operator | TokenKind::Redirect => {
            s.fg(Color::Magenta).add_modifier(Modifier::BOLD)
        }
        TokenKind::Word => s,
    }
}

fn regex_style(kind: RegexTokenKind) -> Style {
    use RegexTokenKind as K;
    let s = Style::new();
    match kind {
        K::StartAnchor
        | K::EndAnchor
        | K::WordBoundary
        | K::NonWordBoundary
        | K::TextStart
        | K::TextEnd => s.fg(Color::Magenta).add_modifier(Modifier::BOLD),
        K::CharacterClass | K::ShorthandClass | K::UnicodeClass | K::AnyChar => s.fg(ACCENT),
        K::Quantifier => s.fg(Color::Yellow),
        K::GroupOpen | K::GroupClose | K::Alternation | K::Backreference | K::Flags => {
            s.fg(Color::Blue)
        }
        K::Literal => s.fg(Color::Green),
        K::Invalid => s.fg(Color::Red).add_modifier(Modifier::UNDERLINED),
    }
}

fn skip_columns(spans: Vec<Span<'static>>, mut skip: usize) -> Vec<Span<'static>> {
    if skip == 0 {
        return spans;
    }
    let mut out = Vec::new();
    for span in spans {
        if skip == 0 {
            out.push(span);
            continue;
        }
        let mut kept = String::new();
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            if skip >= w && skip > 0 {
                skip -= w;
            } else {
                skip = 0;
                kept.push(c);
            }
        }
        if !kept.is_empty() {
            out.push(Span::styled(kept, span.style));
        }
    }
    out
}

fn draw_search(frame: &mut Frame, area: Rect, app: &mut App) {
    let (list_area, preview_area) = if area.width >= SIDE_BY_SIDE {
        let [a, b] = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
            .areas(area);
        (a, b)
    } else {
        let [a, b] =
            Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
        (a, b)
    };
    draw_list(frame, list_area, app);
    draw_preview(frame, preview_area, app);
}

fn panel(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(MUTED))
        .title(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD),
        ))
}

fn draw_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let heading = match app.mode() {
        Mode::Home => " Temas ".to_string(),
        _ => format!(" Sugestões ({}) ", app.suggestions().len()),
    };
    let block = panel(heading);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    if height == 0 {
        return;
    }
    if app.selected < app.list_offset {
        app.list_offset = app.selected;
    } else if app.selected >= app.list_offset + height {
        app.list_offset = app.selected + 1 - height;
    }
    let width = inner.width as usize;
    let lines: Vec<Line> = app
        .suggestions()
        .iter()
        .enumerate()
        .skip(app.list_offset)
        .take(height)
        .map(|(i, s)| {
            let selected = i == app.selected;
            let marker = if selected { "▸ " } else { "  " };
            let indent = if s.indent > 0 { "  ↳ " } else { "" };
            let tag = format!("{:<8} ", s.kind.label());
            let used = 2 + indent.width() + tag.width();
            let title_room = width.saturating_sub(used);
            let title = truncate(&s.title, title_room);
            let sub_room = width.saturating_sub(used + title.width() + 2);
            let mut spans = vec![
                Span::styled(marker, Style::new().fg(ACCENT)),
                Span::styled(indent, Style::new().fg(MUTED)),
                Span::styled(tag, Style::new().fg(kind_color(s.kind))),
                Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
            ];
            if sub_room > 3 && !s.subtitle.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", truncate(&s.subtitle, sub_room)),
                    Style::new().fg(MUTED),
                ));
            }
            let line = Line::from(spans);
            if selected {
                line.style(Style::new().bg(SELECTED_BG))
            } else {
                line
            }
        })
        .collect();
    if lines.is_empty() {
        let msg = Line::from(Span::styled("Nenhum resultado.", Style::new().fg(MUTED)));
        frame.render_widget(Paragraph::new(msg), inner);
    } else {
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn kind_color(kind: SuggestionKind) -> Color {
    match kind {
        SuggestionKind::Command | SuggestionKind::Subcommand => ACCENT,
        SuggestionKind::Recipe => Color::Magenta,
        SuggestionKind::Concept | SuggestionKind::Category => Color::Blue,
        SuggestionKind::CommandLine => CODE,
        SuggestionKind::Option => Color::Yellow,
        SuggestionKind::Analysis | SuggestionKind::Snippet => Color::LightMagenta,
    }
}

fn draw_preview(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel(" Pré-visualização ".to_string());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rendered = render_document(&app.preview, inner.width as usize, None, false);
    let max = rendered.lines.len().saturating_sub(inner.height as usize) as u16;
    app.preview_scroll = app.preview_scroll.min(max);
    let more = app.preview_scroll < max;
    frame.render_widget(
        Paragraph::new(rendered.lines).scroll((app.preview_scroll, 0)),
        inner,
    );
    if more && inner.height > 0 {
        let hint = Span::styled(" ↓ PgDn ", Style::new().fg(MUTED));
        let r = Rect::new(
            inner.right().saturating_sub(8),
            area.bottom().saturating_sub(1),
            8,
            1,
        );
        frame.render_widget(Paragraph::new(Line::from(hint)), r);
    }
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &mut App) {
    let depth = app.details.len();
    let Some(view) = app.detail_mut() else { return };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            " Detalhes ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_top(
            Line::from(Span::styled(
                format!(" nível {depth} · Esc volta "),
                Style::new().fg(MUTED),
            ))
            .right_aligned(),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rendered = render_document(&view.doc, inner.width as usize, view.focus, true);
    let height = inner.height as usize;
    if view.follow_focus {
        if let Some(line) = view.focus.and_then(|f| rendered.link_lines.get(f)) {
            let scroll = view.scroll as usize;
            if *line < scroll {
                view.scroll = *line as u16;
            } else if *line >= scroll + height {
                view.scroll = (line + 1 - height) as u16;
            }
        }
        view.follow_focus = false;
    }
    let max = rendered.lines.len().saturating_sub(height) as u16;
    view.scroll = view.scroll.min(max);
    frame.render_widget(
        Paragraph::new(rendered.lines).scroll((view.scroll, 0)),
        inner,
    );
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let keys: &[(&str, &str)] = if app.details.is_empty() {
        &[
            ("↑↓", "navegar"),
            ("Enter", "abrir"),
            ("Tab", "completar"),
            ("PgUp/PgDn", "rolar"),
            ("Esc", "sair"),
        ]
    } else {
        &[
            ("↑↓", "rolar"),
            ("Tab", "próximo link"),
            ("Enter", "abrir link"),
            ("Esc", "voltar"),
            ("Ctrl+C", "sair"),
        ]
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, d) in keys {
        spans.push(Span::styled(
            *k,
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {d}   "), Style::new().fg(MUTED)));
    }
    let warnings = app.warnings();
    if let Some(w) = warnings.first() {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let room = (area.width as usize).saturating_sub(used + 1);
        let text = if warnings.len() > 1 {
            format!("⚠ {} avisos: {w}", warnings.len())
        } else {
            format!("⚠ {w}")
        };
        spans.push(Span::styled(
            truncate(&text, room),
            Style::new().fg(Color::Yellow),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// Document rendering
// ---------------------------------------------------------------------------

/// Lines of a rendered document, plus the line where each link starts.
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub link_lines: Vec<usize>,
}

struct Writer {
    width: usize,
    focus: Option<usize>,
    interactive: bool,
    lines: Vec<Line<'static>>,
    link_lines: Vec<usize>,
}

impl Writer {
    /// Registers a link at the next line; returns the style modifier for it.
    fn link(&mut self) -> Style {
        let index = self.link_lines.len();
        self.link_lines.push(self.lines.len());
        match (self.interactive, self.focus == Some(index)) {
            (true, true) => Style::new().add_modifier(Modifier::REVERSED),
            (true, false) => Style::new().add_modifier(Modifier::UNDERLINED),
            _ => Style::new(),
        }
    }

    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| l.width() > 0) {
            self.lines.push(Line::default());
        }
    }

    fn push(&mut self, spans: Vec<Span<'static>>) {
        self.lines.push(Line::from(spans));
    }

    /// Wrapped text with a first-line prefix and a hanging indent.
    fn wrapped(&mut self, prefix: Span<'static>, text: &str, style: Style, indent: usize) {
        let room = self.width.saturating_sub(indent).max(8);
        for (i, part) in wrap(text, room).into_iter().enumerate() {
            let lead = if i == 0 {
                prefix.clone()
            } else {
                Span::raw(" ".repeat(indent))
            };
            self.push(vec![lead, Span::styled(part, style)]);
        }
    }
}

/// Renders a document to styled lines. `focus` highlights a link when the
/// view is `interactive` (detail view).
pub fn render_document(
    doc: &Document,
    width: usize,
    focus: Option<usize>,
    interactive: bool,
) -> Rendered {
    let mut w = Writer {
        width: width.max(10),
        focus,
        interactive,
        lines: Vec::new(),
        link_lines: Vec::new(),
    };
    let mut title = vec![Span::styled(
        doc.title.clone(),
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if let Some(sub) = &doc.subtitle {
        title.push(Span::styled(format!("  · {sub}"), Style::new().fg(MUTED)));
    }
    w.push(title);
    w.lines.push(Line::default());

    for block in &doc.blocks {
        match block {
            document::Block::Heading(h) => {
                w.blank();
                w.push(vec![Span::styled(
                    h.clone(),
                    Style::new().fg(HEADING).add_modifier(Modifier::BOLD),
                )]);
            }
            document::Block::Paragraph(p) => {
                w.wrapped(Span::raw(""), p, Style::new(), 0);
            }
            document::Block::Code {
                text,
                caption,
                link,
            } => {
                let style = link.as_ref().map_or(Style::new(), |_| w.link()).fg(CODE);
                w.wrapped(Span::raw("  "), text, style, 4);
                if let Some(c) = caption {
                    w.wrapped(Span::raw("    "), c, Style::new().fg(MUTED), 4);
                }
            }
            document::Block::Table(rows) => render_table(&mut w, rows),
            document::Block::List(items) => {
                for item in items {
                    let style = item.link.as_ref().map_or(Style::new(), |_| w.link());
                    let mut text = vec![
                        Span::raw("  • "),
                        Span::styled(item.text.clone(), style.add_modifier(Modifier::BOLD)),
                    ];
                    if let Some(d) = &item.detail {
                        let room = w.width.saturating_sub(4 + item.text.width() + 3);
                        text.push(Span::styled(
                            format!(" — {}", truncate(d, room)),
                            Style::new().fg(MUTED),
                        ));
                    }
                    w.push(text);
                }
            }
            document::Block::Steps(steps) => {
                for (i, s) in steps.iter().enumerate() {
                    w.push(vec![
                        Span::styled(format!("  {}. ", i + 1), Style::new().fg(HEADING)),
                        Span::styled(s.title.clone(), Style::new().add_modifier(Modifier::BOLD)),
                    ]);
                    if let Some(cmd) = &s.command {
                        let style = s.link.as_ref().map_or(Style::new(), |_| w.link()).fg(CODE);
                        w.wrapped(Span::raw("     "), cmd, style, 7);
                    }
                    if !s.why.is_empty() {
                        w.wrapped(Span::raw("     "), &s.why, Style::new().fg(MUTED), 5);
                    }
                }
            }
            document::Block::Tree { root, children } => {
                w.push(vec![Span::styled(
                    format!("  {root}"),
                    Style::new().add_modifier(Modifier::BOLD),
                )]);
                for (i, c) in children.iter().enumerate() {
                    let branch = if i + 1 == children.len() {
                        "  └── "
                    } else {
                        "  ├── "
                    };
                    let style = c
                        .link
                        .as_ref()
                        .map_or(Style::new(), |_| w.link())
                        .fg(ACCENT);
                    let mut spans = vec![
                        Span::styled(branch, Style::new().fg(MUTED)),
                        Span::styled(c.text.clone(), style),
                    ];
                    if let Some(d) = &c.detail {
                        let room = w.width.saturating_sub(6 + c.text.width() + 2);
                        spans.push(Span::styled(
                            format!("  {}", truncate(d, room)),
                            Style::new().fg(MUTED),
                        ));
                    }
                    w.push(spans);
                }
            }
            document::Block::Flow(nodes) => {
                for n in nodes {
                    let style = if n.edge.is_none() {
                        Style::new().fg(MUTED)
                    } else {
                        Style::new().fg(CODE)
                    };
                    w.wrapped(Span::raw("  "), &n.label, style, 4);
                    if let Some(e) = &n.edge {
                        w.push(vec![Span::styled(
                            format!("  │ {e}"),
                            Style::new().fg(MUTED),
                        )]);
                        w.push(vec![Span::styled("  ▼", Style::new().fg(MUTED))]);
                    }
                }
            }
            document::Block::Note { tone, text } => {
                let (icon, style) = match tone {
                    Tone::Info => ("ℹ ", Style::new().fg(Color::Blue)),
                    Tone::Success => ("✔ ", Style::new().fg(Color::Green)),
                    Tone::Warning => ("⚠ ", Style::new().fg(Color::Yellow)),
                    Tone::Danger => (
                        "✖ ",
                        Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Tone::Muted => ("", Style::new().fg(MUTED).add_modifier(Modifier::ITALIC)),
                    Tone::Normal | Tone::Accent => ("• ", Style::new()),
                };
                if *tone == Tone::Muted {
                    w.blank();
                }
                w.wrapped(Span::styled(icon, style), text, style, icon.width());
            }
        }
    }
    Rendered {
        lines: w.lines,
        link_lines: w.link_lines,
    }
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Normal | Tone::Accent => Style::new().fg(ACCENT),
        Tone::Muted => Style::new().fg(MUTED),
        Tone::Info => Style::new().fg(Color::Blue),
        Tone::Success => Style::new().fg(Color::Green),
        Tone::Warning => Style::new().fg(Color::Yellow),
        Tone::Danger => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

/// Aligned columns; the last column wraps with a hanging indent. A first
/// cell too wide for its column gets a line of its own.
fn render_table(w: &mut Writer, rows: &[Row]) {
    let cols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let cell_width = |r: &Row, i: usize| {
        r.cells.get(i).map_or(0, |c| c.width()) + if i == 0 { 2 * r.indent as usize } else { 0 }
    };
    let caps: Vec<usize> = match cols {
        1 => vec![w.width],
        2 => vec![(w.width * 2 / 5).max(8)],
        _ => vec![(w.width * 3 / 10).max(8), (w.width / 5).max(6)],
    };
    let widths: Vec<usize> = (0..cols.saturating_sub(1))
        .map(|i| {
            let natural = rows
                .iter()
                .filter(|r| r.cells.len() > i + 1)
                .map(|r| cell_width(r, i))
                .max()
                .unwrap_or(0);
            natural.min(caps.get(i).copied().unwrap_or(12))
        })
        .collect();
    let base = 2;
    let desc_col = base + widths.iter().map(|x| x + 2).sum::<usize>();

    for row in rows {
        let link_style = row.link.as_ref().map(|_| w.link());
        let first_style = tone_style(row.tone).patch(link_style.unwrap_or_default());
        let indent = " ".repeat(2 * row.indent as usize);
        if row.cells.len() == 1 || cols == 1 {
            w.wrapped(
                Span::raw(format!("  {indent}")),
                &row.cells[0],
                first_style,
                base + indent.len(),
            );
            continue;
        }
        let mut spans = vec![Span::raw(" ".repeat(base))];
        let mut col = base;
        let last = row.cells.len() - 1;
        for (i, cell) in row.cells.iter().enumerate().take(last) {
            let width = widths.get(i).copied().unwrap_or(0);
            let text = if i == 0 {
                format!("{indent}{cell}")
            } else {
                cell.clone()
            };
            let style = if i == 0 {
                first_style
            } else {
                Style::new().fg(MUTED)
            };
            if text.width() > width {
                // Too wide: the cell takes its own line.
                spans.push(Span::styled(text, style));
                w.push(std::mem::take(&mut spans));
                spans.push(Span::raw(
                    " ".repeat(base + widths[..=i].iter().map(|x| x + 2).sum::<usize>()),
                ));
                col = base + widths[..=i].iter().map(|x| x + 2).sum::<usize>();
            } else {
                let pad = width - text.width() + 2;
                spans.push(Span::styled(text, style));
                spans.push(Span::raw(" ".repeat(pad)));
                col += width + 2;
            }
        }
        let room = w.width.saturating_sub(desc_col).max(12);
        let desc_style = if row.tone == Tone::Danger {
            tone_style(Tone::Danger)
        } else {
            Style::new()
        };
        let parts = wrap(&row.cells[last], room);
        for (i, part) in parts.into_iter().enumerate() {
            if i == 0 {
                spans.push(Span::styled(part, desc_style));
                w.push(std::mem::take(&mut spans));
            } else {
                w.push(vec![
                    Span::raw(" ".repeat(desc_col.max(col))),
                    Span::styled(part, desc_style),
                ]);
            }
        }
        if !spans.is_empty() {
            w.push(spans);
        }
    }
}

/// Word wrap by display width; long words are split.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut line_w = 0;
        for word in paragraph.split(' ') {
            let ww = word.width();
            if line_w > 0 && line_w + 1 + ww > width {
                lines.push(std::mem::take(&mut line));
                line_w = 0;
            }
            if ww > width {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if line_w + cw > width {
                        lines.push(std::mem::take(&mut line));
                        line_w = 0;
                    }
                    line.push(c);
                    line_w += cw;
                }
                continue;
            }
            if line_w > 0 {
                line.push(' ');
                line_w += 1;
            }
            line.push_str(word);
            line_w += ww;
        }
        lines.push(line);
    }
    lines
}

/// Cuts `s` to `width` columns, ending with `…` when shortened.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw + 1 > width {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// Plain-text rendering (for `ts --print`).
pub fn document_text(doc: &Document, width: usize) -> Vec<String> {
    render_document(doc, width, None, false)
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::Assistant;
    use crate::config::Config;
    use crate::input::Action;
    use crate::knowledge::Repository;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app(query: &str) -> App {
        App::new(
            Assistant::new(Repository::embedded().unwrap(), &Config::default()),
            query,
        )
    }

    fn screen(app: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_search_and_detail_views() {
        let mut a = app("grep -r");
        let s = screen(&mut a, 120, 30);
        assert!(s.contains("TermSense"));
        assert!(s.contains("> grep -r"));
        assert!(s.contains("Sugestões"));
        assert!(s.contains("Enter abrir"));
        a.handle(Action::Open);
        let s = screen(&mut a, 120, 30);
        assert!(s.contains("Detalhes"));
        assert!(s.contains("PARTES"));
    }

    #[test]
    fn narrow_and_tiny_terminals_do_not_panic() {
        for (w, h) in [(40, 12), (20, 6), (10, 5), (200, 60)] {
            for q in [
                "",
                "gr",
                "regex ^(a|b)+$",
                "ss -ltnp | grep ':8080'",
                "/24",
                "porta 8080",
            ] {
                let mut a = app(q);
                screen(&mut a, w, h);
                a.handle(Action::Open);
                a.handle(Action::Next);
                screen(&mut a, w, h);
            }
        }
    }

    #[test]
    fn wrap_and_truncate() {
        assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(truncate("conexão", 5), "cone…");
        assert_eq!(truncate("ok", 5), "ok");
    }

    #[test]
    fn link_lines_match_document_links() {
        let repo = Repository::embedded().unwrap();
        let mut a = Assistant::new(repo, &Config::default());
        for q in ["grep", "ssh", "não consigo acessar servidor", "curl"] {
            let r = a.respond(q);
            let doc = a.preview(&r.suggestions[0]);
            let rendered = render_document(&doc, 80, None, true);
            assert_eq!(rendered.link_lines.len(), doc.links().len(), "{q}");
        }
    }

    #[test]
    fn input_highlighting_covers_the_whole_text() {
        for q in [
            "grep -rin \"ERROR\" ./logs | sort",
            "regex ^[0-9]+$",
            "  ls  ",
            "echo 'aberto",
        ] {
            let text: String = highlight_input(q)
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(text, q);
        }
    }
}
