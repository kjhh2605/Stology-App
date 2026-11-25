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
