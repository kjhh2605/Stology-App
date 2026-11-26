//! The note excerpt the study assistant receives when the user attaches the
//! blocks they selected. Pure string logic: the editor supplies the blocks and
//! which of them are selected.
use super::image_edit::markdown;
use super::note_blocks::{Block, Kind};
use std::ops::Range;

/// Largest excerpt sent with one question. Blocks are never cut in the middle.
pub const MAX_BYTES: usize = 16 * 1024;
/// Shown when the chosen blocks alone exceed `MAX_BYTES`.
pub const TOO_LONG: &str = "선택한 블록이 너무 깁니다 · 블록을 줄여 주세요";
/// Shown when nothing is selected. Only selected blocks can be attached.
pub const NOTHING_SELECTED: &str = "붙일 블록을 먼저 골라 주세요 · 본문 왼쪽 여백에서 끌어 고를 수 있어요";

#[derive(Clone, Debug, PartialEq)]
pub struct Excerpt {
    /// `선택한 블록 2개`.
    pub label: String,
    /// What the assistant receives: the note's name, then the blocks.
    pub text: String,
    /// Source ranges of the chosen blocks, to find them again later.
    pub blocks: Vec<Range<usize>>,
    /// First line of each chosen block, for when the note has changed since.
    pub heads: Vec<String>,
}

fn content(source: &str, block: &Block) -> String {
    let text = source[block.bytes.clone()].trim_end_matches(['\r', '\n']);
    if block.kind != Kind::Image {
        return text.to_owned();
    }
    // Images stay on the Mac: the assistant gets one line naming the picture.
    let name = markdown::images(text).into_iter().next().map(|image| {
        let alt = image.alt.trim().to_owned();
        if !alt.is_empty() { return alt; }
        let file = image.destination.rsplit('/').next().unwrap_or("").to_owned();
        if image.destination.contains(".diagram/") { "다이어그램".to_owned() } else { file }
    }).unwrap_or_default();
    format!("[그림: {}]", if name.is_empty() { "이름 없음" } else { &name })
}

fn head(source: &str, block: &Block) -> String {
    source[block.bytes.clone()].lines().map(str::trim).find(|l| !l.is_empty())
        .unwrap_or("").chars().take(80).collect()
}

/// Stands between blocks that are apart in the note. The preview draws it as a rule.
pub const SKIPPED: &str = "[중간 생략]";

/// Blocks one per line; a blank line where the note had one between them, and
/// `SKIPPED` where unchosen text lies between them.
fn joined(source: &str, note: &[Block], indices: &[usize]) -> String {
    let mut text = String::new();
    for (n, i) in indices.iter().enumerate() {
        if n > 0 {
            let between = indices[n - 1] + 1..*i;
            let skipped = between.clone().any(|k| note[k].kind != Kind::Blank && !source[note[k].bytes.clone()].trim().is_empty());
            let gap = between.into_iter().any(|k| note[k].kind == Kind::Blank);
            text.push_str(if skipped { "\n" } else if gap { "\n\n" } else { "\n" });
            if skipped { text.push_str(SKIPPED); text.push('\n'); }
        }
        text.push_str(&content(source, &note[*i]));
    }
    text
}

/// `selected` are block indices. Only the selected blocks go, under one line
/// naming the note.
pub fn excerpt(title: &str, source: &str, note: &[Block], selected: &[usize]) -> Result<Excerpt, String> {
    let filled = |i: &usize| note.get(*i).is_some_and(|b| b.kind != Kind::Blank && !source[b.bytes.clone()].trim().is_empty());
    let mut selected: Vec<usize> = selected.iter().copied().filter(filled).collect();
    selected.sort_unstable();
    selected.dedup();
    if selected.is_empty() { return Err(NOTHING_SELECTED.to_owned()); }
    let header = format!("[보고 있는 노트: {}]\n", if title.trim().is_empty() { "제목 없음" } else { title.trim() });
    let text = header + &joined(source, note, &selected);
    if text.len() > MAX_BYTES { return Err(TOO_LONG.to_owned()); }
    Ok(Excerpt {
        label: format!("선택한 블록 {}개", selected.len()),
        blocks: selected.iter().map(|i| note[*i].bytes.clone()).collect(),
        heads: selected.iter().map(|i| head(source, &note[*i])).collect(),
        text,
    })
}

/// The excerpt as Markdown for the preview: the block contents only. The note
/// line is left out, `SKIPPED` becomes a rule, and each note line stays its own
/// paragraph so blocks read as in the note. Tables, lists and code are left as
/// they are.
pub fn display_markdown(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fence = false;
    let mut previous = "";
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if n == 0 && trimmed.starts_with("[보고 있는 노트:") && trimmed.ends_with(']') { continue; }
        let in_code = fence;
        if trimmed.starts_with("```") { fence = !fence; }
        if !in_code && trimmed == SKIPPED {
            if out.last().is_some_and(|l| !l.is_empty()) { out.push(String::new()); }
            out.push("---".to_owned());
            out.push(String::new());
            previous = "";
            continue;
        }
        // Consecutive table rows, list items and code lines stay together.
        let joined = in_code || trimmed.is_empty() || previous.is_empty()
            || (trimmed.starts_with('|') && previous.starts_with('|'));
        if !joined && !previous.is_empty() { out.push(String::new()); }
        out.push(line.to_owned());
        previous = if trimmed.is_empty() { "" } else { trimmed };
    }
    out.join("\n").trim().to_owned()
}

/// Where the chosen blocks are now. The saved ranges win while their first
/// lines still match; otherwise each block is found again by its first line.
pub fn locate(source: &str, note: &[Block], blocks: &[Range<usize>], heads: &[String]) -> Vec<usize> {
    let exact: Vec<usize> = blocks.iter().zip(heads)
        .filter_map(|(range, line)| note.iter().position(|b| b.bytes == *range && head(source, b) == *line))
        .collect();
    if !exact.is_empty() && exact.len() == blocks.len() { return exact; }
    let mut found: Vec<usize> = heads.iter().filter(|h| !h.is_empty())
        .filter_map(|line| note.iter().position(|b| head(source, b) == *line))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source: &str) -> Vec<Block> {
        let mut offset = 0;
        source.split_inclusive('\n').map(|line| {
            let start = offset;
            offset += line.len();
            let kind = if line.trim().is_empty() { Kind::Blank } else if line.starts_with("![") { Kind::Image } else { Kind::Text };
            Block { bytes: start..offset, kind, indent: 0 }
        }).collect()
    }

    const NOTE: &str = "# 3주차\n\n앞 문단\n고른 문단\n| 표 |\n뒤 문단\n끝\n";

    #[test]
    fn only_the_selected_blocks_are_sent() {
        let note = lines(NOTE);
        let got = excerpt("3주차 정리", NOTE, &note, &[3, 4]).unwrap();
        assert_eq!(got.label, "선택한 블록 2개");
        assert_eq!(got.text, "[보고 있는 노트: 3주차 정리]\n고른 문단\n| 표 |");
        assert_eq!(got.blocks, vec![note[3].bytes.clone(), note[4].bytes.clone()]);
        assert_eq!(got.heads, vec!["고른 문단", "| 표 |"]);
    }

    #[test]
    fn nothing_is_attached_without_selected_blocks() {
        let note = lines(NOTE);
        assert_eq!(excerpt("", NOTE, &note, &[]).unwrap_err(), NOTHING_SELECTED);
        // A selected blank line alone is nothing to send.
        assert_eq!(excerpt("", NOTE, &note, &[1]).unwrap_err(), NOTHING_SELECTED);
        assert_eq!(excerpt("", NOTE, &note, &[2]).unwrap().text, "[보고 있는 노트: 제목 없음]\n앞 문단");
    }

    #[test]
    fn blocks_apart_in_the_note_are_marked_and_drawn_as_a_rule() {
        let note = lines(NOTE);
        // 앞 문단 + 뒤 문단: 고른 문단과 표를 건너뛴다. 끝은 뒤 문단 바로 다음이다.
        let got = excerpt("t", NOTE, &note, &[2, 5, 6]).unwrap();
        assert_eq!(got.text, "[보고 있는 노트: t]\n앞 문단\n[중간 생략]\n뒤 문단\n끝");
        assert_eq!(display_markdown(&got.text), "앞 문단\n\n---\n\n뒤 문단\n\n끝");
        // 빈 줄만 사이에 있으면 이어진 블록이다.
        let got = excerpt("t", NOTE, &note, &[0, 2]).unwrap();
        assert_eq!(got.text, "[보고 있는 노트: t]\n# 3주차\n\n앞 문단");
    }

    #[test]
    fn pictures_are_named_not_sent() {
        let source = "![회로도](images/a.png)\n![](images/b%20c.png)\n![](.diagram/x.svg)\n";
        let note = lines(source);
        let got = excerpt("t", source, &note, &[0, 1, 2]).unwrap();
        assert!(got.text.contains("[그림: 회로도]\n[그림: b%20c.png]\n[그림: 다이어그램]"), "{}", got.text);
    }

    #[test]
    fn long_selections_are_refused() {
        let big = "a".repeat(MAX_BYTES / 3 + 10);
        let source = format!("{big}\n{big}\n{big}\n");
        let note = lines(&source);
        assert_eq!(excerpt("t", &source, &note, &[0, 1, 2]).unwrap_err(), TOO_LONG);
        let got = excerpt("t", &source, &note, &[1]).unwrap();
        assert_eq!(got.text.matches(big.as_str()).count(), 1);
    }

    #[test]
    fn the_preview_shows_only_the_blocks() {
        let text = "[보고 있는 노트: 3주차]\n문단 하나\n| a | b |\n| --- | --- |\n| 1 | 2 |\n뒤";
        assert_eq!(display_markdown(text), "문단 하나\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n뒤");
        assert_eq!(display_markdown("```rs\nlet a;\nlet b;\n```"), "```rs\nlet a;\nlet b;\n```");
    }

    #[test]
    fn blocks_are_found_again_after_the_note_changed() {
        let note = lines(NOTE);
        let got = excerpt("t", NOTE, &note, &[3]).unwrap();
        assert_eq!(locate(NOTE, &note, &got.blocks, &got.heads), vec![3]);
        let edited = format!("새 줄\n{NOTE}");
        assert_eq!(locate(&edited, &lines(&edited), &got.blocks, &got.heads), vec![4]);
        assert!(locate("다른 노트\n", &lines("다른 노트\n"), &got.blocks, &got.heads).is_empty());
    }
}
