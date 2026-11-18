//! Standard Markdown image links with optional Stology layout in the title.
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct ImageLink {
    pub bytes: Range<usize>,
    pub destination: String,
    pub title: String,
    pub alt: String,
    pub width: Option<f64>,
    pub x: f64,
    pub wrap: bool,
}
impl ImageLink {
    pub fn markdown(&self, destination: &str, width: f64, x: f64, wrap: bool) -> String {
        let title = self.title.split("stology:").next().unwrap_or("").trim();
        let title = if title.is_empty() {
            String::new()
        } else {
            format!("{} ", title.replace('"', "&quot;"))
        };
        format!(
            "![{}](<{}> \"{}stology:width={:.0};x={:.3};wrap={}\")",
            self.alt.replace(']', "\\]"),
            destination,
            title,
            width.clamp(32.0, 4096.0),
            x.clamp(0.0, 1.0),
            if wrap { 1 } else { 0 }
        )
    }
}
pub fn images(source: &str) -> Vec<ImageLink> {
    Parser::new_ext(source, Options::all())
        .into_offset_iter()
        .filter_map(|(event, bytes)| {
            let Event::Start(Tag::Image {
                dest_url, title, ..
            }) = event
            else {
                return None;
            };
            let mut image = ImageLink {
                alt: source[bytes.clone()]
                    .strip_prefix("![")
                    .and_then(|s| s.split_once(']'))
                    .map(|(s, _)| s.to_owned())
                    .unwrap_or_default(),
                bytes,
                destination: dest_url.into_string(),
                title: title.into_string(),
                width: None,
                x: 0.0,
                wrap: false,
            };
            if let Some((_, metadata)) = image.title.split_once("stology:") {
                for field in metadata.split(';') {
                    if let Some((key, value)) = field.split_once('=') {
                        match key {
                            "width" => {
                                image.width = value
                                    .parse::<f64>()
                                    .ok()
                                    .filter(|v| v.is_finite())
                                    .map(|v| v.clamp(32.0, 4096.0))
                            }
                            "x" => {
                                image.x = value
                                    .parse::<f64>()
                                    .ok()
                                    .filter(|v| v.is_finite())
                                    .unwrap_or(0.0)
                                    .clamp(0.0, 1.0)
                            }
                            "wrap" => image.wrap = value == "1",
                            _ => (),
                        }
                    }
                }
            }
            Some(image)
        })
        .collect()
}
/// Rebase only app-owned image links. Never touch code, remote links or prose.
pub fn rebase(source: &str, old_folder: &str, new_folder: &str) -> String {
    let old_depth = old_folder.split('/').filter(|s| !s.is_empty()).count();
    let prefix = format!("{}이미지/", "../".repeat(old_depth));
    let new_prefix = format!(
        "{}이미지/",
        "../".repeat(new_folder.split('/').filter(|s| !s.is_empty()).count())
    );
    let mut result = source.to_owned();
    for image in images(source).into_iter().rev() {
        let Some(file) = image.destination.strip_prefix(&prefix) else {
            continue;
        };
        let Some((hash, ext)) = file.rsplit_once('.') else {
            continue;
        };
        if hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(ext, "png" | "jpg" | "jpeg")
        {
            continue;
        }
        let raw = &source[image.bytes.clone()];
        if let Some(start) = raw.find("](").map(|p| p + 2) {
            let start = start + usize::from(raw[start..].starts_with('<'));
            if raw[start..].starts_with(&image.destination) {
                result.replace_range(
                    image.bytes.start + start..image.bytes.start + start + image.destination.len(),
                    &format!("{new_prefix}{file}"),
                );
            }
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_roundtrip_and_rebase() {
        let hash = "a".repeat(64);
        let source = format!("앞\n![그림](이미지/{hash}.png)\n뒤");
        let image = images(&source).remove(0);
        let markup = image.markdown(&image.destination, 237.0, 0.7, true);
        let restored = images(&markup).remove(0);
        assert_eq!(restored.width, Some(237.0));
        assert_eq!(restored.x, 0.7);
        assert!(restored.wrap);
        let moved = rebase(&markup, "", "a/b");
        assert!(moved.contains("../../이미지/"));
        assert_eq!(rebase(&moved, "a/b", ""), markup);
        let code = format!("```\n{markup}\n```");
        assert_eq!(rebase(&code, "", "a"), code);
    }
}
