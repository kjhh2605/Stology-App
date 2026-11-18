//! Source-based arrow shortcuts and structural selection. All offsets are UTF-8.
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::ops::Range;

/// Complete an isolated arrow token on Space. Never rewrite pasted text, code,
/// link destinations, escaped tokens, or a suffix of a longer operator.
pub fn arrow(source: &str, caret: usize) -> Option<(Range<usize>, &'static str)> {
    let before = source.get(..caret)?;
    let start = before
        .rfind(char::is_whitespace)
        .map_or(0, |n| n + before[n..].chars().next().unwrap().len_utf8());
    let replacement = match &source[start..caret] {
        "->" | "-->" => "→ ",
        "<-" | "<--" | "←-" => "← ",
        "<->" | "<-->" | "←>" => "↔ ",
        "=>" | "==>" => "⇒ ",
        "<=" | "<==" | "⇐=" => "⇐ ",
        "<=>" | "<==>" | "⇐>" => "⇔ ",
        _ => return None,
    };
    // Also covers unfinished backticks, links and HTML while they are typed.
    let line = before.rsplit('\n').next().unwrap_or("");
    if line.contains('`') || line.contains("](") || line.contains("<!--") {
        return None;
    }
    if Parser::new_ext(source, Options::ENABLE_TABLES)
        .into_offset_iter()
        .any(|(event, range)| {
            matches!(
                event,
                Event::Code(_)
                    | Event::Html(_)
                    | Event::InlineHtml(_)
                    | Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. })
            ) && range.start <= start
                && caret <= range.end
        })
    {
        return None;
    }
    Some((start..caret, replacement))
}

/// Convert a just-typed operator without consuming a following space. Accept
/// continuations of an already converted left arrow so <-> remains typeable.
pub fn typed_arrow(source: &str, caret: usize, input: &str) -> Option<(Range<usize>, &'static str)> {
    if !matches!(input, ">" | "-" | "=") { return None; }
    let mut candidate = source.to_owned();
    candidate.get(..caret)?;
    candidate.insert_str(caret, input);
    let (range, replacement) = arrow(&candidate, caret + input.len())?;
    Some((range.start..caret, replacement.trim_end()))
}

/// Commit a newly typed thematic break and leave an editable paragraph below.
pub fn typed_rule(source: &str, caret: usize, input: &str) -> Option<&'static str> {
    if input != "-" { return None; }
    let before = source.get(..caret)?;
    let start = before.rfind('\n').map_or(0, |n| n + 1);
    if &source[start..caret] != "--" { return None; }
    let after = source.get(caret..)?;
    if !after.is_empty() && !after.starts_with(['\r', '\n']) { return None; }
    let mut candidate = source.to_owned();
    candidate.insert(caret, '-');
    if !Parser::new_ext(&candidate, Options::ENABLE_TABLES).into_offset_iter()
        .any(|(event, range)| matches!(event, Event::Rule) && range.start == start && range.end >= caret + 1) {
        return None;
    }
    Some(if source.contains("\r\n") { "-\r\n" } else { "-\n" })
}

fn trimmed(source: &str, range: Range<usize>) -> Range<usize> {
    let raw = &source[range.clone()];
    let start = range.start + raw.len() - raw.trim_start().len();
    start..start + raw.trim().len()
}

/// Pick the smallest strictly enclosing range. `word` comes from AppKit's
/// Unicode-aware word selection, so composed characters are never split here.
pub fn expand(source: &str, selection: Range<usize>, word: Range<usize>) -> Option<Range<usize>> {
    source.get(selection.clone())?;
    if selection.is_empty()
        && !word.is_empty()
        && source.get(word.clone()).is_some()
        && word.start <= selection.start
        && selection.end <= word.end
    {
        return Some(word);
    }
    let mut candidates = vec![word, 0..source.len()];
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut escaped = false;
    for (offset, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        // Apostrophes within words are not paired quotation marks.
        if ch == '\''
            && source[..offset]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            && source[offset + 1..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        // Markdown backticks are parsed below; treating triple fences as a
        // quote would hide every pair of parentheses in their code body.
        let quote = matches!(ch, '\'' | '"');
        if quote && stack.last().is_some_and(|(open, _)| *open == ch) {
            let (_, start) = stack.pop().unwrap();
            candidates.push(start + ch.len_utf8()..offset);
            candidates.push(start..offset + ch.len_utf8());
        } else if stack
            .last()
            .is_some_and(|(open, _)| matches!(open, '\'' | '"'))
        {
            continue;
        } else if matches!(ch, '(' | '[' | '{') || quote {
            stack.push((ch, offset));
        } else if let Some(expected) = match ch {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        } {
            if stack.last().is_some_and(|(open, _)| *open == expected) {
                let (_, start) = stack.pop().unwrap();
                candidates.push(trimmed(source, start + 1..offset));
                candidates.push(start..offset + 1);
            }
        }
    }
    let mut start = 0;
    for line in source.split_inclusive('\n') {
        let end = start + line.len();
        candidates.push(trimmed(source, start..end));
        candidates.push(start..end);
        let mut sentence = start;
        for (index, ch) in line.char_indices() {
            let boundary = index + ch.len_utf8();
            if matches!(ch, '.' | '!' | '?' | '。' | '！' | '？')
                && (boundary == line.len() || line[boundary..].starts_with(char::is_whitespace))
            {
                candidates.push(trimmed(source, sentence..start + boundary));
                sentence = start + boundary;
            }
        }
        candidates.push(trimmed(source, sentence..end));
        start = end;
    }
    for (event, range) in Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    )
    .into_offset_iter()
    {
        if matches!(event, Event::Code(_)) {
            let raw = &source[range.clone()];
            let ticks = raw.bytes().take_while(|b| *b == b'`').count();
            if ticks > 0 && raw.len() >= ticks * 2 {
                candidates.push(trimmed(source, range.start + ticks..range.end - ticks));
            }
        }
        if matches!(event, Event::Start(_) | Event::Text(_) | Event::Code(_)) {
            candidates.push(trimmed(source, range.clone()));
            candidates.push(range);
        }
    }
    candidates
        .into_iter()
        .filter(|range| {
            source.get(range.clone()).is_some()
                && !range.is_empty()
                && range.start <= selection.start
                && selection.end <= range.end
                && *range != selection
        })
        .min_by_key(|range| (range.len(), range.start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_literal_contexts() {
        for (token, expected) in [
            ("->", "→ "),
            ("<-", "← "),
            ("<->", "↔ "),
            ("-->", "→ "),
            ("<-->", "↔ "),
            ("=>", "⇒ "),
            ("<=", "⇐ "),
            ("<=>", "⇔ "),
        ] {
            let source = format!("한글 😀 {token}");
            let (range, replacement) = arrow(&source, source.len()).unwrap();
            assert_eq!(&source[range], token);
            assert_eq!(replacement, expected);
        }
        for source in [
            "word->",
            "\\->",
            "--->",
            "` ->",
            "` -> `",
            "```rust\n->",
            "~~~\n->",
            "    ->",
            "[link](https://example.com/ ->",
            "<!-- ->",
        ] {
            assert!(arrow(source, source.len()).is_none(), "{source}");
        }
        assert!(arrow("한글", 1).is_none());
    }

    #[test]
    fn immediate_arrows_allow_bidirectional_continuation() {
        for (keys, expected) in [("->", "→"), ("<->", "↔"), ("<-->", "↔"), ("<=>", "⇔"), ("-->", "→")] {
            let mut source = String::from("한글 😀 ");
            for key in keys.chars() {
                if let Some((range, replacement)) = typed_arrow(&source, source.len(), &key.to_string()) {
                    source.replace_range(range, replacement);
                } else { source.push(key); }
            }
            assert_eq!(source, format!("한글 😀 {expected}"));
        }
        for source in ["word-", "`-", "```rust\n-", "\\-"] {
            assert!(typed_arrow(source, source.len(), ">").is_none());
        }
    }

    #[test]
    fn typed_rule_moves_into_a_new_paragraph_only_for_a_thematic_break() {
        for source in ["--", "한글 😀\n\n--"] {
            let insert = typed_rule(source, source.len(), "-").unwrap();
            assert_eq!(format!("{source}{insert}text"), format!("{source}-\ntext"));
        }
        for source in ["```\n--", "    --", "paragraph\n--", "`--", "word--"] {
            assert!(typed_rule(source, source.len(), "-").is_none());
        }
        assert!(typed_rule("--tail", 2, "-").is_none());
        assert!(typed_rule("--", 2, "---").is_none());
        assert_eq!(typed_rule("a\r\n\r\n--", 7, "-"), Some("-\r\n"));
    }

    #[test]
    fn grows_word_brackets_sentence_paragraph_document() {
        let source = "Say (hello world). Next sentence.\nSame paragraph.\n\nLast.";
        let mut selection = 7..7;
        let word = 5..10;
        for expected in [
            "hello",
            "hello world",
            "(hello world)",
            "Say (hello world).",
            "Say (hello world). Next sentence.",
            "Say (hello world). Next sentence.\n",
            "Say (hello world). Next sentence.\nSame paragraph.",
            "Say (hello world). Next sentence.\nSame paragraph.\n",
            source,
        ] {
            selection = expand(source, selection, word.clone()).unwrap();
            assert_eq!(&source[selection.clone()], expected);
        }
        assert_eq!(expand(source, selection, word), None);
    }

    #[test]
    fn arbitrary_selection_unicode_and_empty_edges() {
        let source = "한글 😀 (안녕 세상)\n\n끝";
        let start = source.find("안녕").unwrap();
        assert_eq!(
            expand(source, start..start, start..start + "안녕".len()),
            Some(start..start + "안녕".len())
        );
        let selected = start..start + "안녕 세".len();
        let next = expand(source, selected.clone(), selected).unwrap();
        assert_eq!(&source[next], "안녕 세상");
        assert!(expand(source, 1..1, 0..0).is_none());
        assert!(expand("", 0..0, 0..0).is_none());
        assert_eq!(expand("abc", 3..3, 0..3), Some(0..3));
    }

    #[test]
    fn code_fences_keep_nested_pairs_and_inline_code_content() {
        for source in ["```rust\ncall(alpha, beta);\n```", "`call(alpha, beta)`"] {
            let start = source.find("alpha").unwrap();
            let next = expand(source, start..start + 5, start..start + 5).unwrap();
            assert_eq!(&source[next], "alpha, beta");
        }
        let source = "Use `two words` here";
        assert_eq!(expand(source, 5..8, 5..8), Some(5..14));
    }
}
