//! A small Markdown parser for the quick ask answers, the rules of the page
//! this window replaced (src/markdown.ts). It is run again for every streamed
//! piece, so a half-written answer renders too: an unclosed code fence is a
//! code block so far, an unclosed **bold** shows its stars until the closing
//! ones arrive. Nothing is ever read as HTML.
//!
//! Supported: paragraphs (single line breaks kept), headings, lists (nested
//! by indentation), block quotes, fenced code, tables, rules, and inline
//! code, bold, italic, strikethrough and links.
//!
//! The answer becomes a flat list of blocks, each with the space above it
//! (CSS margins collapsed as the page had them); the window lays them out
//! one under the other. Inline formatting becomes Slint's styled text
//! ([`styled`]).

use i_slint_common::styled_text::{FormattedSpan, Style, StyledTextParagraph};

/// Inline content of one line
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strike(Vec<Inline>),
    Link { href: String, label: Vec<Inline> },
}

/// Lines of inline content, shown one under the other (a paragraph's single
/// line breaks)
pub type Lines = Vec<Vec<Inline>>;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Paragraph(Lines),
    /// 3 for `#` and `##`, 4 below
    Heading(u8, Lines),
    ListItem { marker: String, depth: usize, lines: Lines },
    Code(String),
    Rule,
    TableRow { cells: Vec<Vec<Inline>>, header: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: Kind,
    /// Inside a block quote
    pub quoted: bool,
    /// In the same block quote as the block above
    pub joined: bool,
    /// Inside a list item (a code block or text indented under it): how many
    /// list levels in
    pub nest: usize,
    /// Space above, in logical pixels
    pub gap: f32,
}

/// Margins as the page's CSS gave them: above and below a block
struct Margins {
    top: f32,
    bottom: f32,
}

struct Parsed {
    kind: Kind,
    /// The block quote it is in (counted from 1), or 0
    quote: usize,
    nest: usize,
    margins: Margins,
}

/// The blocks of an answer.
pub fn parse(text: &str) -> Vec<Block> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut parsed = Vec::new();
    let mut quotes = 0;
    blocks(&lines, 0, &mut quotes, &mut parsed);

    // Adjacent margins collapse into the larger one; nothing above the first
    let mut out = Vec::with_capacity(parsed.len());
    let mut previous: Option<(f32, usize)> = None;
    for block in parsed {
        let gap = previous.map_or(0.0, |(bottom, _)| bottom.max(block.margins.top));
        let joined = block.quote > 0 && previous.is_some_and(|(_, quote)| quote == block.quote);
        previous = Some((block.margins.bottom, block.quote));
        out.push(Block {
            kind: block.kind,
            quoted: block.quote > 0,
            joined,
            nest: block.nest,
            gap,
        });
    }
    out
}

fn push(out: &mut Vec<Parsed>, kind: Kind, quote: usize, top: f32, bottom: f32) {
    out.push(Parsed {
        kind,
        quote,
        nest: 0,
        margins: Margins { top, bottom },
    });
}

/// Characters of indentation
fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// `line` without up to `cut` characters of indentation
fn dedent(line: &str, cut: usize) -> &str {
    let skip: usize = line
        .chars()
        .take(cut)
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum();
    &line[skip..]
}

/// `quoted` is the block quote the lines are in, 0 for none; `quotes`
/// counts them
fn blocks(lines: &[&str], quoted: usize, quotes: &mut usize, out: &mut Vec<Parsed>) {
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        if let Some(marker) = fence(line) {
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with(&marker) {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // the closing fence, if it has arrived
            push(out, Kind::Code(code.join("\n")), quoted, 0.0, 8.0);
            continue;
        }

        if let Some((level, text)) = heading(line) {
            let level = if level <= 2 { 3 } else { 4 };
            push(out, Kind::Heading(level, vec![inline(text)]), quoted, 12.0, 6.0);
            i += 1;
            continue;
        }

        if is_rule(line) {
            push(out, Kind::Rule, quoted, 10.0, 10.0);
            i += 1;
            continue;
        }

        if quote(line).is_some() {
            let mut inner = Vec::new();
            while i < lines.len() {
                let Some(rest) = quote(lines[i]) else { break };
                inner.push(rest);
                i += 1;
            }
            let start = out.len();
            *quotes += 1;
            let this = if quoted > 0 { quoted } else { *quotes };
            blocks(&inner, this, quotes, out);
            // The quote's own margin below
            if let Some(last) = out[start..].last_mut() {
                last.margins.bottom = last.margins.bottom.max(8.0);
            }
            continue;
        }

        if list_item(line).is_some() {
            i = list(lines, i, 0, quoted, out);
            if let Some(last) = out.last_mut() {
                last.margins.bottom = last.margins.bottom.max(8.0);
            }
            continue;
        }

        if line.contains('|') && i + 1 < lines.len() && is_table_divider(lines[i + 1]) {
            i = table(lines, i, quoted, out);
            continue;
        }

        // A paragraph: up to a blank line or the start of another block
        let mut paragraph = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() && (paragraph.is_empty() || !is_block_start(lines[i])) {
            paragraph.push(inline(lines[i].trim()));
            i += 1;
        }
        push(out, Kind::Paragraph(paragraph), quoted, 0.0, 8.0);
    }
}

/// A list from line `start`; returns the first line after it.
fn list(lines: &[&str], start: usize, depth: usize, quoted: usize, out: &mut Vec<Parsed>) -> usize {
    let Some(first) = list_item(lines[start]) else {
        return start + 1;
    };
    let indent = first.indent;
    let ordered = first.number.is_some();
    let mut number = first.number.unwrap_or(1);
    let mut first_item = true;

    let mut i = start;
    while i < lines.len() {
        let Some(item) = list_item(lines[i]) else { break };
        if item.indent < indent {
            break;
        }
        if item.indent > indent {
            // A nested list belongs to the item before it
            i = list(lines, i, depth + 1, quoted, out);
            continue;
        }
        if item.number.is_some() != ordered {
            break;
        }

        let mut body = vec![inline(item.rest)];
        i += 1;
        // What is indented under the item: more of its text, then maybe code
        // blocks and paragraphs that belong to it
        let mut nested: Vec<Parsed> = Vec::new();
        let mut paragraph: Lines = Vec::new();
        let mut apart = false;
        let flush = |paragraph: &mut Lines, nested: &mut Vec<Parsed>| {
            if !paragraph.is_empty() {
                nested.push(Parsed {
                    kind: Kind::Paragraph(std::mem::take(paragraph)),
                    quote: quoted,
                    nest: depth + 1,
                    margins: Margins { top: 6.0, bottom: 6.0 },
                });
            }
        };
        while i < lines.len() {
            let line = lines[i];
            if line.trim().is_empty() {
                // A blank line: the item goes on if more is indented under it
                let mut next = i;
                while next < lines.len() && lines[next].trim().is_empty() {
                    next += 1;
                }
                match lines.get(next) {
                    Some(after) if indent_of(after) > indent && list_item(after).is_none() => {
                        flush(&mut paragraph, &mut nested);
                        apart = true;
                        i = next;
                        continue;
                    }
                    _ => break,
                }
            }
            if list_item(line).is_some() || indent_of(line) <= indent {
                break;
            }
            if let Some(marker) = fence(line.trim_start()) {
                flush(&mut paragraph, &mut nested);
                apart = true;
                let cut = indent_of(line);
                let mut code = Vec::new();
                i += 1;
                while i < lines.len() && !lines[i].trim().starts_with(&marker) {
                    code.push(dedent(lines[i], cut));
                    i += 1;
                }
                i += 1; // the closing fence, if it has arrived
                nested.push(Parsed {
                    kind: Kind::Code(code.join("\n")),
                    quote: quoted,
                    nest: depth + 1,
                    margins: Margins { top: 6.0, bottom: 8.0 },
                });
                continue;
            }
            if apart {
                paragraph.push(inline(line.trim()));
            } else {
                body.push(inline(line.trim()));
            }
            i += 1;
        }
        flush(&mut paragraph, &mut nested);
        let marker = if ordered {
            format!("{}.", number)
        } else {
            // The browser's markers: disc, circle, square
            match depth {
                0 => "•",
                1 => "◦",
                _ => "▪",
            }
            .to_string()
        };
        number += 1;
        // The first item of a top-level list sits where the list starts; a
        // nested list and the next items keep 3px from the item above
        let top = if first_item && depth == 0 { 0.0 } else { 3.0 };
        first_item = false;
        push(
            out,
            Kind::ListItem {
                marker,
                depth,
                lines: body,
            },
            quoted,
            top,
            0.0,
        );
        out.extend(nested);

        // A blank line inside the list, if the list goes on after it
        if i < lines.len() && lines[i].trim().is_empty() {
            let mut next = i;
            while next < lines.len() && lines[next].trim().is_empty() {
                next += 1;
            }
            match lines.get(next).and_then(|line| list_item(line)) {
                Some(after) if after.indent >= indent => i = next,
                _ => break,
            }
        }
    }
    i
}

/// A table from line `start` (the header); returns the first line after it.
fn table(lines: &[&str], start: usize, quoted: usize, out: &mut Vec<Parsed>) -> usize {
    let cells = |line: &str| -> Vec<Vec<Inline>> { table_cells(line).iter().map(|cell| inline(cell)).collect() };
    push(
        out,
        Kind::TableRow {
            cells: cells(lines[start]),
            header: true,
        },
        quoted,
        0.0,
        0.0,
    );
    let mut i = start + 2;
    while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
        push(
            out,
            Kind::TableRow {
                cells: cells(lines[i]),
                header: false,
            },
            quoted,
            0.0,
            0.0,
        );
        i += 1;
    }
    if let Some(last) = out.last_mut() {
        last.margins.bottom = 8.0;
    }
    i
}

fn table_cells(line: &str) -> Vec<String> {
    let mut row = line.trim();
    row = row.strip_prefix('|').unwrap_or(row);
    row = row.strip_suffix('|').unwrap_or(row);
    row.split('|').map(|cell| cell.trim().to_string()).collect()
}

// ── Block starts ────────────────────────────────────────

/// Up to three spaces of indentation, and the rest
fn indented(line: &str) -> Option<&str> {
    let spaces = line.len() - line.trim_start_matches(' ').len();
    (spaces <= 3).then(|| &line[spaces..])
}

/// A code fence: ``` or ~~~ (three or more), maybe a language
fn fence(line: &str) -> Option<String> {
    let rest = indented(line)?;
    let fence_char = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = rest.chars().take_while(|c| *c == fence_char).count();
    if count < 3 {
        return None;
    }
    let info = rest[count..].trim();
    info.chars()
        .all(|c| c.is_alphanumeric() || "_+#.-".contains(c))
        .then(|| fence_char.to_string().repeat(count))
}

/// `#` to `######`, a space, the text (closing #s after a space go)
fn heading(line: &str) -> Option<(usize, &str)> {
    let rest = indented(line)?;
    let level = rest.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let after = &rest[level..];
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    let mut text = after.trim();
    let without_hashes = text.trim_end_matches('#');
    if without_hashes.is_empty() {
        text = "";
    } else if without_hashes.len() < text.len() && without_hashes.ends_with(char::is_whitespace) {
        text = without_hashes.trim_end();
    }
    Some((level, text))
}

/// Three or more of -, * or _, maybe with spaces between
fn is_rule(line: &str) -> bool {
    let Some(rest) = indented(line) else { return false };
    let Some(mark) = rest.chars().next().filter(|c| "-*_".contains(*c)) else {
        return false;
    };
    let mut count = 0;
    for c in rest.chars() {
        if c == mark {
            count += 1;
        } else if !c.is_whitespace() {
            return false;
        }
    }
    count >= 3
}

/// `>` and the quoted text
fn quote(line: &str) -> Option<&str> {
    let rest = indented(line)?.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

struct ListItem<'a> {
    /// Characters of indentation
    indent: usize,
    /// The number of an ordered item
    number: Option<u64>,
    rest: &'a str,
}

/// `- `, `* `, `+ `, `• ` or `1. ` / `1) ` and the item's text
fn list_item(line: &str) -> Option<ListItem<'_>> {
    let body = line.trim_start();
    let indent = line[..line.len() - body.len()].chars().count();
    let (number, after) = if let Some(rest) = body.strip_prefix(['-', '*', '+', '•']) {
        (None, rest)
    } else {
        let digits = body.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 3 {
            return None;
        }
        let rest = body[digits..].strip_prefix(['.', ')'])?;
        (body[..digits].parse().ok(), rest)
    };
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    Some(ListItem {
        indent,
        number,
        rest: after.trim_start(),
    })
}

/// `|---|:--:|` under a table's header
fn is_table_divider(line: &str) -> bool {
    let mut row = line.trim();
    row = row.strip_prefix('|').unwrap_or(row);
    row = row.strip_suffix('|').unwrap_or(row);
    !row.trim().is_empty()
        && row.split('|').all(|cell| {
            let cell = cell.trim();
            let cell = cell.strip_prefix(':').unwrap_or(cell);
            let cell = cell.strip_suffix(':').unwrap_or(cell);
            cell.len() >= 2 && cell.chars().all(|c| c == '-')
        })
}

fn is_block_start(line: &str) -> bool {
    fence(line).is_some() || heading(line).is_some() || is_rule(line) || quote(line).is_some() || list_item(line).is_some()
}

// ── Inline ──────────────────────────────────────────────

const ESCAPABLE: &str = "\\`*_~[]()#>|.!-";

/// Inline formatting of one line.
pub fn inline(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    inline_chars(&chars)
}

fn is_word(c: Option<&char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric())
}

fn find(text: &[char], marker: &[char], from: usize) -> Option<usize> {
    if marker.is_empty() || from > text.len() {
        return None;
    }
    (from..=text.len().saturating_sub(marker.len())).find(|&i| text[i..].starts_with(marker))
}

/// Index of `marker` closing an inline span that starts at `from`: it
/// follows text, not a space
fn closing(text: &[char], marker: &[char], from: usize) -> Option<usize> {
    let mut index = find(text, marker, from);
    while let Some(i) = index {
        if i > from && text[i - 1] != ' ' {
            return Some(i);
        }
        index = find(text, marker, i + 1);
    }
    None
}

/// `[label](href)` or `[label](href "title")` at the start of `text`:
/// the label, the address and how many characters it takes
fn link(text: &[char]) -> Option<(&[char], String, usize)> {
    if text.first() != Some(&'[') {
        return None;
    }
    let label_end = text.iter().position(|c| *c == ']')?;
    if label_end < 2 || text.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let href_start = label_end + 2;
    let mut i = href_start;
    while i < text.len() && text[i] != ')' && !text[i].is_whitespace() {
        i += 1;
    }
    if i == href_start {
        return None;
    }
    let href: String = text[href_start..i].iter().collect();
    // An optional title in quotes
    let mut j = i;
    while j < text.len() && text[j].is_whitespace() {
        j += 1;
    }
    if j > i && text.get(j) == Some(&'"') {
        let close = text[j + 1..].iter().position(|c| *c == '"')? + j + 1;
        j = close + 1;
    } else {
        j = i;
    }
    (text.get(j) == Some(&')')).then(|| (&text[1..label_end], href, j + 1))
}

fn inline_chars(text: &[char]) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<Inline>| {
        if !plain.is_empty() {
            out.push(Inline::Text(std::mem::take(plain)));
        }
    };

    let mut i = 0;
    while i < text.len() {
        let ch = text[i];
        let next = text.get(i + 1).copied();

        // An escaped character is plain text
        if ch == '\\' && next.is_some_and(|c| ESCAPABLE.contains(c)) {
            plain.push(next.unwrap());
            i += 2;
            continue;
        }

        if ch == '`' {
            if let Some(end) = find(text, &['`'], i + 1).filter(|end| *end > i + 1) {
                flush(&mut plain, &mut out);
                out.push(Inline::Code(text[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        }

        if i + 2 < text.len() && text[i + 2] != ' ' {
            let double = &text[i..i + 2];
            if double == ['*', '*'] || double == ['_', '_'] || double == ['~', '~'] {
                if let Some(end) = closing(text, double, i + 2) {
                    flush(&mut plain, &mut out);
                    let inner = inline_chars(&text[i + 2..end]);
                    out.push(if double[0] == '~' { Inline::Strike(inner) } else { Inline::Strong(inner) });
                    i = end + 2;
                    continue;
                }
            }
        }

        // _ inside a word (snake_case) is no emphasis
        let word_before = i > 0 && is_word(text.get(i - 1));
        if (ch == '*' || (ch == '_' && !word_before)) && next.is_some_and(|n| n != ' ' && n != ch) {
            if let Some(end) = closing(text, &[ch], i + 1) {
                let word_after = is_word(text.get(end + 1));
                if !(ch == '_' && word_after) {
                    flush(&mut plain, &mut out);
                    out.push(Inline::Emphasis(inline_chars(&text[i + 1..end])));
                    i = end + 1;
                    continue;
                }
            }
        }

        if ch == '[' {
            if let Some((label, href, length)) = link(&text[i..]) {
                flush(&mut plain, &mut out);
                out.push(Inline::Link {
                    href,
                    label: inline_chars(label),
                });
                i += length;
                continue;
            }
        }

        plain.push(ch);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

// ── Styled text ─────────────────────────────────────────

/// Colours the formatting adds (ARGB)
pub struct Ink {
    /// Bold text stands out
    pub strong: u32,
    /// Struck-out text recedes
    pub muted: u32,
}

/// Slint's styled text for `lines`; `bold` makes all of it bold.
pub fn styled(lines: &[Vec<Inline>], ink: &Ink, bold: bool) -> slint::StyledText {
    let paragraphs: slint::SharedVector<StyledTextParagraph> = lines
        .iter()
        .map(|line| {
            let mut paragraph = StyledTextParagraph {
                text: String::new(),
                formatting: Vec::new(),
                links: Vec::new(),
            };
            append(&mut paragraph, line, ink);
            if bold && !paragraph.text.is_empty() {
                paragraph.formatting.push(FormattedSpan {
                    range: 0..paragraph.text.len(),
                    style: Style::Strong,
                });
            }
            paragraph
        })
        .collect();
    // SAFETY: `slint::StyledText` is `#[repr(transparent)]` over exactly this
    // vector of paragraphs (i-slint-core 1.18.1, pinned in Cargo.toml; the
    // test below checks it against Slint's own Markdown parser). Building it
    // here keeps our parser's rules and never lets the model's text be read
    // as Markdown or HTML a second time.
    unsafe { std::mem::transmute::<slint::SharedVector<StyledTextParagraph>, slint::StyledText>(paragraphs) }
}

fn append(paragraph: &mut StyledTextParagraph, content: &[Inline], ink: &Ink) {
    for item in content {
        let start = paragraph.text.len();
        let styles: &[Style] = match item {
            Inline::Text(text) => {
                paragraph.text.push_str(text);
                continue;
            }
            Inline::Code(code) => {
                paragraph.text.push_str(code);
                &[Style::Code]
            }
            Inline::Strong(inner) => {
                append(paragraph, inner, ink);
                &[Style::Strong, Style::Color(ink.strong)]
            }
            Inline::Emphasis(inner) => {
                append(paragraph, inner, ink);
                &[Style::Emphasis]
            }
            Inline::Strike(inner) => {
                append(paragraph, inner, ink);
                &[Style::Strikethrough, Style::Color(ink.muted)]
            }
            Inline::Link { href, label } => {
                append(paragraph, label, ink);
                paragraph.links.push((start..paragraph.text.len(), href.clone()));
                &[Style::Link]
            }
        };
        let end = paragraph.text.len();
        if end > start {
            for style in styles {
                paragraph.formatting.push(FormattedSpan {
                    range: start..end,
                    style: style.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Inline {
        Inline::Text(t.to_string())
    }

    const INK: Ink = Ink {
        strong: 0xfff8f8f6,
        muted: 0xff9c9a92,
    };

    #[test]
    fn test_styled_text_matches_slints_own() {
        // Our paragraphs make the same StyledText as Slint's parser does
        let ours = styled(&[inline("plain *italic* and `code`")], &INK, false);
        let slints = slint::StyledText::from_markdown("plain *italic* and `code`").unwrap();
        assert_eq!(ours, slints);
        let ours = styled(&[inline("one"), inline("two")], &INK, false);
        assert_eq!(ours, slint::StyledText::from_markdown("one\\\ntwo").unwrap());
    }

    #[test]
    fn test_inline_formatting() {
        assert_eq!(
            inline("a **b** c"),
            vec![text("a "), Inline::Strong(vec![text("b")]), text(" c")]
        );
        assert_eq!(inline("snake_case_name"), vec![text("snake_case_name")]);
        assert_eq!(inline("2 * 3 * 4"), vec![text("2 * 3 * 4")]);
        assert_eq!(inline("\\*not\\*"), vec![text("*not*")]);
        assert_eq!(
            inline("see [Exa](https://exa.ai \"x\")."),
            vec![
                text("see "),
                Inline::Link {
                    href: "https://exa.ai".into(),
                    label: vec![text("Exa")]
                },
                text(".")
            ]
        );
        // Still being written: the stars show until the closing ones come
        assert_eq!(inline("**bol"), vec![text("**bol")]);
        assert_eq!(inline("`a` and ``"), vec![Inline::Code("a".into()), text(" and ``")]);
        assert_eq!(inline("ёлка _курсив_"), vec![text("ёлка "), Inline::Emphasis(vec![text("курсив")])]);
    }

    #[test]
    fn test_blocks_and_gaps() {
        let blocks = parse("# Title\n\nText\nmore\n\n- one\n- two\n  - nested\n- three\n\n```js\nlet a;\n```\n---\n> quoted");
        let kinds: Vec<&Kind> = blocks.iter().map(|b| &b.kind).collect();
        assert!(matches!(kinds[0], Kind::Heading(3, _)));
        assert!(matches!(kinds[1], Kind::Paragraph(lines) if lines.len() == 2));
        assert!(matches!(kinds[2], Kind::ListItem { marker, depth: 0, .. } if marker == "•"));
        assert!(matches!(kinds[4], Kind::ListItem { marker, depth: 1, .. } if marker == "◦"));
        assert!(matches!(kinds[5], Kind::ListItem { depth: 0, .. }));
        assert!(matches!(kinds[6], Kind::Code(code) if code == "let a;"));
        assert!(matches!(kinds[7], Kind::Rule));
        assert!(blocks[8].quoted && !blocks[8].joined);
        // Margins: nothing above the first, the heading's 6 below vs the
        // paragraph's 0 above, 3 between list items, 10 around the rule
        let gaps: Vec<f32> = blocks.iter().map(|b| b.gap).collect();
        assert_eq!(gaps, vec![0.0, 6.0, 8.0, 3.0, 3.0, 3.0, 8.0, 10.0, 10.0]);
    }

    #[test]
    fn test_code_blocks_inside_list_items() {
        let text = "Install it:\n\n- Debian / Ubuntu  \n  ```bash\n  sudo apt update\n  sudo apt install neofetch\n  ```\n\n- Arch\n  ```bash\n  sudo pacman -S neofetch\n  ```\n\nOr build it:\n\n```bash\nmake install\n```bash\n\nThen run it.";
        let blocks = parse(text);
        let kinds: Vec<&Kind> = blocks.iter().map(|b| &b.kind).collect();
        assert!(matches!(kinds[0], Kind::Paragraph(_)));
        assert!(matches!(kinds[1], Kind::ListItem { lines, .. } if lines.len() == 1));
        assert!(matches!(kinds[2], Kind::Code(code) if code == "sudo apt update\nsudo apt install neofetch"));
        assert_eq!(blocks[2].nest, 1);
        assert!(matches!(kinds[3], Kind::ListItem { .. }));
        assert!(matches!(kinds[4], Kind::Code(code) if code == "sudo pacman -S neofetch"));
        assert!(matches!(kinds[5], Kind::Paragraph(_)));
        // A closing fence with a language (a model's slip) still closes
        assert!(matches!(kinds[6], Kind::Code(code) if code == "make install"));
        assert_eq!(blocks[6].nest, 0);
        assert!(matches!(kinds[7], Kind::Paragraph(_)));
        assert_eq!(blocks.len(), 8);
    }

    #[test]
    fn test_text_after_a_blank_line_stays_in_the_item() {
        let blocks = parse("1. First\n\n   more about it\n2. Second");
        assert!(matches!(&blocks[1].kind, Kind::Paragraph(_)));
        assert_eq!(blocks[1].nest, 1);
        assert!(matches!(&blocks[2].kind, Kind::ListItem { marker, .. } if marker == "2."));
    }

    #[test]
    fn test_unclosed_fence_is_code_so_far() {
        let blocks = parse("Look:\n\n```py\nprint(1)");
        assert!(matches!(&blocks[1].kind, Kind::Code(code) if code == "print(1)"));
    }

    #[test]
    fn test_ordered_lists_count_on() {
        let blocks = parse("1. a\n1. b\n\n3) c");
        let markers: Vec<String> = blocks
            .iter()
            .filter_map(|b| match &b.kind {
                Kind::ListItem { marker, .. } => Some(marker.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(markers, vec!["1.", "2.", "3."]);
    }

    #[test]
    fn test_table() {
        let blocks = parse("| a | b |\n|---|:--:|\n| 1 | **2** |\nafter");
        assert!(matches!(&blocks[0].kind, Kind::TableRow { header: true, cells } if cells.len() == 2));
        assert!(matches!(&blocks[1].kind, Kind::TableRow { header: false, .. }));
        assert!(matches!(&blocks[2].kind, Kind::Paragraph(_)));
        assert_eq!(blocks[2].gap, 8.0);
    }

    #[test]
    fn test_headings_and_rules() {
        assert_eq!(heading("## C# tips"), Some((2, "C# tips")));
        assert_eq!(heading("### Title ###"), Some((3, "Title")));
        assert_eq!(heading("#hashtag"), None);
        assert!(is_rule("* * *"));
        assert!(!is_rule("**bold**"));
        assert!(list_item("**bold** text").is_none());
        assert!(list_item("10. ten").is_some());
    }
}
