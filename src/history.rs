//! Scrollback transcript rows, Aster-style.
//!
//! Every builder returns finished `Vec<Line<'static>>`, already wrapped to
//! `width`. The event loop pushes them above the bottom-anchored viewport with
//! `insert_before` and never touches them again — scrolling, selection, and
//! copy belong to the terminal. Anatomy verified against Aster's
//! `history.rs`: `hang`/`bullet`/`branch`, user band + rail, tool label +
//! sub-rows, patch counts + tinted bands, HEAD/TAIL = 4 elision.

use std::ops::Range;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;
use crate::tools::{TodoItem, TodoStatus};

/// Hanging-indent gutter: `• ` on the first wrapped row, spaces after.
const GUTTER: usize = 2;
/// Elision window for long tool output: first + last lines stay visible.
const ELIDE_HEAD: usize = 4;
const ELIDE_TAIL: usize = 4;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn faint() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn col_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn body_width(width: usize) -> usize {
    width.saturating_sub(GUTTER + 2).max(8)
}

/// Split `text` into display rows of at most `max` columns, as byte ranges.
/// Breaks after spaces; a word wider than a row is cut mid-word. Ranges tile
/// the input so every byte belongs to exactly one row.
fn rows(text: &str, max: usize) -> Vec<Range<usize>> {
    let max = max.max(1);
    let mut out = Vec::new();
    let mut start = 0;
    let mut col = 0;
    let mut pos = 0;
    for chunk in text.split_inclusive(' ') {
        let chunk_start = pos;
        pos += chunk.len();
        let word = chunk.trim_end_matches(' ');
        let word_w = col_width(word);
        if col > 0 && col + word_w > max {
            out.push(start..chunk_start);
            start = chunk_start;
            col = 0;
        }
        if word_w > max {
            let mut w = 0;
            for (off, ch) in word.char_indices() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                if w + cw > max && chunk_start + off > start {
                    out.push(start..chunk_start + off);
                    start = chunk_start + off;
                    w = 0;
                }
                w += cw;
            }
            col = w + (chunk.len() - word.len());
        } else {
            col += col_width(chunk);
        }
    }
    out.push(start..text.len());
    out
}

/// `text` as display rows, trailing spaces dropped.
fn wrapped(text: &str, max: usize) -> Vec<String> {
    rows(text, max)
        .into_iter()
        .map(|r| text[r].trim_end().to_string())
        .collect()
}

/// Re-flow a styled line to `max` columns, carrying each span's style across
/// the break.
pub(crate) fn wrap_line(line: Line<'static>, max: usize) -> Vec<Line<'static>> {
    let mut text = String::new();
    let mut runs = Vec::with_capacity(line.spans.len());
    for span in &line.spans {
        let start = text.len();
        text.push_str(&span.content);
        runs.push((start..text.len(), span.style));
    }
    let ranges = rows(&text, max);
    if ranges.len() <= 1 {
        return vec![line];
    }
    ranges
        .into_iter()
        .map(|row| {
            let spans: Vec<Span<'static>> = runs
                .iter()
                .filter_map(|(run, style)| {
                    let from = run.start.max(row.start);
                    let to = run.end.min(row.end);
                    (from < to).then(|| Span::styled(text[from..to].to_string(), *style))
                })
                .collect();
            Line::from(spans).style(line.style)
        })
        .collect()
}

/// Pad `line` with spaces so its background reaches the full width.
fn pad_to(mut line: Line<'static>, max: usize, style: Style) -> Line<'static> {
    let used = line
        .spans
        .iter()
        .map(|s| col_width(&s.content))
        .sum::<usize>();
    if used < max {
        line.spans.push(Span::styled(" ".repeat(max - used), style));
    }
    line
}

/// Hanging indent: first wrapped row gets `bullet`, continuations get
/// `GUTTER` spaces. Every input line is re-wrapped to the body width.
fn hang(lines: Vec<Line<'static>>, bullet: Span<'static>, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut first = true;
    for line in lines {
        for wrapped in wrap_line(line, body_width(width)) {
            let lead = match first {
                true => bullet.clone(),
                false => Span::raw(" ".repeat(GUTTER)),
            };
            let mut spans = vec![lead];
            spans.extend(wrapped.spans);
            out.push(Line::from(spans).style(wrapped.style));
            first = false;
        }
    }
    out
}

fn bullet() -> Span<'static> {
    Span::styled("• ", Style::default().fg(Color::DarkGray))
}

fn branch(first: bool) -> Span<'static> {
    match first {
        true => Span::styled("└ ", faint()),
        false => Span::raw("  "),
    }
}

/// Blank separator above every group, so groups read as chapters.
fn prepend_blank(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if lines.is_empty() {
        return lines;
    }
    lines.insert(0, Line::from(""));
    lines
}

/// A message the user sent: accent rail on a filled band, the only chapter
/// mark in the transcript. Multi-line prompts continue under `❯ `.
pub(crate) fn user_row(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let fill = Style::default().bg(theme.rail_bg);
    let body = body_width(width);
    let mut out = Vec::new();
    let mut first = true;
    for raw in text.lines() {
        // Keep one row per prompt line even when empty (no wrap of "").
        let chunks = match raw.is_empty() {
            true => vec![String::new()],
            false => wrapped(raw, body),
        };
        for chunk in chunks {
            let lead = match first {
                true => "❯ ",
                false => "  ",
            };
            let line = Line::from(vec![
                Span::styled("▌", Style::default().fg(theme.accent).bg(theme.rail_bg)),
                Span::styled(lead, Style::default().fg(theme.accent).bg(theme.rail_bg)),
                Span::styled(chunk, fill),
            ]);
            out.push(pad_to(line, width.max(1), fill));
            first = false;
        }
    }
    if out.is_empty() {
        return out;
    }
    prepend_blank(out)
}

/// Model reply: rendered Markdown under a bullet.
pub(crate) fn reply_rows(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let lines = crate::markdown::render(text, body_width(width), theme);
    match lines.is_empty() {
        true => Vec::new(),
        false => prepend_blank(hang(lines, bullet(), width)),
    }
}

/// Chapter break between a turn's tool block and the model's final reply:
/// a blank row, then a dim full-width rule. The reply's own leading blank
/// lands after it, so the rule sits centred in the gap.
pub(crate) fn divider_row(width: usize) -> Vec<Line<'static>> {
    prepend_blank(vec![Line::from(Span::styled(
        "─".repeat(width.max(1)),
        dim(),
    ))])
}

/// Short harness status line (`cleared.`, audit notes, `interrupted.`).
pub(crate) fn notice_row(text: &str, width: usize) -> Vec<Line<'static>> {
    prepend_blank(hang(
        vec![Line::from(Span::styled(text.to_string(), dim()))],
        Span::styled("· ", dim()),
        width,
    ))
}

/// Fatal turn error.
pub(crate) fn error_row(text: &str, width: usize) -> Vec<Line<'static>> {
    prepend_blank(hang(
        vec![Line::from(Span::styled(
            text.to_string(),
            Style::default().fg(Color::Red),
        ))],
        bullet(),
        width,
    ))
}

/// Turn trailer (`Done …` / `Interrupted …`): italic dim bullet row.
pub(crate) fn trailer_row(text: &str, width: usize) -> Vec<Line<'static>> {
    prepend_blank(hang(
        vec![Line::from(Span::styled(
            text.to_string(),
            dim().add_modifier(Modifier::ITALIC),
        ))],
        bullet(),
        width,
    ))
}

/// One finished tool call: bold label, `└ summary`, then elided output.
/// Failed calls render the label and body in red.
pub(crate) fn tool_row(
    name: &str,
    preview: &str,
    ok: bool,
    summary: &str,
    output: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let label = match preview.is_empty() {
        true => name.to_string(),
        false => format!("{name} {preview}"),
    };
    let head_style = match ok {
        true => Style::default().add_modifier(Modifier::BOLD),
        false => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    };
    let mut lines = vec![Line::from(Span::styled(label, head_style))];

    let sub_style = match ok {
        true => dim(),
        false => Style::default().fg(Color::Red),
    };
    if !summary.trim().is_empty() {
        lines.push(Line::from(vec![
            branch(true),
            Span::styled(summary.to_string(), sub_style),
        ]));
    }
    let body: Vec<&str> = output.lines().collect();
    for (i, text) in elide(&body).into_iter().enumerate() {
        let (text, style) = match text {
            Elided::Text(s) => (s.to_string(), sub_style),
            Elided::Gap(n) => (format!("… +{n} lines"), faint()),
        };
        // The summary already took the `└`; continuation rows indent.
        let lead = match i == 0 && summary.trim().is_empty() {
            true => branch(true),
            false => Span::raw("  "),
        };
        lines.push(Line::from(vec![lead, Span::styled(text, style)]));
    }
    prepend_blank(hang(lines, bullet(), width))
}

/// Max todo items shown in the pinned panel; overflow collapses to `+N more`.
pub(crate) const TODO_PIN_MAX_ITEMS: usize = 10;

/// Pinned-panel height in rows for `count` todos: 0 when empty, else one
/// header row plus one row per visible item (plus a `+N more` row on overflow).
pub(crate) fn todo_pin_height(count: usize) -> u16 {
    match count {
        0 => 0,
        _ => {
            let visible = count.min(TODO_PIN_MAX_ITEMS);
            (1 + visible + usize::from(count > visible)) as u16
        }
    }
}

/// Live todo tree pinned to the top of the bottom pane (not scrollback):
/// a `Todos · done/total` header plus one truncated row per item with
/// `├`/`└` branches and `○`/`◌`/`●` status marks. Empty renders nothing so
/// the panel costs zero rows when quiet.
pub(crate) fn todo_pin_rows(todos: &[TodoItem], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    if todos.is_empty() {
        return Vec::new();
    }
    let done = todos
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    let mut rows = vec![Line::from(vec![
        Span::styled(
            "Todos",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" · {done}/{}", todos.len()), dim()),
    ])];
    let visible = todos.len().min(TODO_PIN_MAX_ITEMS);
    let overflow = todos.len() - visible;
    for (i, item) in todos.iter().take(visible).enumerate() {
        let fork = match i + 1 == visible && overflow == 0 {
            true => "└ ",
            false => "├ ",
        };
        let (mark, style) = match item.status {
            TodoStatus::Pending => ("○ ", dim()),
            TodoStatus::InProgress => ("◌ ", Style::default().fg(theme.accent)),
            TodoStatus::Completed => ("● ", Style::default().fg(theme.add_fg)),
        };
        let flat = item
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let budget = width
            .saturating_sub(col_width(fork) + col_width(mark))
            .max(1);
        rows.push(Line::from(vec![
            Span::styled(fork.to_string(), dim()),
            Span::styled(mark.to_string(), style),
            Span::styled(truncate_width(&flat, budget), style),
        ]));
    }
    if overflow > 0 {
        rows.push(Line::from(vec![
            Span::styled("└ ".to_string(), dim()),
            Span::styled(format!("+{overflow} more"), faint()),
        ]));
    }
    rows
}

/// Truncate `s` to at most `max` display columns, appending `…` when cut.
fn truncate_width(s: &str, max: usize) -> String {
    if col_width(s) <= max {
        return s.to_string();
    }
    let mut width = 0;
    let mut out = String::new();
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + w + 1 > max {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push('…');
    out
}

/// A diff body (`git_diff` output): `verb path` header with `+N −M` counts
/// pushed right, then full-row tinted bands with a darker mark glyph.
pub(crate) fn patch_row(
    verb: &str,
    path: &str,
    body: &str,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    // `+++`/`---` file headers are not changed lines.
    let added = body
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .count();
    let removed = body
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .count();

    let inner = body_width(width);
    let left = match path.is_empty() {
        true => verb.to_string(),
        false => format!("{verb} {path}"),
    };
    let right = format!("+{added} −{removed}");
    let gap = inner.saturating_sub(col_width(&left) + col_width(&right) + 1);
    let header = Line::from(vec![
        Span::styled(
            format!("{verb} "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(path.to_string(), Style::default().fg(Color::Blue)),
        Span::raw(" ".repeat(gap + 1)),
        Span::styled(format!("+{added}"), Style::default().fg(theme.add_fg)),
        Span::raw(" "),
        Span::styled(format!("−{removed}"), Style::default().fg(theme.del_fg)),
    ]);

    let mut lines = vec![header];
    lines.extend(diff_lines(body, inner, theme));
    prepend_blank(hang(lines, bullet(), width))
}

/// Tint a unified-ish patch body row by row. File headers (`---`/`+++`,
/// `diff --git`, `index`) and hunk headers (`@@ … @@`) are skipped: the
/// hunk start lines seed old/new counters, and every change row renders a
/// `marker + number` gutter (context/add show the new-file number, deletes
/// show the old-file number) over the code without its ` `/`+`/`-` prefix.
/// Context rows stay faint on the terminal background; added/removed rows
/// get full-width green/red bands with a darker mark glyph. Long code lines
/// wrap within the code budget; continuations carry a blank gutter so the
/// band stays rectangular.
fn diff_lines(body: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    enum Kind {
        Context,
        Add,
        Del,
        Note,
    }
    struct Row {
        kind: Kind,
        num: Option<usize>,
        text: String,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut old_no: usize = 1;
    let mut new_no: usize = 1;
    for raw in body.lines() {
        if raw.starts_with("--- ")
            || raw.starts_with("+++ ")
            || raw.starts_with("diff --git")
            || raw.starts_with("index ")
            || raw.starts_with("Binary ")
        {
            continue;
        }
        if let Some((old_start, new_start)) = parse_hunk_start(raw) {
            old_no = old_start.max(1);
            new_no = new_start.max(1);
            continue;
        }
        if raw.starts_with('\\') {
            rows.push(Row {
                kind: Kind::Note,
                num: None,
                text: raw.to_string(),
            });
            continue;
        }
        match raw.chars().next() {
            Some('+') if !raw.starts_with("+++") => {
                rows.push(Row {
                    kind: Kind::Add,
                    num: Some(new_no),
                    text: raw[1..].to_string(),
                });
                new_no += 1;
            }
            Some('-') if !raw.starts_with("---") => {
                rows.push(Row {
                    kind: Kind::Del,
                    num: Some(old_no),
                    text: raw[1..].to_string(),
                });
                old_no += 1;
            }
            Some(' ') => {
                rows.push(Row {
                    kind: Kind::Context,
                    num: Some(new_no),
                    text: raw[1..].to_string(),
                });
                old_no += 1;
                new_no += 1;
            }
            None => {
                rows.push(Row {
                    kind: Kind::Context,
                    num: Some(new_no),
                    text: String::new(),
                });
                old_no += 1;
                new_no += 1;
            }
            _ => {
                rows.push(Row {
                    kind: Kind::Note,
                    num: None,
                    text: raw.to_string(),
                });
            }
        }
    }
    let num_w = rows
        .iter()
        .filter_map(|r| r.num)
        .max()
        .map(|m| m.to_string().len())
        .unwrap_or(1)
        .max(1);
    let gutter_w = num_w + 2;
    let code_budget = width.saturating_sub(gutter_w).max(1);
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let (fg, bg, mark_fg, marker) = match row.kind {
            Kind::Add => (theme.add_fg, theme.add_bg, theme.add_mark, '+'),
            Kind::Del => (theme.del_fg, theme.del_bg, theme.del_mark, '-'),
            Kind::Context => (Color::DarkGray, Color::Reset, Color::DarkGray, ' '),
            Kind::Note => (Color::DarkGray, Color::Reset, Color::DarkGray, ' '),
        };
        let gutter_text = match row.num {
            Some(n) => format!("{marker}{n:>num_w$} "),
            None => " ".repeat(gutter_w),
        };
        let gutter_style = Style::default().fg(mark_fg).bg(bg);
        let code_style = Style::default().fg(fg).bg(bg);
        let chunks = match row.text.is_empty() {
            true => vec![String::new()],
            false => wrapped(&row.text, code_budget),
        };
        for (i, chunk) in chunks.into_iter().enumerate() {
            let gutter = match i == 0 {
                true => Span::styled(gutter_text.clone(), gutter_style),
                false => Span::styled(" ".repeat(gutter_w), gutter_style),
            };
            let line = Line::from(vec![gutter, Span::styled(chunk, code_style)]);
            out.push(pad_to(line, width, code_style));
        }
    }
    out
}

/// Parse a unified hunk header (`@@ -old[,len] +new[,len] @@ …`) into its
/// 1-indexed old/new start lines. Anything else returns `None`.
fn parse_hunk_start(line: &str) -> Option<(usize, usize)> {
    let line = line.strip_prefix("@@ ")?;
    let (old_part, rest) = line.split_once(' ')?;
    let (new_part, _) = rest.split_once(" @@").unwrap_or((rest, ""));
    let old_start = old_part
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()?;
    let new_start = new_part
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()?;
    Some((old_start, new_start))
}

enum Elided<'a> {
    Text(&'a str),
    Gap(usize),
}

fn elide<'a>(body: &[&'a str]) -> Vec<Elided<'a>> {
    if body.len() <= ELIDE_HEAD + ELIDE_TAIL + 1 {
        return body.iter().copied().map(Elided::Text).collect();
    }
    let mut out: Vec<Elided<'a>> = body[..ELIDE_HEAD]
        .iter()
        .copied()
        .map(Elided::Text)
        .collect();
    out.push(Elided::Gap(body.len() - ELIDE_HEAD - ELIDE_TAIL));
    out.extend(
        body[body.len() - ELIDE_TAIL..]
            .iter()
            .copied()
            .map(Elided::Text),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wrap_line_breaks_long_styled_lines() {
        let line = Line::from(vec![Span::styled(
            "aaa bbb ccc",
            Style::default().fg(Color::Red),
        )]);
        let out = wrap_line(line, 5);
        assert_eq!(out.len(), 3);
        // Style survives the break.
        for l in &out {
            for s in &l.spans {
                assert_eq!(s.style.fg, Some(Color::Red));
            }
        }
        // Breaks land after spaces; only `wrapped()` trims the tail.
        assert_eq!(out[0].spans[0].content.as_ref(), "aaa ");
        assert_eq!(out[2].spans[0].content.as_ref(), "ccc");
    }

    #[test]
    fn hang_indents_continuations_under_bullet() {
        let lines = vec![Line::from("0123456789")];
        let out = hang(lines, bullet(), 10);
        // body width 10-4=6: "012345" + "6789".
        assert_eq!(out.len(), 2);
        let text: Vec<String> = plain(&out);
        assert!(text[0].starts_with("• "), "got: {}", text[0]);
        assert!(text[1].starts_with("  "), "got: {}", text[1]);
    }

    #[test]
    fn user_row_renders_rail_prompt_and_full_width_band() {
        let theme = Theme::default();
        let out = user_row("hello", 20, &theme);
        assert_eq!(out.len(), 2); // blank + band
        let text: Vec<String> = plain(&out);
        assert!(text[1].contains('▌'), "got: {}", text[1]);
        assert!(text[1].contains("❯ hello"), "got: {}", text[1]);
        // Band reaches the full width (bg fill).
        assert_eq!(col_width(&text[1]), 20, "got: {}", text[1]);
        // Rail carries the accent.
        let rail = &out[1].spans[0];
        assert_eq!(rail.style.fg, Some(theme.accent));
        assert_eq!(rail.style.bg, Some(theme.rail_bg));
    }

    #[test]
    fn tool_row_shows_label_summary_and_elides_long_output() {
        let output = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = tool_row("bash", "cargo test", true, "ok", &output, 80);
        let text: Vec<String> = plain(&out);
        assert!(text[1].contains("bash cargo test"), "got: {:?}", text);
        assert!(text[2].contains("└ ok"), "got: {:?}", text);
        // 4 head + gap + 4 tail, plus blank + label + summary = 12.
        assert_eq!(out.len(), 12, "got: {text:?}");
        assert!(
            text.iter().any(|l| l.contains("… +12 lines")),
            "got: {text:?}"
        );
        assert!(text.iter().any(|l| l.contains("line 19")), "got: {text:?}");
        // No gap for short output.
        let short = tool_row("read", "a.rs", true, "s", "l1\nl2", 80);
        let short_text: Vec<String> = plain(&short);
        assert!(
            !short_text.iter().any(|l| l.contains('+')),
            "got: {short_text:?}"
        );
    }

    #[test]
    fn failed_tool_row_renders_red() {
        let out = tool_row("bash", "rm x", false, "failed: nope", "err", 80);
        assert_eq!(out[1].spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn patch_row_counts_and_tints_bands() {
        let theme = Theme::default();
        let body = "--- a/f\n+++ b/f\n ctx\n+ add\n- del";
        let out = patch_row("Diff", "f", body, 40, &theme);
        let text: Vec<String> = plain(&out);
        assert!(text[1].contains("+1"), "got: {:?}", text);
        assert!(text[1].contains("−1"), "got: {:?}", text);
        // File headers never render; the gutter carries the change marks.
        assert!(!text.iter().any(|l| l.contains("--- a/f")), "got: {text:?}");
        // Added line carries the green band + `+` gutter; deletes are red.
        let add_line = out
            .iter()
            .find(|l| plain(std::slice::from_ref(l))[0].contains("add"));
        let add_line = add_line.expect("added line present");
        let add_text = plain(std::slice::from_ref(add_line))[0].clone();
        assert!(add_text.contains("+"), "got: {add_text:?}");
        assert_eq!(add_line.spans.last().unwrap().style.bg, Some(theme.add_bg));
        assert_eq!(add_line.spans.last().unwrap().style.fg, Some(theme.add_fg));
        let del_line = out
            .iter()
            .find(|l| plain(std::slice::from_ref(l))[0].contains("del"));
        let del_line = del_line.expect("deleted line present");
        let del_text = plain(std::slice::from_ref(del_line))[0].clone();
        assert!(del_text.contains("-"), "got: {del_text:?}");
        assert_eq!(del_line.spans.last().unwrap().style.bg, Some(theme.del_bg));
        assert_eq!(del_line.spans.last().unwrap().style.fg, Some(theme.del_fg));
        let ctx_line = out
            .iter()
            .find(|l| plain(std::slice::from_ref(l))[0].contains("ctx"));
        let ctx_line = ctx_line.expect("context line present");
        assert_eq!(ctx_line.spans.last().unwrap().style.bg, Some(Color::Reset));
    }

    #[test]
    fn patch_row_numbers_track_hunk_starts_like_screenshot() {
        let theme = Theme::default();
        let body = "--- a/src/history.rs\n+++ b/src/history.rs\n@@ -360,6 +360,6 @@\n ctx-a\n-old\n+new\n ctx-b";
        let out = patch_row("Edit", "src/history.rs", body, 60, &theme);
        let text: Vec<String> = plain(&out);
        // Hunk header feeds the counters; context uses the new-file number.
        assert!(text.iter().any(|l| l.contains("+361 new")), "got: {text:?}");
        assert!(text.iter().any(|l| l.contains("-361 old")), "got: {text:?}");
        assert!(
            text.iter().any(|l| l.contains(" 360 ctx-a")),
            "got: {text:?}"
        );
        assert!(!text.iter().any(|l| l.contains("@@")), "got: {text:?}");
    }

    #[test]
    fn parse_hunk_start_reads_old_and_new_starts() {
        assert_eq!(
            parse_hunk_start("@@ -360,6 +361,5 @@ fn x"),
            Some((360, 361))
        );
        assert_eq!(parse_hunk_start("@@ -1 +1 @@"), Some((1, 1)));
        assert_eq!(parse_hunk_start(" ctx"), None);
    }

    #[test]
    fn divider_row_is_blank_then_full_width_rule() {
        let out = divider_row(20);
        let text: Vec<String> = plain(&out);
        assert_eq!(text.len(), 2, "got: {text:?}");
        assert_eq!(text[0], "", "got: {text:?}");
        assert_eq!(text[1], "─".repeat(20), "got: {text:?}");
        assert_eq!(out[1].spans[0].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn todo_pin_rows_match_reference_tree() {
        let theme = Theme::default();
        let todos = vec![
            TodoItem {
                content: "Add shared file diff helper with tests".to_string(),
                status: TodoStatus::Completed,
            },
            TodoItem {
                content: "Emit diffs from write/edit tools".to_string(),
                status: TodoStatus::InProgress,
            },
            TodoItem {
                content: "Render write/edit as patch rows in TUI".to_string(),
                status: TodoStatus::Pending,
            },
            TodoItem {
                content: "Verify with tests, fmt, clippy".to_string(),
                status: TodoStatus::Pending,
            },
        ];
        let out = todo_pin_rows(&todos, 60, &theme);
        let text: Vec<String> = plain(&out);
        assert_eq!(text.len(), 5, "got: {text:?}");
        assert_eq!(text[0], "Todos · 1/4", "got: {text:?}");
        assert_eq!(
            text[1], "├ ● Add shared file diff helper with tests",
            "got: {text:?}"
        );
        assert_eq!(
            text[2], "├ ◌ Emit diffs from write/edit tools",
            "got: {text:?}"
        );
        assert_eq!(
            text[4], "└ ○ Verify with tests, fmt, clippy",
            "got: {text:?}"
        );
        assert_eq!(todo_pin_height(todos.len()), 5);
        assert_eq!(out[0].spans[0].style.fg, Some(theme.accent));
        assert_eq!(out[3].spans[1].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn todo_pin_empty_renders_nothing_and_collapses_overflow() {
        let theme = Theme::default();
        assert!(todo_pin_rows(&[], 60, &theme).is_empty());
        assert_eq!(todo_pin_height(0), 0);
        let todos = (0..12)
            .map(|i| TodoItem {
                content: format!("task {i}"),
                status: TodoStatus::Pending,
            })
            .collect::<Vec<_>>();
        let out = todo_pin_rows(&todos, 60, &theme);
        let text: Vec<String> = plain(&out);
        assert_eq!(out.len(), 12, "got: {text:?}");
        assert_eq!(todo_pin_height(12), 12);
        assert!(text[0].starts_with("Todos · 0/12"), "got: {text:?}");
        assert_eq!(text[11], "└ +2 more", "got: {text:?}");
        assert!(text[1].starts_with("├ ○ task 0"), "got: {text:?}");
        let long = vec![TodoItem {
            content: "a very long todo item that must not overflow the panel width".to_string(),
            status: TodoStatus::Pending,
        }];
        let rows = todo_pin_rows(&long, 20, &theme);
        let flat: Vec<String> = plain(&rows);
        assert!(flat[1].ends_with('…'), "got: {flat:?}");
    }

    #[test]
    fn trailer_and_notice_carry_bullet_glyphs() {
        let t: Vec<String> = plain(&trailer_row("Done (1.2s · 2 tools)", 80));
        assert!(t[1].starts_with("• "), "got: {t:?}");
        let n: Vec<String> = plain(&notice_row("cleared.", 80));
        assert!(n[1].starts_with("· "), "got: {n:?}");
    }
}
