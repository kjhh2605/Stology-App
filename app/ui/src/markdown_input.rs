//! Markdown source edits for native rich-editor keyboard commands.
//!
//! These are pure transformations: callers guard marked text/selection and apply
//! exactly one native replacement, keeping undo and the Markdown source intact.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Enter,
    ShiftEnter,
    Tab,
    BackTab,
    Backspace,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    /// Absolute UTF-8 byte offset in the resulting source.
    pub caret: usize,
}

fn leading_indent(line: &str) -> usize {
    line.bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count()
}

/// Prefix changes for an item and its contiguous children, in source coordinates.
fn item_indents(
    source: &str,
    start: usize,
    outdent: bool,
) -> impl Iterator<Item = (usize, isize)> + '_ {
    let line = source[start..].split('\n').next().unwrap_or("");
    let indent = leading_indent(line);
    let shift = if outdent {
        -(if line.starts_with('\t') {
            1
        } else {
            indent.min(2)
        } as isize)
    } else {
        // Without a preceding parent, deeper indentation becomes literal code.
        let previous = source[..start].lines().rev().find(|s| !s.trim().is_empty());
        let limit = previous
            .and_then(|s| {
                list_or_quote_prefix(s)
                    .filter(|p| !p.quote)
                    .map(|_| leading_indent(s) + 2)
            })
            .unwrap_or(2);
        if indent + 2 <= limit {
            2
        } else {
            0
        }
    };
    source[start..]
        .split_inclusive('\n')
        .enumerate()
        .take_while(move |(i, raw)| {
            *i == 0 || (!raw.trim().is_empty() && leading_indent(raw) > indent)
        })
        .scan(start, move |offset, (_, raw)| {
            let at = *offset;
            *offset += raw.len();
            Some((at, shift))
        })
}

/// Apply the prefix changes as one undoable edit. Rebase both selection ends
/// over inserted spaces or the portion of a removed prefix preceding the end.
fn indent_edit(
    source: &str,
    selection: Range<usize>,
    changes: impl IntoIterator<Item = (usize, isize)>,
) -> super::markdown_shortcuts::Edit {
    let mut start = None;
    let mut end = selection.start;
    let mut replacement = String::new();
    let mut after = selection.clone();
    for (at, shift) in changes {
        if shift == 0 {
            continue;
        }
        if start.is_some() {
            replacement.push_str(&source[end..at]);
        }
        start.get_or_insert(at);
        let remove = (-shift).max(0) as usize;
        end = at + remove;
        if shift > 0 {
            replacement.push_str("  ");
        }
        let delta = |pos: usize| {
            if pos < at {
                0
            } else if shift > 0 {
                shift
            } else {
                -(remove.min(pos - at) as isize)
            }
        };
        after.start = after.start.saturating_add_signed(delta(selection.start));
        // A selection ending at the next line start excludes that line's prefix.
        if selection.is_empty() || selection.end != at || shift < 0 {
            after.end = after.end.saturating_add_signed(delta(selection.end));
        }
    }
    super::markdown_shortcuts::Edit {
        range: start.unwrap_or(end)..end,
        replacement,
        selection: after,
    }
}

fn indent_item(source: &str, start: usize, caret: usize, outdent: bool) -> Edit {
    let edit = indent_edit(source, caret..caret, item_indents(source, start, outdent));
    Edit {
        range: edit.range,
        replacement: edit.replacement,
        caret: edit.selection.start,
    }
}

/// Only displayed numbered items participate. A selected parent's children
/// move with it, including children outside the selection, exactly once.
pub fn indent_selected_numbers(
    source: &str,
    selection: Range<usize>,
    outdent: bool,
) -> Option<super::markdown_shortcuts::Edit> {
    let mut changes = Vec::new();
    for block in super::markdown_shortcuts::blocks(source) {
        let at = block.bytes.start;
        if at >= selection.end {
            break;
        }
        if block.bytes.end <= selection.start
            || block.quote_depth != 0
            || !source[block.prefix]
                .trim_start()
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit)
            || changes.last().is_some_and(|&(last, _)| at <= last)
        {
            continue;
        }
        changes.extend(item_indents(source, at, outdent));
    }
    if changes.is_empty() {
        return None;
    }
    Some(indent_edit(source, selection, changes))
}

#[derive(Debug)]
struct Fence {
    bytes: Range<usize>,
    opening_end: usize,
    body: Range<usize>,
    closing: Option<Range<usize>>,
    marker: String,
}

fn line_without_ending(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

fn opening(line: &str) -> Option<(&str, &str)> {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let first = *rest.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let count = rest.bytes().take_while(|b| *b == first).count();
    if count < 3 || (first == b'`' && rest[count..].contains('`')) {
        return None;
    }
    Some((&rest[..count], rest[count..].trim()))
}

fn is_closing(line: &str, marker: &str) -> bool {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let count = rest
        .bytes()
        .take_while(|b| *b == marker.as_bytes()[0])
        .count();
    count >= marker.len() && rest[count..].trim().is_empty()
}

fn fences(source: &str) -> Vec<Fence> {
    let mut result = Vec::new();
    let mut current: Option<Fence> = None;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let raw = line_without_ending(line);
        if let Some(block) = current.as_mut() {
            if is_closing(raw, &block.marker) {
                let mut block = current.take().unwrap();
                block.bytes.end = offset + line.len();
                block.body.end = offset;
                // The newline immediately before a closing fence is Markdown
                // structure, not the code block's editable final newline.
                if block.body.end > block.body.start && source[..block.body.end].ends_with('\n') {
                    block.body.end -= 1;
                    if source[..block.body.end].ends_with('\r') {
                        block.body.end -= 1;
                    }
                }
                block.closing = Some(offset..offset + line.len());
                result.push(block);
            }
        } else if let Some((marker, _)) = opening(raw) {
            current = Some(Fence {
                bytes: offset..source.len(),
                opening_end: offset + raw.len(),
                body: offset + line.len()..source.len(),
                closing: None,
                marker: marker.to_string(),
            });
        }
        offset += line.len();
    }
    if let Some(block) = current {
        result.push(block);
    }
    result
}

/// Complete a typed fence before existing following text can be parsed as code.
pub fn typed_fence(source: &str, caret: usize, typed: &str) -> Option<Edit> {
    if typed != "`" || caret > source.len() || !source.is_char_boundary(caret) { return None; }
    let start = source[..caret].rfind('\n').map_or(0, |n| n + 1);
    if &source[start..caret] != "``" || !source[caret..].starts_with('\n') { return None; }
    if fences(&source[..start]).iter().any(|f| f.closing.is_none()) { return None; }
    let ending = newline(source);
    Some(Edit { range: caret..caret, replacement: format!("`{ending}{ending}```"), caret: caret + 1 })
}

fn language_id(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "js" | "javascript" => "javascript".into(),
        "ts" | "typescript" => "typescript".into(),
        "py" | "python" => "python".into(),
        "rs" | "rust" => "rust".into(),
        "sh" | "shell" | "bash" => "bash".into(),
        "java" => "java".into(),
        "c#" | "cs" | "csharp" => "csharp".into(),
        "c++" | "cpp" => "cpp".into(),
        "plaintext" | "plain" | "txt" | "text" | "" => "text".into(),
        _ => value.into(),
    }
}

fn insert(caret: usize, value: &str) -> Edit {
    Edit {
        range: caret..caret,
        replacement: value.into(),
        caret: caret + value.len(),
    }
}

fn newline(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn next_content(source: &str, start: usize) -> Option<usize> {
    let mut position = start;
    for line in source[start..].split_inclusive('\n') {
        if !line.trim().is_empty() {
            return Some(position);
        }
        position += line.len();
    }
    None
}

/// Clamp a caret produced by native movement out of concealed code fences.
/// Inputs and outputs are UTF-8 byte offsets. Call after native movement; `true`
/// means left/up and `false` means right/down. Editable body boundaries are
/// valid positions. At a document edge the caret stays in the code body.
pub fn fenced_caret(source: &str, caret: usize, backwards: bool) -> Option<usize> {
    if caret > source.len() || !source.is_char_boundary(caret) {
        return None;
    }
    for block in fences(source) {
        // No committed newline: the user is still typing the fence shortcut.
        if block.opening_end == block.body.start {
            continue;
        }
        if block.bytes.start <= caret && caret < block.body.start {
            let previous = source[..block.bytes.start].trim_end_matches(['\r', '\n']);
            return Some(if backwards && !previous.is_empty() {
                previous.len()
            } else {
                block.body.start
            });
        }
        if block.closing.is_some() && block.body.end < caret && caret <= block.bytes.end {
            if caret == block.bytes.end && source[..caret].ends_with('\n') {
                return None;
            }
            return Some(if backwards {
                block.body.end
            } else {
                next_content(source, block.bytes.end).unwrap_or(block.body.end)
            });
        }
    }
    None
}

fn exit_code(source: &str, block: &Fence, trim: usize, force_new: bool) -> Edit {
    let remove_start = block.body.end - trim;
    let ending = newline(source);
    if let Some(closing) = &block.closing {
        if !force_new {
            if let Some(next) = next_content(source, closing.end) {
                return Edit {
                    range: remove_start..block.body.end,
                    replacement: String::new(),
                    caret: next - trim,
                };
            }
        }
        let mut replacement = source[block.body.end..block.bytes.end].to_string();
        if !replacement.ends_with('\n') {
            replacement.push_str(ending);
        }
        replacement.push_str(ending);
        let caret = remove_start + replacement.len();
        // When a next block exists, put a fresh empty paragraph before it.
        if force_new && block.bytes.end < source.len() {
            replacement.push_str(ending);
        }
        Edit {
            range: remove_start..block.bytes.end,
            replacement,
            caret,
        }
    } else {
        let replacement = format!("{ending}{}{ending}{ending}", block.marker);
        Edit {
            range: remove_start..block.body.end,
            caret: remove_start + replacement.len(),
            replacement,
        }
    }
}

struct Prefix {
    end: usize,
    next: String,
    quote: bool,
}
fn list_or_quote_prefix(line: &str) -> Option<Prefix> {
    let indent = line
        .bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count();
    let rest = &line[indent..];
    let mut length;
    let mut next;
    let mut quote = false;
    if rest.starts_with('>') && matches!(rest.as_bytes().get(1), Some(b' ' | b'\t')) {
        length = 2;
        while rest[length..].starts_with("> ") {
            length += 2;
        }
        next = rest[..length].to_string();
        quote = true;
    } else if matches!(rest.as_bytes().first(), Some(b'-' | b'+' | b'*'))
        && matches!(rest.as_bytes().get(1), Some(b' ' | b'\t'))
    {
        length = 2;
        next = rest[..length].to_string();
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0
            || digits > 9
            || !matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
            || !matches!(rest.as_bytes().get(digits + 1), Some(b' ' | b'\t'))
        {
            return None;
        }
        let number: u64 = rest[..digits].parse().ok()?;
        length = digits + 2;
        next = format!("{}{} ", number + 1, rest.as_bytes()[digits] as char);
    }
    while matches!(rest.as_bytes().get(length), Some(b' ' | b'\t')) {
        length += 1;
    }
    if rest[length..].starts_with("[ ] ")
        || rest[length..].starts_with("[x] ")
        || rest[length..].starts_with("[X] ")
    {
        length += 4;
        next.push_str("[ ] ");
    }
    Some(Prefix {
        end: indent + length,
        next: format!("{}{next}", &line[..indent]),
        quote,
    })
}

/// Resolve the structural newlines emitted by block insertion along with the
/// block. Additional blank paragraphs belong to the writer.
fn preceding_block(source: &str, caret: usize) -> Option<Range<usize>> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        let Some(gap) = source.get(range.end..caret) else { continue };
        let raw = &source[range.clone()];
        let adjacent = match event {
            Event::Start(Tag::Table(_)) => gap.is_empty()
                || super::table_separator(source, range.end).is_some_and(|r| r.end == caret),
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                let Some((first, rest)) = raw.split_once('\n') else { continue };
                let Some((marker, _)) = opening(first) else { continue };
                rest.lines().last().is_some_and(|last| is_closing(last, marker))
                    && (gap.is_empty() || (!raw.ends_with('\n') && matches!(gap, "\n" | "\r\n")))
            }
            Event::Start(Tag::Image { dest_url, .. }) if super::image_edit::is_diagram_link(&dest_url) => {
                gap.is_empty() || (source[..range.start].rsplit('\n').next().unwrap_or("").trim().is_empty()
                    && matches!(gap, "\n" | "\r\n" | "\n\n" | "\r\n\r\n"))
            }
            _ => false,
        };
        if adjacent { return Some(range.start..caret); }
    }
    None
}

/// Return one source edit for a collapsed selection, or let native text editing
/// handle the command. Invalid offsets are rejected instead of panicking.
pub fn command(source: &str, caret: usize, command: Command) -> Option<Edit> {
    if caret > source.len() || !source.is_char_boundary(caret) {
        return None;
    }
    if command == Command::Backspace {
        if let Some(range) = preceding_block(source, caret) {
            return Some(Edit { caret: range.start, range, replacement: String::new() });
        }
    }
    let ending = newline(source);
    for block in fences(source) {
        // The fence becomes a code block only once Enter commits the shortcut.
        if command == Command::Enter && caret == block.opening_end && (block.closing.is_none() || block.closing.as_ref().is_some_and(|c| block.body.start == block.body.end && c.start == block.body.start + ending.len())) {
            if block.closing.is_some() {
                return Some(Edit { range: caret..caret, replacement: String::new(), caret: block.body.start });
            }
            let raw = &source[block.bytes.start..block.opening_end];
            let (marker, language) = opening(raw)?;
            let replacement = format!(
                "{}{}{ending}{ending}{marker}",
                marker,
                language_id(language)
            );
            let caret =
                block.bytes.start + marker.len() + language_id(language).len() + ending.len();
            return Some(Edit {
                range: block.bytes.start..block.opening_end,
                replacement,
                caret,
            });
        }
        if block.body.start <= caret && caret <= block.body.end {
            return match command {
                Command::BackTab => None,
                Command::Tab => {
                    if block
                        .closing
                        .as_ref()
                        .is_some_and(|c| c.start == block.body.start)
                    {
                        // Imported empty fences may have no intervening body
                        // line. Keep the closing fence on its own line.
                        let mut edit = insert(caret, &format!("  {ending}"));
                        edit.caret = caret + 2;
                        Some(edit)
                    } else {
                        Some(insert(caret, "  "))
                    }
                }
                Command::ShiftEnter => Some(exit_code(source, &block, 0, true)),
                Command::Enter => {
                    let content = &source[block.body.clone()];
                    let double = format!("{ending}{ending}");
                    if caret == block.body.end && content.ends_with(&double) {
                        Some(exit_code(source, &block, double.len(), false))
                    } else {
                        Some(insert(caret, ending))
                    }
                }
                Command::Backspace => {
                    if caret == block.body.start && block.body.is_empty() {
                        // Empty code block -> paragraph, with a single undoable edit.
                        Some(Edit {
                            range: block.bytes.clone(),
                            replacement: String::new(),
                            caret: block.bytes.start,
                        })
                    } else if caret == block.body.start {
                        // The opening newline is structure. Deleting it would
                        // concatenate the first code token into the language ID.
                        Some(Edit {
                            range: caret..caret,
                            replacement: String::new(),
                            caret,
                        })
                    } else {
                        None
                    }
                }
            };
        }
        // Never interpret code text as list or quote syntax, including fences.
        if block.bytes.start <= caret && caret < block.bytes.end {
            return None;
        }
    }
    let start = source[..caret].rfind('\n').map(|n| n + 1).unwrap_or(0);
    let end = source[caret..]
        .find('\n')
        .map(|n| caret + n)
        .unwrap_or(source.len());
    let line = line_without_ending(&source[start..end]);
    if command == Command::Enter {
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        let hashes = line[indent..].bytes().take_while(|b| *b == b'#').count();
        let hash_end = indent + hashes;
        if indent <= 3
            && (1..=6).contains(&hashes)
            && matches!(line.as_bytes().get(hash_end), Some(b' ' | b'\t'))
        {
            let spaces = line[hash_end..]
                .bytes()
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count();
            let prefix_end = hash_end + spaces;
            if caret == start + prefix_end && prefix_end < line.len() {
                return Some(Edit {
                    range: start..start,
                    replacement: ending.into(),
                    caret: caret + ending.len(),
                });
            }
        }
    }
    let prefix = list_or_quote_prefix(line)?;
    if caret < start + prefix.end {
        return None;
    }
    let indent = leading_indent(line);
    if !prefix.quote && matches!(command, Command::Tab | Command::BackTab) {
        let literal_code =
            pulldown_cmark::Parser::new(source)
                .into_offset_iter()
                .any(|(event, range)| {
                    matches!(
                        event,
                        pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
                    ) && range.start < end
                        && start < range.end
                });
        if !literal_code {
            return Some(indent_item(
                source,
                start,
                caret,
                command == Command::BackTab,
            ));
        }
        return None;
    }
    if command == Command::Enter
        && !prefix.quote
        && indent > 0
        && line[prefix.end..].trim().is_empty()
    {
        return Some(indent_item(source, start, caret, true));
    }
    match command {
        Command::Backspace if caret == start + prefix.end => Some(Edit {
            range: start..start + prefix.end,
            replacement: String::new(),
            caret: start,
        }),
        Command::Enter if line[prefix.end..].trim().is_empty() => Some(Edit {
            range: start..start + line.len(),
            replacement: String::new(),
            caret: start,
        }),
        // BlockNote's quote uses the default split-to-paragraph behavior.
        // Markdown needs a blank separator to avoid lazy quote continuation.
        Command::Enter if prefix.quote => Some(insert(caret, &format!("{ending}{ending}"))),
        Command::ShiftEnter if prefix.quote => {
            Some(insert(caret, &format!("{ending}{}", prefix.next)))
        }
        Command::Enter => Some(insert(caret, &format!("{ending}{}", prefix.next))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backspace_after_table_deletes_only_the_table() {
        for ending in ["\n", "\r\n"] {
            let table = format!("| 이름 | 값 |{ending}| --- | --- |{ending}| 한글 | 😀 |{ending}");
            let source = format!("앞{ending}{ending}{table}{ending}뒤");
            let caret = source.find("뒤").unwrap() - ending.len();
            let (result, after) = apply(&source, caret, Command::Backspace);
            assert_eq!(result, format!("앞{ending}{ending}{ending}뒤"));
            assert_eq!(after, format!("앞{ending}{ending}").len());
            assert!(command(&source, source.len(), Command::Backspace).is_none());
        }
        let table = "| A | B |\n| - | - |\n| 1 | 2 |";
        assert_eq!(apply(table, table.len(), Command::Backspace), (String::new(), 0));
        let code = "```\n| A | B |\n| - | - |\n```\n";
        assert_eq!(apply(code, code.len(), Command::Backspace), (String::new(), 0));
    }
    #[test]
    fn backspace_at_attachment_boundaries() {
        let link = "![회로](<../.diagram/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.svg>)";
        for ending in ["\n", "\r\n"] {
            for block in [format!("{link}{ending}{ending}"), format!("~~~rs{ending}😀{ending}~~~{ending}"),
                format!("| 열 |{ending}| --- |{ending}| 값 |{ending}{ending}")] {
                let prefix = format!("앞{ending}{ending}");
                let source = format!("{prefix}{block}뒤");
                assert_eq!(apply(&source, prefix.len()+block.len(), Command::Backspace),
                    (format!("{prefix}뒤"), prefix.len()));
            }
        }
        for text in [format!("`{link}`"), format!("    {link}"), "~~~rs\n미완성".into(),
            "![이미지](pic.png)\n".into(), format!("{link}\n\n\n")] {
            assert!(command(&text, text.len(), Command::Backspace).is_none());
        }
    }
    #[test]
    fn bullet_tab_enter_outdent_exit_sequence() {
        for marker in ["-", "*", "+"] {
            let source = format!("{marker} 부모\n{marker} 자식 😀");
            let (nested, caret) = apply(&source, source.len(), Command::Tab);
            assert_eq!(nested, format!("{marker} 부모\n  {marker} 자식 😀"));
            let (empty, caret) = apply(&nested, caret, Command::Enter);
            assert_eq!(empty, format!("{nested}\n  {marker} "));
            let (parent, caret) = apply(&empty, caret, Command::Enter);
            assert_eq!(parent, format!("{nested}\n{marker} "));
            let (plain, caret) = apply(&parent, caret, Command::Enter);
            assert_eq!(plain, format!("{nested}\n"));
            assert_eq!(caret, plain.len());
        }
    }
    #[test]
    fn bullet_indent_moves_children_and_preserves_midline_caret() {
        let source = "- 부모\r\n- 자식 😀\r\n  - 하위\r\n- 다음";
        let caret = source.find('😀').unwrap();
        let (nested, after) = apply(source, caret, Command::Tab);
        assert_eq!(nested, "- 부모\r\n  - 자식 😀\r\n    - 하위\r\n- 다음");
        assert_eq!(&nested[after..after + 4], "😀");
        let (restored, after) = apply(&nested, after, Command::BackTab);
        assert_eq!(restored, source);
        assert_eq!(after, caret);
        assert_eq!(apply("- ", 2, Command::BackTab), ("- ".into(), 2));
        let (first, caret) = apply("- ", 2, Command::Tab);
        assert_eq!(first, "  - ");
        assert_eq!(apply(&first, caret, Command::Tab), (first, caret));
        assert!(command("```\n- code\n```", 10, Command::BackTab).is_none());
        assert!(command("    - literal", 13, Command::Tab).is_none());
    }
    #[test]
    fn numbered_tab_preserves_partial_selection_and_moves_each_item_once() {
        for newline in ["\n", "\r\n"] {
            let source = ["1. 부모", "2. 앞 한글😀 뒤", "   - 하위", "3) 마지막", "4. 다음"].join(newline);
            let range = source.find("한글").unwrap()..source.find("4.").unwrap();
            let edit = indent_selected_numbers(&source, range.clone(), false).unwrap();
            let mut result = source.clone();
            result.replace_range(edit.range, &edit.replacement);
            assert_eq!(result, ["1. 부모", "  2. 앞 한글😀 뒤", "     - 하위", "  3) 마지막", "4. 다음"].join(newline));
            assert_eq!(edit.selection, result.find("한글").unwrap()..result.find("4.").unwrap());
            let edit = indent_selected_numbers(&result, edit.selection, true).unwrap();
            result.replace_range(edit.range, &edit.replacement);
            assert_eq!((result, edit.selection), (source, range));
        }
    }

    #[test]
    fn numbered_tab_keeps_text_selected_at_indent_limit_and_skips_literal_code() {
        let source = "  1. 앞😀뒤\n     - 하위";
        let start = source.find('😀').unwrap();
        let selection = start..start + '😀'.len_utf8();
        let edit = indent_selected_numbers(source, selection.clone(), false).unwrap();
        assert!(edit.range.is_empty() && edit.replacement.is_empty());
        assert_eq!(edit.selection, selection);
        let edit = indent_selected_numbers(source, selection, true).unwrap();
        let mut result = source.to_owned();
        result.replace_range(edit.range, &edit.replacement);
        assert_eq!(result, "1. 앞😀뒤\n   - 하위");
        assert_eq!(&result[edit.selection], "😀");
        for source in ["```\n1. 예시\n```", "    1. 예시", "일반 문단", "- 불릿"] {
            assert!(indent_selected_numbers(source, 0..source.len(), false).is_none());
        }
    }

    fn apply(source: &str, caret: usize, key: Command) -> (String, usize) {
        let edit = command(source, caret, key).unwrap_or_else(|| panic!("unhandled {key:?} at {caret} in {source:?}"));
        assert!(
            source.is_char_boundary(edit.range.start) && source.is_char_boundary(edit.range.end)
        );
        let mut result = source.to_string();
        result.replace_range(edit.range, &edit.replacement);
        assert!(result.is_char_boundary(edit.caret));
        (result, edit.caret)
    }
    #[test]
    fn typed_fence_protects_following_paragraph() {
        let source = "앞\n``\n뒤 텍스트";
        let at = "앞\n``".len();
        let edit = typed_fence(source, at, "`").unwrap();
        let mut result = source.to_string();
        result.replace_range(edit.range, &edit.replacement);
        assert_eq!(result, "앞\n```\n\n```\n뒤 텍스트");
        let (result, caret) = apply(&result, edit.caret, Command::Enter);
        assert_eq!(&result[caret..], "\n```\n뒤 텍스트");
        assert!(typed_fence("```\n``\ncode", 6, "`").is_none());
    }

    #[test]
    fn fence_shortcut_creates_empty_body_and_normalizes_alias() {
        let (text, caret) = apply("```js", 5, Command::Enter);
        assert_eq!(text, "```javascript\n\n```");
        assert_eq!(&text[..caret], "```javascript\n");
        let (text, _) = apply("```my-Dsl", 9, Command::Enter);
        assert_eq!(text, "```my-Dsl\n\n```");
        let source = "앞\n\n```java\n\n뒤";
        let caret = source.find("java").unwrap() + 4;
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "앞\n\n```java\n\n```\n\n뒤");
        assert!(text[caret..].starts_with("\n```"));
    }
    #[test]
    fn code_tab_and_enter_do_not_create_markdown_lists() {
        let source = "```rust\n- literal\n```";
        let caret = source.find("\n```").unwrap();
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "```rust\n- literal\n\n```");
        let (text, _) = apply(&text, caret, Command::Tab);
        assert_eq!(text, "```rust\n- literal\n  \n```");
    }
    #[test]
    fn double_newline_exit_preserves_fence_and_moves_to_existing_paragraph() {
        let source = "```java\nhello\n\n\n```\n\n다음 문단";
        let caret = source.find("\n```").unwrap();
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "```java\nhello\n```\n\n다음 문단");
        assert_eq!(&text[caret..], "다음 문단");
    }
    #[test]
    fn double_newline_exit_creates_paragraph_at_eof() {
        let source = "```\na\n\n\n```";
        let caret = source.find("\n```").unwrap();
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "```\na\n```\n\n");
        assert_eq!(caret, text.len());
    }
    #[test]
    fn shift_enter_exits_from_middle_without_deleting_code() {
        let source = "```python\nprint('한글 😀')\n```";
        let caret = source.find("한글").unwrap();
        let (text, caret) = apply(source, caret, Command::ShiftEnter);
        assert_eq!(text, format!("{source}\n\n"));
        assert_eq!(caret, text.len());
    }
    #[test]
    fn unclosed_fence_can_exit_and_close_safely() {
        let source = "~~~rust\nfn main() {}\n\n";
        let (text, caret) = apply(source, source.len(), Command::Enter);
        assert_eq!(text, "~~~rust\nfn main() {}\n~~~\n\n");
        assert_eq!(caret, text.len());
    }
    #[test]
    fn ordered_task_and_nested_list_continue() {
        for (source, expected) in [
            ("- 내용", "- 내용\n- "),
            ("  * 내용", "  * 내용\n  * "),
            ("9. 내용", "9. 내용\n10. "),
            ("2) 내용", "2) 내용\n3) "),
            ("- [x] 완료", "- [x] 완료\n- [ ] "),
            ("> 인용", "> 인용\n\n"),
        ] {
            let (text, caret) = apply(source, source.len(), Command::Enter);
            assert_eq!(text, expected);
            assert_eq!(caret, text.len());
        }
    }
    #[test]
    fn empty_list_enter_and_prefix_backspace_become_paragraph() {
        for source in ["- ", "4. ", "- [ ] ", "> "] {
            assert_eq!(
                apply(source, source.len(), Command::Enter),
                (String::new(), 0)
            );
        }
        let source = "앞 😀\n- [x] 한글";
        let caret = source.find("한글").unwrap();
        let (text, caret) = apply(source, caret, Command::Backspace);
        assert_eq!(text, "앞 😀\n한글");
        assert_eq!(&text[caret..], "한글");
    }
    #[test]
    fn list_split_in_middle_and_crlf_preserve_offsets() {
        let source = "앞\r\n- 가나다";
        let caret = source.find("다").unwrap();
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "앞\r\n- 가나\r\n- 다");
        assert_eq!(&text[caret..], "다");
    }
    #[test]
    fn invalid_offsets_plain_text_and_code_backspace_pass_through() {
        assert!(command("한글", 1, Command::Enter).is_none());
        assert!(command("hello", 50, Command::Enter).is_none());
        assert!(command("not a list", 10, Command::Enter).is_none());
        let source = "```text\n- item\n```";
        assert!(command(source, source.find("item").unwrap(), Command::Backspace).is_none());
    }
    #[test]
    fn empty_code_backspace_removes_whole_block() {
        let source = "before\n\n```text\n\n```\n\nafter";
        let caret = source.find("```text\n").unwrap() + "```text\n".len();
        let (text, caret) = apply(source, caret, Command::Backspace);
        assert_eq!(text, "before\n\n\nafter");
        assert_eq!(&text[caret..], "\nafter");
    }
    #[test]
    fn enter_at_heading_start_preserves_existing_heading() {
        let source = "앞 😀\n\n## 제목";
        let caret = source.find("제목").unwrap();
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "앞 😀\n\n\n## 제목");
        assert_eq!(&text[caret..], "제목");
    }
    #[test]
    fn crlf_code_enter_and_exit_preserve_line_endings() {
        let source = "```rust\r\n한글\r\n\r\n\r\n```\r\n\r\n다음";
        let caret = source.find("\r\n```").unwrap();
        let (text, caret) = apply(source, caret, Command::Enter);
        assert_eq!(text, "```rust\r\n한글\r\n```\r\n\r\n다음");
        assert_eq!(&text[caret..], "다음");
    }
    #[test]
    fn tab_in_imported_empty_code_keeps_closing_fence() {
        let source = "```java\n```";
        let (text, caret) = apply(source, "```java\n".len(), Command::Tab);
        assert_eq!(text, "```java\n  \n```");
        assert_eq!(&text[caret..], "\n```");
    }
    #[test]
    fn backspace_at_code_body_start_cannot_corrupt_language() {
        for source in [
            "```java\nX\n```",
            "~~~python\r\n한글\r\n~~~",
            "```rust\nfn main() {}",
        ] {
            let caret = source.find('\n').unwrap() + 1;
            let (text, after) = apply(source, caret, Command::Backspace);
            assert_eq!(text, source);
            assert_eq!(after, caret);
        }
    }
    #[test]
    fn fenced_navigation_skips_structure_and_preserves_body_boundaries() {
        let source = "앞\n\n```java\n한😀\n```\n\n뒤";
        let open = source.find("```java").unwrap();
        let body = source.find("한😀").unwrap();
        let body_end = body + "한😀".len();
        let close = source.rfind("\n```").unwrap() + 1;
        assert_eq!(fenced_caret(source, body - 1, true), Some("앞".len()));
        assert_eq!(fenced_caret(source, open + 1, false), Some(body));
        assert_eq!(fenced_caret(source, body, true), None);
        assert_eq!(fenced_caret(source, body_end, false), None);
        assert_eq!(fenced_caret(source, close + 1, true), Some(body_end));
        assert_eq!(
            fenced_caret(source, close + 1, false),
            Some(source.find('뒤').unwrap())
        );
        let source = "```java\nX\n```";
        assert_eq!(fenced_caret(source, 7, true), Some(8));
        assert_eq!(fenced_caret(source, source.len() - 1, false), Some(9));
        assert_eq!(fenced_caret("```java", 3, false), None);
        assert_eq!(fenced_caret("한글", 1, false), None);
    }
}
