//! Source-preserving edits and presentation metadata for Markdown shortcuts.
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::ops::Range;

#[derive(Debug)]
pub struct BlockLine {
    pub bytes: Range<usize>,
    pub prefix: Range<usize>,
    pub label: Option<String>,
    pub quote_depth: usize,
    pub indent: f64,
    pub list_depth: usize,
    pub is_list: bool,
}

/// A `---` line is a rule whatever precedes it. CommonMark reads it under a
/// paragraph line as that paragraph's heading underline; given such a heading's
/// range, return its last line when that line alone would be a rule.
pub fn rule_under_text(source: &str, heading: &Range<usize>) -> Option<Range<usize>> {
    let raw = source.get(heading.clone())?;
    let body = raw.trim_end_matches(['\r', '\n']);
    let start = body.rfind('\n')? + 1;
    let marks = body[start..].trim_matches([' ', '\t']);
    (marks.len() >= 3 && marks.bytes().all(|b| b == b'-')).then(|| heading.start + start..heading.end)
}

/// Every rule line, with its line ending.
pub fn rules(source: &str) -> Vec<Range<usize>> {
    Parser::new_ext(source, Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Rule => Some(range),
            Event::Start(Tag::Heading { .. }) => rule_under_text(source, &range),
            _ => None,
        })
        .collect()
}

/// A rule is one block: the caret never rests among its hidden marks. Returns
/// where a caret on a rule line belongs, the end of the line above when moving
/// backwards and otherwise the start of the line below.
pub fn rule_caret(source: &str, caret: usize, backwards: bool) -> Option<usize> {
    let rule = rules(source).into_iter().find(|rule| rule.start <= caret && caret < rule.end)?;
    let head = &source[..rule.start];
    let head = head.strip_suffix('\n').unwrap_or(head);
    let above = head.strip_suffix('\r').unwrap_or(head).len();
    let below = source[rule.clone()].ends_with('\n');
    if rule.start > 0 && (backwards || !below) {
        Some(above)
    } else {
        below.then_some(rule.end)
    }
}

/// Undo hands a restored rule back selected, and the highlight covers the
/// line. A selection that holds nothing but one rule becomes a caret below it.
pub fn rule_selection_caret(source: &str, selection: Range<usize>) -> Option<usize> {
    if selection.is_empty() {
        return rule_caret(source, selection.start, false);
    }
    let rule = rules(source).into_iter().find(|rule| rule.start <= selection.start && selection.end <= rule.end)?;
    Some(if source[rule.clone()].ends_with('\n') { rule.end } else { rule_caret(source, rule.start, false)? })
}

pub fn blocks(source: &str) -> Vec<BlockLine> {
    let excluded: Vec<_> = Parser::new_ext(source, Options::ENABLE_TABLES)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(
                event,
                Event::Start(Tag::CodeBlock(_) | Tag::Table(_)) | Event::Rule
            )
            .then_some(range)
        })
        .collect();
    let mut offset = 0;
    source
        .split_inclusive('\n')
        .filter_map(|raw| {
            let start = offset;
            offset += raw.len();
            if excluded.iter().any(|r| r.start < offset && start < r.end) {
                return None;
            }
            let line = raw.trim_end_matches(['\r', '\n']);
            let leading = line
                .bytes()
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count();
            let mut pos = leading;
            let mut quotes = 0;
            while line[pos..].starts_with("> ") {
                quotes += 1;
                pos += 2;
            }
            let rest = &line[pos..];
            let mut label = None;
            let mut list = false;
            if matches!(rest.as_bytes().first(), Some(b'-' | b'+' | b'*'))
                && rest.as_bytes().get(1) == Some(&b' ')
            {
                pos += 2;
                label = Some("•".into());
                list = true;
            } else {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                if (1..=9).contains(&digits)
                    && matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
                    && rest.as_bytes().get(digits + 1) == Some(&b' ')
                {
                    label = Some(format!("{}.", &rest[..digits]));
                    pos += digits + 2;
                    list = true;
                }
            }
            if !list && quotes == 0 {
                return None;
            }
            while line.as_bytes().get(pos) == Some(&b' ') {
                pos += 1;
            }
            if list
                && ["[ ] ", "[x] ", "[X] "]
                    .iter()
                    .any(|s| line[pos..].starts_with(s))
            {
                label = None;
                pos += 4;
            }
            let label_width = label
                .as_ref()
                .map_or(26.0, |s| (s.chars().count() as f64 * 10.0 + 4.0).max(24.0));
            Some(BlockLine {
                bytes: start..offset,
                prefix: start..start + pos,
                label,
                quote_depth: quotes,
                list_depth: leading / 2,
                is_list: list,
                indent: leading as f64 * 8.0
                    + quotes as f64 * 20.0
                    + if list { label_width } else { 4.0 },
            })
        })
        .collect()
}

pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}

pub fn wrap(source: &str, selection: Range<usize>, open: &str, close: &str) -> Option<Edit> {
    let content = source.get(selection.clone())?;
    if content.is_empty() || content.contains(['\r', '\n']) {
        return None;
    }
    // Accept both a selection of visible content and a selection including markers.
    let (range, body, adding) = if selection.start >= open.len()
        && source[..selection.start].ends_with(open)
        && source[selection.end..].starts_with(close)
    {
        (
            selection.start - open.len()..selection.end + close.len(),
            content,
            false,
        )
    } else if content.len() >= open.len() + close.len()
        && content.starts_with(open)
        && content.ends_with(close)
    {
        (
            selection.clone(),
            &content[open.len()..content.len() - close.len()],
            false,
        )
    } else {
        (selection.clone(), content, true)
    };
    let replacement = if adding {
        format!("{open}{body}{close}")
    } else {
        body.into()
    };
    let start = range.start + if adding { open.len() } else { 0 };
    Some(Edit {
        range,
        replacement,
        selection: start..start + body.len(),
    })
}

pub fn block_edit(source: &str, selection: Range<usize>, kind: u8) -> Option<Edit> {
    source.get(selection.clone())?;
    let start = source[..selection.start].rfind('\n').map_or(0, |n| n + 1);
    let tail = if selection.end > selection.start && source[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let end = source[tail..].find('\n').map_or(source.len(), |n| tail + n);
    let lines = blocks(source);
    let mut position = start;
    let mut output = Vec::new();
    for (index, raw) in source[start..end].split('\n').enumerate() {
        let content = if let Some(block) = lines.iter().find(|b| b.bytes.start == position) {
            &raw[block.prefix.end - position..]
        } else {
            let n = raw.bytes().take_while(|b| *b == b'#').count();
            if (1..=6).contains(&n) && raw.as_bytes().get(n) == Some(&b' ') {
                &raw[n + 1..]
            } else {
                raw
            }
        };
        let prefix = match kind {
            0 => String::new(),
            1..=3 => format!("{} ", "#".repeat(kind as usize)),
            4 => "- [ ] ".into(),
            5 => "- ".into(),
            6 => format!("{}. ", index + 1),
            8 => String::new(),
            _ => return None,
        };
        output.push(format!("{prefix}{content}"));
        position += raw.len() + 1;
    }
    let body = output.join("\n");
    let replacement = if kind == 8 {
        format!("```text\n{body}\n```")
    } else {
        body
    };
    let caret = if kind == 8 {
        start + 8
    } else {
        start + replacement.len()
    };
    Some(Edit {
        range: start..end,
        replacement,
        selection: caret..caret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_empty_markers_and_code_table_exclusions() {
        for source in ["- ", "* ", "+ ", "17. ", "> "] {
            let b = blocks(source);
            assert_eq!(b.len(), 1, "{source}");
            assert_eq!(b[0].prefix, 0..source.len());
            assert!(b[0].indent >= 24.0);
        }
        assert!(blocks("```\n- code\n> code\n```\n\n    - code\n\n---\n").is_empty());
        assert!(blocks("| a | b |\n| --- | --- |\n| c | d |\n").is_empty());
        assert_eq!(blocks("text\n12. ")[0].label.as_deref(), Some("12."));
        assert_eq!(blocks("> > quote")[0].quote_depth, 2);
    }
    #[test]
    fn rules_are_whole_blocks_for_the_caret() {
        let source = "본문\n---\n뒤";
        let rule = source.find("---").unwrap();
        assert_eq!(rules(source), vec![rule..rule + 4]);
        for caret in rule..rule + 4 {
            assert_eq!(rule_caret(source, caret, false), Some(rule + 4));
            assert_eq!(rule_caret(source, caret, true), Some(rule - 1));
        }
        assert_eq!(rule_caret(source, rule + 4, true), None);
        assert_eq!(rule_caret(source, rule - 1, false), None);
        // CRLF: the line above ends before its \r\n.
        assert_eq!(rule_caret("가\r\n\r\n***\r\n나", 7, true), Some(5));
        // First line: nothing above, so the caret stays below.
        assert_eq!(rule_caret("---\n뒤", 1, true), Some(4));
        // Last line without an ending: nothing below.
        assert_eq!(rule_caret("앞\n\n---", 6, false), Some(4));
        assert_eq!(rule_caret("---", 1, false), None);
        assert_eq!(rule_caret("```\n---\n```\n", 5, false), None);
        assert_eq!(rule_caret("본문\n--\n", 8, false), None);
        assert_eq!(rule_selection_caret(source, rule..rule + 4), Some(rule + 4));
        assert_eq!(rule_selection_caret(source, rule..rule + 3), Some(rule + 4));
        assert_eq!(rule_selection_caret(source, rule + 1..rule + 1), Some(rule + 4));
        assert_eq!(rule_selection_caret(source, rule - 1..rule + 4), None);
        assert_eq!(rule_selection_caret(source, rule..source.len()), None);
        assert_eq!(rule_selection_caret(source, 0..3), None);
    }

    #[test]
    fn unicode_toggle_and_multiple_lines() {
        let a = wrap("한글 😀", 0..11, "~~", "~~").unwrap();
        assert_eq!(a.replacement, "~~한글 😀~~");
        let b = wrap(&a.replacement, a.selection, "~~", "~~").unwrap();
        assert_eq!(b.replacement, "한글 😀");
        assert!(wrap("a\nb", 0..3, "**", "**").is_none());
        let e = block_edit("# 첫째\n- 둘째", 0.."# 첫째\n- 둘째".len(), 6).unwrap();
        assert_eq!(e.replacement, "1. 첫째\n2. 둘째");
    }
}
