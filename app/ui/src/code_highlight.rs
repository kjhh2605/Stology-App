//! Language-aware code presentation. Source bytes remain the canonical Markdown;
//! callers convert these UTF-8 boundaries to their native text system's ranges.
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use std::{ops::Range, sync::OnceLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxDefinition, SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

/// Tolaria-compatible menu IDs and labels. The shell aliases share one Bash entry.
/// IDs remain stable in Markdown; labels are only presentation.
pub const CODE_LANGUAGES: &[(&str, &str)] = &[
    ("text", "Plain Text"),
    ("c", "C"),
    ("cpp", "C++"),
    ("css", "CSS"),
    ("glsl", "GLSL"),
    ("graphql", "GraphQL"),
    ("haml", "Ruby Haml"),
    ("html", "HTML"),
    ("java", "Java"),
    ("javascript", "JavaScript"),
    ("json", "JSON"),
    ("jsonc", "JSON with Comments"),
    ("jsonl", "JSON Lines"),
    ("jsx", "JSX"),
    ("julia", "Julia"),
    ("less", "Less"),
    ("markdown", "Markdown"),
    ("mdx", "MDX"),
    ("php", "PHP"),
    ("postcss", "PostCSS"),
    ("pug", "Pug"),
    ("python", "Python"),
    ("r", "R"),
    ("regexp", "RegExp"),
    ("sass", "Sass"),
    ("scss", "SCSS"),
    ("sql", "SQL"),
    ("svelte", "Svelte"),
    ("typescript", "TypeScript"),
    ("vue", "Vue"),
    ("vue-html", "Vue HTML"),
    ("wasm", "WebAssembly"),
    ("wgsl", "WGSL"),
    ("xml", "XML"),
    ("yaml", "YAML"),
    ("tsx", "TSX"),
    ("haskell", "Haskell"),
    ("csharp", "C#"),
    ("latex", "LaTeX"),
    ("lua", "Lua"),
    ("mermaid", "Mermaid"),
    ("ruby", "Ruby"),
    ("rust", "Rust"),
    ("scala", "Scala"),
    ("swift", "Swift"),
    ("kotlin", "Kotlin"),
    ("objective-c", "Objective C"),
    ("go", "Go"),
    ("bash", "Bash"),
    ("powershell", "PowerShell"),
    ("vbscript", "VBScript"),
    ("dart", "Dart"),
    ("groovy", "Groovy"),
    ("matlab", "MATLAB"),
    ("perl", "Perl"),
    ("elixir", "Elixir"),
    ("erlang", "Erlang"),
    ("fsharp", "F#"),
    ("clojure", "Clojure"),
    ("asm", "Assembly"),
    ("zig", "Zig"),
    ("hcl", "HCL"),
    ("terraform", "Terraform"),
    ("dockerfile", "Dockerfile"),
    ("batch", "Batch"),
    ("diff", "Diff"),
    ("ini", "INI"),
    ("toml", "TOML"),
    ("pascal", "Pascal"),
];

/// Reserved for catalogue entries without a bundled grammar; currently empty.
pub const UNSUPPORTED_CODE_LANGUAGES: &[&str] = &[];

/// Normalize known aliases while preserving unknown fence identifiers.
pub fn canonical_language(language: &str) -> String {
    let normalized = language
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match normalized.as_str() {
        "txt" | "plain" | "" | "plaintext" | "none" => "text",
        "c++" | "cc" | "cxx" => "cpp",
        "gql" => "graphql",
        "js" | "node" | "nodejs" => "javascript",
        "jl" => "julia",
        "md" => "markdown",
        "jade" => "pug",
        "py" | "python3" | "py3" => "python",
        "regex" | "re" => "regexp",
        "ts" => "typescript",
        "yml" => "yaml",
        "typescriptreact" => "tsx",
        "hs" => "haskell",
        "c#" | "cs" => "csharp",
        "mmd" => "mermaid",
        "rb" => "ruby",
        "rs" => "rust",
        "kt" | "kts" => "kotlin",
        "objc" => "objective-c",
        "golang" => "go",
        "sh" | "shell" | "shellscript" | "zsh" => "bash",
        "ps" | "ps1" => "powershell",
        "vbs" | "vb" | "vba" | "visual-basic" | "visualbasic" => "vbscript",
        "pl" | "pm" => "perl",
        "ex" | "exs" => "elixir",
        "erl" => "erlang",
        "f#" | "fs" => "fsharp",
        "clj" => "clojure",
        "assembly" => "asm",
        "tf" | "tfvars" => "terraform",
        "docker" => "dockerfile",
        "bat" | "cmd" => "batch",
        "patch" => "diff",
        "properties" => "ini",
        "pas" | "objectpascal" | "delphi" => "pascal",
        "javascriptreact" => "jsx",
        "tex" => "latex",
        other => other,
    }
    .to_string()
}

const MAX_INFERENCE_BYTES: usize = 64 * 1024;

/// Infer an unlabelled block locally, up to 64 KiB. Explicit choices never change.
pub fn infer_language(source: &str) -> Option<&'static str> {
    if source.len() > MAX_INFERENCE_BYTES {
        return None;
    }
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    if source.starts_with(['{', '[']) && serde_json::from_str::<serde_json::Value>(source).is_ok() {
        return Some("json");
    }
    if source.starts_with('<')
        && source.ends_with('>')
        && source
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!')
    {
        return Some("html");
    }
    let lines: Vec<_> = source.lines().map(str::trim).collect();
    if lines.iter().any(|line| {
        ((line.starts_with("def ") || line.starts_with("class ")) && line.ends_with(':'))
            || (line.starts_with("from ") && line.contains(" import "))
            || (line.starts_with("import ") && !line.contains(['{', '\'', '"', ';']))
    }) {
        return Some("python");
    }
    if lines.iter().any(|line| {
        (line.starts_with("#!")
            && ["bash", "/sh", "zsh"]
                .iter()
                .any(|shell| line.contains(shell)))
            || ["pnpm ", "npm ", "yarn ", "git ", "cd ", "echo "]
                .iter()
                .any(|command| line.starts_with(command))
            || line
                .strip_prefix("export ")
                .and_then(|value| value.split_once('='))
                .is_some_and(|(name, _)| {
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
    }) {
        return Some("bash");
    }
    let words: Vec<_> = source
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .collect();
    if [
        "interface",
        "type",
        "enum",
        "implements",
        "readonly",
        "namespace",
        "declare",
    ]
    .iter()
    .any(|word| words.contains(word))
        || [
            ": string",
            ": number",
            ": boolean",
            ": unknown",
            ": never",
            ": void",
            ": Record<",
        ]
        .iter()
        .any(|annotation| source.contains(annotation))
        || lines.iter().any(|line| {
            ["const ", "let ", "var ", "function "]
                .iter()
                .any(|prefix| line.starts_with(prefix))
                && line
                    .find('(')
                    .zip(line.find(')'))
                    .is_some_and(|(a, b)| a < b && line[a..b].contains(':'))
        })
    {
        return Some("typescript");
    }
    if [
        "import", "export", "const", "let", "var", "function", "return",
    ]
    .iter()
    .any(|word| words.contains(word))
        || source.contains("=>")
    {
        return Some("javascript");
    }
    let upper = source.to_ascii_uppercase();
    if ["SELECT ", "WITH ", "INSERT ", "UPDATE ", "DELETE "]
        .iter()
        .any(|start| upper.starts_with(start))
        && upper.split_whitespace().any(|word| word == "FROM")
    {
        return Some("sql");
    }
    if source.split_once(':').is_some_and(|(key, _)| {
        !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    }) {
        return Some("yaml");
    }
    None
}

/// Infer only missing fence info. Explicit Plain Text and unknown IDs are kept.
pub fn effective_language(info: &str, body: &str) -> String {
    if info.trim().is_empty() {
        infer_language(body).unwrap_or("text").to_string()
    } else {
        canonical_language(info)
    }
}

pub fn supports_language_highlighting(language: &str) -> bool {
    syntax_for_language(&resources().0, language).is_some()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub bytes: Range<usize>,
    pub rgb: [u8; 3],
}

fn resources() -> &'static (SyntaxSet, Theme) {
    static RESOURCES: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
    RESOURCES.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults();
        let mut builder = two_face::syntax::extra_newlines().into_builder();
        for source in [
            include_str!("../assets/syntaxes/Pug.sublime-syntax"),
            include_str!("../assets/syntaxes/Mermaid.sublime-syntax"),
            include_str!("../assets/syntaxes/WebAssembly.sublime-syntax"),
            include_str!("../assets/syntaxes/VBScript.sublime-syntax"),
            include_str!("../assets/syntaxes/MDX.sublime-syntax"),
        ] {
            builder.add(
                SyntaxDefinition::load_from_str(source, true, None).expect("bundled code grammar"),
            );
        }
        (
            builder.build(),
            themes
                .themes
                .remove("InspiredGitHub")
                .expect("bundled light theme"),
        )
    })
}

fn syntax_for_language<'a>(set: &'a SyntaxSet, language: &str) -> Option<&'a SyntaxReference> {
    let language = canonical_language(language);
    let name = match language.as_str() {
        "text" => return None,
        // Avoid ambiguous extensions (C/C++, Sass/Haml, F#/GLSL).
        "c" => "C",
        "cpp" => "C++",
        "sass" => "Sass",
        "haml" => "Ruby Haml",
        "fsharp" => "F#",
        "regexp" => "Regular Expression",
        "objective-c" => "Objective-C",
        "batch" => "Batch File",
        "wasm" => "WAST (WebAssembly Text)",
        "jsonc" | "jsonl" => "JSON",
        "jsx" | "tsx" => "TypeScriptReact",
        "postcss" => "CSS",
        "vue" | "vue-html" => "Vue Component",
        "hcl" | "terraform" => "Terraform",
        _ => "",
    };
    if !name.is_empty() {
        return set.find_syntax_by_name(name);
    }
    let token = match language.as_str() {
        "rust" => "rs",
        "python" => "py",
        "javascript" => "js",
        "typescript" => "ts",
        "csharp" => "cs",
        "julia" => "jl",
        "kotlin" => "kt",
        "haskell" => "hs",
        "latex" => "tex",
        other => other,
    };
    set.find_syntax_by_token(token)
}

/// Highlight recognized fenced languages, inferring only unlabelled blocks.
/// Explicit Plain Text and unknown languages retain neutral monospace styling.
/// Markdown parsing prevents inline code and longer nested fences from being
/// mistaken for separate blocks; UTF-8 offsets always refer to original source.
pub fn highlight_markdown(source: &str) -> Vec<HighlightSpan> {
    let mut result = Vec::<HighlightSpan>::new();
    let mut block: Option<(String, Vec<(pulldown_cmark::CowStr<'_>, Range<usize>)>)> = None;
    for (event, bytes) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))) => {
                block = Some((language.into_string(), Vec::new()));
            }
            Event::Text(code) => {
                if let Some((_, fragments)) = block.as_mut() {
                    fragments.push((code, bytes));
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                let Some((info, fragments)) = block.take() else {
                    continue;
                };
                // Inference sees parser text, excluding quote/list prefixes and
                // the closing fence; no guesses about source string boundaries.
                let language = if info.trim().is_empty() {
                    if fragments.iter().map(|(code, _)| code.len()).sum::<usize>()
                        > MAX_INFERENCE_BYTES
                    {
                        "text".to_string()
                    } else {
                        let body: String =
                            fragments.iter().map(|(code, _)| code.as_ref()).collect();
                        effective_language(&info, &body)
                    }
                } else {
                    canonical_language(&info)
                };
                let (syntaxes, theme) = resources();
                let Some(syntax) = syntax_for_language(syntaxes, &language) else {
                    continue;
                };
                let mut highlighter = HighlightLines::new(syntax, theme);
                for (code, bytes) in fragments {
                    // Synthesized parser fragments advance lexer state without
                    // guessing a source offset or touching unrelated characters.
                    let direct = source.get(bytes.clone()) == Some(code.as_ref());
                    let mut offset = bytes.start;
                    for line in LinesWithEndings::from(code.as_ref()) {
                        if let Ok(tokens) = highlighter.highlight_line(line, syntaxes) {
                            if direct {
                                let mut token_offset = offset;
                                for (style, token) in tokens {
                                    let end = token_offset + token.len();
                                    if end > token_offset {
                                        let color = style.foreground;
                                        let rgb = [color.r, color.g, color.b];
                                        if let Some(last) = result.last_mut().filter(|last| {
                                            last.bytes.end == token_offset && last.rgb == rgb
                                        }) {
                                            last.bytes.end = end;
                                        } else {
                                            result.push(HighlightSpan {
                                                bytes: token_offset..end,
                                                rgb,
                                            });
                                        }
                                    }
                                    token_offset = end;
                                }
                            }
                        }
                        offset += line.len();
                    }
                }
            }
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn color_at(source: &str, spans: &[HighlightSpan], needle: &str) -> [u8; 3] {
        let offset = source.find(needle).unwrap();
        spans
            .iter()
            .find(|span| span.bytes.contains(&offset))
            .unwrap()
            .rgb
    }

    #[test]
    fn unlabelled_json_is_colored_but_explicit_plain_text_is_preserved() {
        let body = "{\"name\": \"한글\", \"count\": 42}";
        assert_eq!(effective_language("", body), "json");
        assert_eq!(effective_language("text", body), "text");
        assert_eq!(
            effective_language("unknown-grammar", body),
            "unknown-grammar"
        );
        let source = format!("```\n{body}\n```\n");
        let spans = highlight_markdown(&source);
        assert!(
            spans
                .iter()
                .map(|span| span.rgb)
                .collect::<HashSet<_>>()
                .len()
                >= 3
        );
        assert!(highlight_markdown(&format!("```text\n{body}\n```\n")).is_empty());
        // Quote prefixes and an unfinished closing fence must not enter inference.
        assert!(!highlight_markdown(&format!("> ```\n> {body}\n> ```")).is_empty());
        assert!(!highlight_markdown(&format!("```\n{body}")).is_empty());
    }

    #[test]
    fn language_suggestions_prioritize_json_and_typed_code() {
        assert_eq!(infer_language("{\"count\": 1}"), Some("json"));
        assert_eq!(
            infer_language("interface Item { name: string }"),
            Some("typescript")
        );
        assert_eq!(
            infer_language("const greet = (name: string) => name;"),
            Some("typescript")
        );
        assert_eq!(infer_language("from pathlib import Path"), Some("python"));
        assert_eq!(
            infer_language("import Demo from './demo';"),
            Some("javascript")
        );
        assert_eq!(infer_language("#!/bin/bash\necho hello"), Some("bash"));
        assert_eq!(infer_language("SELECT name FROM people"), Some("sql"));
        assert_eq!(infer_language("name: value"), Some("yaml"));
        assert_eq!(
            infer_language("export interface Item { id: string }"),
            Some("typescript")
        );
        assert_eq!(infer_language("export VALUE=1"), Some("bash"));
        assert_eq!(infer_language(&"a".repeat(MAX_INFERENCE_BYTES + 1)), None);
        assert_eq!(infer_language("plain words"), None);
        assert_eq!(infer_language(" "), None);
    }

    #[test]
    fn menu_catalog_is_unique_canonical_and_explicit_about_unsupported_grammars() {
        let mut identifiers = HashSet::new();
        let mut labels = HashSet::new();
        assert_eq!(CODE_LANGUAGES.len(), 69);
        for &(id, label) in CODE_LANGUAGES {
            assert!(identifiers.insert(id), "duplicate ID: {id}");
            assert!(labels.insert(label), "duplicate label: {label}");
            assert_eq!(canonical_language(id), id);
            let supported = supports_language_highlighting(id);
            assert_eq!(
                supported,
                id != "text" && !UNSUPPORTED_CODE_LANGUAGES.contains(&id),
                "{id}"
            );
        }
        assert_eq!(canonical_language(" golang "), "go");
        assert_eq!(canonical_language(" Visual-Basic "), "vbscript");
        assert_eq!(canonical_language("TSX"), "tsx");
        assert_eq!(canonical_language("shellscript"), "bash");
        assert_eq!(canonical_language("unknown-grammar"), "unknown-grammar");
    }

    #[test]
    fn common_additional_languages_and_ambiguous_extensions_have_colors() {
        for (language, code) in [
            ("java", "public class Hello { public static void main(String[] args) { System.out.println(42); } }"),
            ("c", "#include <stdio.h>\nint main(void) { printf(\"hello\"); return 0; }"),
            ("go", "package main\nfunc main() { fmt.Println(\"hello\") }"),
            ("tsx", "const Hello = () => <div className=\"greeting\">Hello</div>;"),
            ("fsharp", "let message = \"hello\""),
            ("mdx", "import Demo from \"./demo\";\n\n# Heading\n\n<Demo label=\"hello\" />\n\n{1 + 2}"),
            ("pug", "doctype html\nhtml\n  body\n    h1 Hello\n    p(class=\"example\") World"),
            ("mermaid", "graph TD\n  A[Start] --> B[End]\n  %% comment"),
            ("wasm", "(module (func (result i32) i32.const 42))"),
            ("powershell", "$name = \"hello\"; Write-Host $name # comment"),
            ("vbscript", "Dim message\nmessage = \"hello\"\nMsgBox message ' comment"),
        ] {
            let source = format!("```{language}\n{code}\n```\n");
            let colors: HashSet<_> = highlight_markdown(&source).iter().map(|s| s.rgb).collect();
            assert!(colors.len() >= 2, "{language}: {colors:?}");
        }
        let syntaxes = &resources().0;
        assert_eq!(syntax_for_language(syntaxes, "c").unwrap().name, "C");
        assert_eq!(syntax_for_language(syntaxes, "sass").unwrap().name, "Sass");
        assert_eq!(syntax_for_language(syntaxes, "fs").unwrap().name, "F#");
    }

    #[test]
    fn languages_use_real_grammars_and_distinct_token_colors() {
        for (language, code) in [
            (
                "rust",
                "fn main() { let count = 42; println!(\"안녕 😀\"); } // comment",
            ),
            ("python", "def greet(name):\n    return \"안녕\" # comment"),
            ("javascript", "const greeting = \"hello\"; // comment"),
            (
                "typescript",
                "interface Person { name: string; age: number; }\nconst active: boolean = true;",
            ),
            ("json", "{\"name\": \"안녕\", \"count\": 42, \"ok\": true}"),
            ("bash", "#!/bin/bash\necho \"hello $USER\" # comment"),
        ] {
            let source = format!("앞 문단 😀\n\n```{language}\n{code}\n```\n\n뒷 문단");
            let spans = highlight_markdown(&source);
            assert!(
                spans.iter().map(|s| s.rgb).collect::<HashSet<_>>().len() >= 3,
                "{language}"
            );
            for span in &spans {
                assert!(source.is_char_boundary(span.bytes.start));
                assert!(source.is_char_boundary(span.bytes.end));
                assert!(span.bytes.start >= source.find(code).unwrap());
                assert!(span.bytes.end <= source.find(code).unwrap() + code.len() + 1);
            }
        }
    }

    #[test]
    fn aliases_and_case_match_canonical_language() {
        let (syntaxes, _) = resources();
        for aliases in [
            ["rust", "rs", "RUST"],
            ["python", "py", "python3"],
            ["javascript", "js", "node"],
            ["typescript", "ts", "TYPESCRIPT"],
            ["bash", "sh", "shell"],
        ] {
            let name = &syntax_for_language(syntaxes, aliases[0]).unwrap().name;
            for alias in &aliases[1..] {
                assert_eq!(name, &syntax_for_language(syntaxes, alias).unwrap().name);
            }
        }
    }

    #[test]
    fn multiline_state_survives_crlf_unicode_and_unclosed_fence() {
        let source =
            "😀\r\n\r\n~~~rust\r\n/* 시작\r\ncontinued comment */\r\nlet value = \"한글 😀\";\r\n";
        let spans = highlight_markdown(source);
        assert_eq!(
            color_at(source, &spans, "시작"),
            color_at(source, &spans, "continued")
        );
        assert_ne!(
            color_at(source, &spans, "continued"),
            color_at(source, &spans, "let")
        );
        assert_ne!(
            color_at(source, &spans, "let"),
            color_at(source, &spans, "한글")
        );
    }

    #[test]
    fn no_prose_inline_unknown_or_plain_language_highlighting() {
        for source in [
            "# fn main()\n`let x = 42;`",
            "```\nplain unrecognized words\n```",
            "```text\nlet x = 42;\n```",
            "```unknown-grammar\nlet x = 42;\n```",
            "    let x = 42;\n",
        ] {
            assert!(highlight_markdown(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn nested_blocks_keep_exact_source_ranges() {
        for source in [
            "> ```rust\n> let value = \"한글\";\n> ```\n",
            "- item\n\n  ```python\n  print(\"hello\")\n  ```\n",
        ] {
            let spans = highlight_markdown(source);
            assert!(!spans.is_empty());
            for span in spans {
                assert!(!source[span.bytes].contains('>'));
            }
        }
    }
}
