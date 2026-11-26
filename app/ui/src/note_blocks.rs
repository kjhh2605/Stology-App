//! Line-based note blocks, the rows attachments share with one side block, and
//! source-preserving block moves. Pure string logic; the editor supplies the
//! ranges it actually renders as code, tables, images and rules.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Blank,
    Text,
    Item,
    Code,
    Table,
    Image,
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Whole source lines. Tables and images keep their one blank separator.
    pub bytes: Range<usize>,
    pub kind: Kind,
    /// Leading whitespace of a list item. Deeper items move with their parent.
    pub indent: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Full,
    Left,
    Right,
}

/// An attachment and the single block that sits in the other column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub media: usize,
    pub side: Option<usize>,
    pub placement: Placement,
}

fn blank(line: &str) -> bool {
    line.trim_matches([' ', '\t', '\r', '\n']).is_empty()
}

fn item_indent(line: &str) -> Option<usize> {
    let indent = line.bytes().take_while(|b| matches!(b, b' ' | b'\t')).count();
    let rest = line[indent..].as_bytes();
    let bullet = matches!(rest.first(), Some(b'-' | b'+' | b'*')) && rest.get(1) == Some(&b' ');
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    let numbered = (1..=9).contains(&digits)
        && matches!(rest.get(digits), Some(b'.' | b')'))
        && rest.get(digits + 1) == Some(&b' ');
    (bullet || numbered).then_some(indent)
}

/// `structures` are sorted, rendered ranges of code, tables, standalone image
/// links and rules. Every other line is its own block.
pub fn blocks(source: &str, structures: &[(Range<usize>, Kind)]) -> Vec<Block> {
    let mut result = Vec::new();
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        lines.push(offset..offset + line.len());
        offset += line.len();
    }
    let mut next = 0;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].clone();
        while structures.get(next).is_some_and(|(range, _)| range.end <= line.start) {
            next += 1;
        }
        let structure = structures
            .get(next)
            .filter(|(range, _)| range.start < line.end && range.start >= line.start);
        if let Some((range, kind)) = structure {
            let mut end = index;
            while end + 1 < lines.len() && lines[end].end < range.end {
                end += 1;
            }
            if matches!(kind, Kind::Table | Kind::Image)
                && lines.get(end + 1).is_some_and(|l| blank(&source[l.clone()]))
                && source[..lines[end].end].ends_with('\n')
            {
                end += 1;
            }
            result.push(Block { bytes: line.start..lines[end].end, kind: *kind, indent: 0 });
            index = end + 1;
            next += 1;
            continue;
        }
        let raw = &source[line.clone()];
        let (kind, indent) = if blank(raw) {
            (Kind::Blank, 0)
        } else if let Some(indent) = item_indent(raw) {
            (Kind::Item, indent)
        } else {
            (Kind::Text, 0)
        };
        result.push(Block { bytes: line, kind, indent });
        index += 1;
    }
    result
}

pub fn block_at(blocks: &[Block], byte: usize) -> Option<usize> {
    blocks
        .iter()
        .position(|b| b.bytes.start <= byte && byte < b.bytes.end)
        .or_else(|| (!blocks.is_empty() && byte >= blocks[blocks.len() - 1].bytes.end).then(|| blocks.len() - 1))
}

/// A list item together with the deeper items that follow it.
pub fn subtree(blocks: &[Block], index: usize) -> Range<usize> {
    let mut end = index + 1;
    if blocks[index].kind == Kind::Item {
        while blocks.get(end).is_some_and(|b| b.kind == Kind::Item && b.indent > blocks[index].indent) {
            end += 1;
        }
    }
    index..end
}

/// Pair each placed attachment with the block that follows it. A rule always
/// takes a whole row, and a block already used as a side block starts no row.
pub fn rows(blocks: &[Block], placement: impl Fn(usize) -> Option<Placement>) -> Vec<Row> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < blocks.len() {
        if let Some(placement) = placement(index) {
            let side = (placement != Placement::Full && blocks.get(index + 1).is_some_and(|b| b.kind != Kind::Rule))
                .then_some(index + 1);
            result.push(Row { media: index, side, placement });
            index += 1 + usize::from(side.is_some());
        } else {
            index += 1;
        }
    }
    result
}

/// Reorder whole blocks. `moving` holds ascending block indices and `target`
/// the block they go in front of (`blocks.len()` for the end); `None` deletes
/// them. Returns the new source and where the moved text now starts and ends.
pub fn reordered(
    source: &str,
    blocks: &[Block],
    moving: &[usize],
    target: Option<usize>,
) -> Option<(String, Range<usize>)> {
    if moving.is_empty() || moving.iter().any(|i| *i >= blocks.len()) || target.is_some_and(|t| t > blocks.len()) {
        return None;
    }
    let kept = |i: &usize| !moving.contains(i);
    let mut order: Vec<usize> = (0..blocks.len()).filter(kept).collect();
    let mut landing = 0..0;
    if let Some(target) = target {
        let at = (0..target).filter(kept).count();
        landing = at..at + moving.len();
        order.splice(at..at, moving.iter().copied());
    }
    if order.iter().copied().eq(0..blocks.len()) {
        return None;
    }
    let newline = if source.contains("\r\n") { "\r\n" } else { "\n" };
    let mut result = String::with_capacity(source.len() + 8);
    let mut moved = 0..0;
    for (position, block) in order.iter().enumerate() {
        if position == landing.start {
            moved.start = result.len();
        }
        let text = &source[blocks[*block].bytes.clone()];
        result.push_str(text);
        let last = position + 1 == order.len();
        if !last && !text.ends_with('\n') {
            result.push_str(newline);
        }
        // A table swallows the next line unless a blank line ends it.
        if !last && blocks[*block].kind == Kind::Table && !blank(result.rsplit_terminator('\n').next().unwrap_or("")) {
            result.push_str(newline);
        }
        if position + 1 == landing.end {
            moved.end = result.len();
        }
    }
    Some((result, moved))
}

/// The smallest replacement that turns `old` into `new`, on char boundaries.
pub fn edit_span<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let limit = old.len().min(new.len()) - prefix;
    let mut suffix = old.bytes().rev().zip(new.bytes().rev()).take(limit).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str, marks: &[(&str, Kind)]) -> Vec<Block> {
        let mut structures: Vec<_> = marks
            .iter()
            .map(|(needle, kind)| {
                let start = source.find(needle).unwrap();
                (start..start + needle.len(), *kind)
            })
            .collect();
        structures.sort_by_key(|(range, _)| range.start);
        blocks(source, &structures)
    }
    fn texts<'a>(source: &'a str, blocks: &[Block]) -> Vec<&'a str> {
        blocks.iter().map(|b| &source[b.bytes.clone()]).collect()
    }

    #[test]
    fn every_line_is_a_block_and_attachments_keep_one_separator() {
        let source = "제목\n\n![그림](a.png)\n\n옆 글\n| a |\n| - |\n| 1 |\n\n\n```rs\nlet a = 1;\n\n```\n끝";
        let blocks = parse(source, &[
            ("![그림](a.png)", Kind::Image),
            ("| a |\n| - |\n| 1 |\n", Kind::Table),
            ("```rs\nlet a = 1;\n\n```\n", Kind::Code),
        ]);
        assert_eq!(texts(source, &blocks), [
            "제목\n", "\n", "![그림](a.png)\n\n", "옆 글\n", "| a |\n| - |\n| 1 |\n\n", "\n",
            "```rs\nlet a = 1;\n\n```\n", "끝",
        ]);
        assert_eq!(
            blocks.iter().map(|b| b.kind).collect::<Vec<_>>(),
            [Kind::Text, Kind::Blank, Kind::Image, Kind::Text, Kind::Table, Kind::Blank, Kind::Code, Kind::Text]
        );
        assert_eq!(blocks.iter().map(|b| b.bytes.len()).sum::<usize>(), source.len());
        assert_eq!(block_at(&blocks, source.find("옆").unwrap()), Some(3));
        assert_eq!(block_at(&blocks, source.len()), Some(7));
    }

    #[test]
    fn crlf_and_missing_final_newline_keep_exact_ranges() {
        let source = "가\r\n![그림](a.png)\r\n\r\n나";
        let blocks = parse(source, &[("![그림](a.png)", Kind::Image)]);
        assert_eq!(texts(source, &blocks), ["가\r\n", "![그림](a.png)\r\n\r\n", "나"]);
        let alone = "![그림](a.png)";
        assert_eq!(texts(alone, &parse(alone, &[(alone, Kind::Image)])), [alone]);
    }

    #[test]
    fn list_items_carry_deeper_items() {
        let source = "- 가\n  - 나\n    1. 다\n- 라\n본문\n";
        let blocks = parse(source, &[]);
        assert_eq!(blocks.iter().map(|b| (b.kind, b.indent)).collect::<Vec<_>>(), [
            (Kind::Item, 0), (Kind::Item, 2), (Kind::Item, 4), (Kind::Item, 0), (Kind::Text, 0),
        ]);
        assert_eq!(subtree(&blocks, 0), 0..3);
        assert_eq!(subtree(&blocks, 1), 1..3);
        assert_eq!(subtree(&blocks, 3), 3..4);
        assert_eq!(subtree(&blocks, 4), 4..5);
    }

    #[test]
    fn placed_attachments_take_exactly_one_side_block() {
        let source = "![a](a.png)\n\n옆\n아래\n![b](b.png)\n\n---\n![c](c.png)\n\n![d](d.png)\n\n뒤\n![e](e.png)\n";
        let blocks = parse(source, &[
            ("![a](a.png)", Kind::Image), ("![b](b.png)", Kind::Image), ("---", Kind::Rule),
            ("![c](c.png)", Kind::Image), ("![d](d.png)", Kind::Image), ("![e](e.png)", Kind::Image),
        ]);
        let placements = [Placement::Left, Placement::Right, Placement::Left, Placement::Left, Placement::Left];
        let rows = rows(&blocks, |i| {
            (blocks[i].kind == Kind::Image).then(|| {
                placements[blocks[..i].iter().filter(|b| b.kind == Kind::Image).count()]
            })
        });
        assert_eq!(rows, [
            Row { media: 0, side: Some(1), placement: Placement::Left },
            // A rule cannot sit beside an attachment.
            Row { media: 3, side: None, placement: Placement::Right },
            // The second image is the first one's side block and starts no row.
            Row { media: 5, side: Some(6), placement: Placement::Left },
            // Nothing follows the last attachment.
            Row { media: 8, side: None, placement: Placement::Left },
        ]);
        let full = super::rows(&blocks, |i| (i == 0).then_some(Placement::Full));
        assert_eq!(full, [Row { media: 0, side: None, placement: Placement::Full }]);
    }

    fn apply(source: &str, blocks: &[Block], moving: &[usize], target: Option<usize>) -> (String, String) {
        let (result, moved) = reordered(source, blocks, moving, target).unwrap();
        let (span, replacement) = edit_span(source, &result);
        let mut patched = source.to_owned();
        patched.replace_range(span, replacement);
        assert_eq!(patched, result, "the minimal edit reproduces the reordered source");
        let landed = result[moved].to_owned();
        (result, landed)
    }

    #[test]
    fn moves_keep_every_byte_and_report_the_landing_range() {
        let source = "가\n나\n다\n라";
        let blocks = parse(source, &[]);
        assert_eq!(apply(source, &blocks, &[2], Some(0)), ("다\n가\n나\n라".into(), "다\n".into()));
        // The last line gains the newline it needs once it is no longer last.
        assert_eq!(apply(source, &blocks, &[3], Some(1)), ("가\n라\n나\n다\n".into(), "라\n".into()));
        assert_eq!(apply(source, &blocks, &[0, 1], Some(4)), ("다\n라\n가\n나\n".into(), "가\n나\n".into()));
        assert_eq!(apply(source, &blocks, &[1, 2], None).0, "가\n라");
        // Dropping a block where it already is changes nothing.
        assert!(reordered(source, &blocks, &[1], Some(1)).is_none());
        assert!(reordered(source, &blocks, &[1], Some(2)).is_none());
        assert!(reordered(source, &blocks, &[], Some(0)).is_none());
        assert!(reordered(source, &blocks, &[9], Some(0)).is_none());
    }

    #[test]
    fn moves_protect_tables_and_rules() {
        let source = "| a |\n| - |\n| 1 |";
        let table = format!("앞\n{source}");
        let blocks = parse(&table, &[(source, Kind::Table)]);
        // A table that leaves the end of the note needs its blank separator.
        assert_eq!(apply(&table, &blocks, &[1], Some(0)).0, format!("{source}\n\n앞\n"));
        let rule = "글\n\n---\n위\n";
        let blocks = parse(rule, &[("---", Kind::Rule)]);
        // A rule stays a rule under a text line, so no separator is added.
        assert_eq!(apply(rule, &blocks, &[3], Some(2)).0, "글\n\n위\n---\n");
        let crlf = "가\r\n나";
        assert_eq!(apply(crlf, &parse(crlf, &[]), &[1], Some(0)).0, "나\r\n가\r\n");
    }

    #[test]
    fn edit_span_respects_multibyte_boundaries() {
        assert_eq!(edit_span("한글", "한국"), (3..6, "국"));
        assert_eq!(edit_span("가나다", "가다"), (3..6, ""));
        assert_eq!(edit_span("aa", "aaa"), (2..2, "a"));
        assert_eq!(edit_span("같음", "같음"), (6..6, ""));
    }
}
