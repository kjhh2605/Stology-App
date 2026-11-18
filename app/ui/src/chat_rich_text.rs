//! Local, read-only presentation. Stored/copyable answers remain Markdown.
use super::*;
use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

pub fn content(source: &str) -> String {
    let mut diagram = false;
    let events = Parser::new_ext(source, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS).map(|e| match e {
        // Model HTML is text, never executable markup. Remote images are alt text.
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        Event::Start(Tag::Image { .. }) => Event::Html("<span class=\"image-alt\">".into()),
        Event::End(TagEnd::Image) => Event::Html("</span>".into()),
        Event::Start(Tag::Link { dest_url, .. }) => {
            let mut escaped = String::new();
            html::push_html(&mut escaped, [Event::Text(dest_url)].into_iter());
            Event::Html(format!("<span class=\"link\">{escaped} · ").into())
        }
        Event::End(TagEnd::Link) => Event::Html("</span>".into()),
        Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if info.split_whitespace().next().is_some_and(|v| v.eq_ignore_ascii_case("mermaid")) => {
            diagram = true;
            Event::Html("<pre class=\"diagram-source\"><code>".into())
        }
        Event::End(TagEnd::CodeBlock) if diagram => {
            diagram = false;
            Event::Html("</code></pre>".into())
        }
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}
fn shell() -> String {
    format!(r#"<!doctype html><html lang="ko"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-stology'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; base-uri 'none'; form-action 'none'"><style>
html,body{{margin:0;padding:0;background:transparent;color:#252735;font:13px/1.65 -apple-system,BlinkMacSystemFont,sans-serif;overflow-wrap:anywhere}}
#answer{{display:flow-root}}p{{margin:0 0 10px}}h1,h2,h3,h4,h5,h6{{line-height:1.4;margin:16px 0 8px;font-weight:700}}h1{{font-size:20px}}h2{{font-size:17px}}h3{{font-size:15px}}h4,h5,h6{{font-size:13px}}#answer>:first-child{{margin-top:0}}#answer>:last-child{{margin-bottom:0}}
ul,ol{{padding-left:22px;margin:8px 0}}li{{margin:4px 0}}li>p{{margin:0}}blockquote{{border-left:3px solid #a6a3d7;margin:10px 0;padding:2px 10px;color:#626274}}code{{font:12px/1.6 ui-monospace,monospace;background:#ececf4;padding:2px 4px;border-radius:4px}}pre{{background:#eeeef5;border-radius:8px;padding:10px;overflow:auto;white-space:pre;margin:10px 0}}pre code{{padding:0;background:none}}table{{border-collapse:collapse;font-size:12px;width:100%;margin:10px 0}}th,td{{border:1px solid #d8d8e6;padding:6px;text-align:left;min-width:32px}}th{{background:#eeeef6}}hr{{border:0;border-top:1px solid #d8d8e6;margin:14px 0}}.link{{color:#5554a0}}.diagram{{overflow:auto;margin:10px 0;padding:8px;background:#f7f7fc;border-radius:8px}}.diagram svg{{max-width:100%;height:auto;display:block;margin:auto}}summary{{color:#777;font-size:11px;cursor:pointer}}.notice{{font-size:11px;color:#777}}
</style></head><body><main id="answer"></main><script nonce="stology">{}</script><script nonce="stology">{}</script></body></html>"#,
        include_str!("../../assets/chat/mermaid.min.js").replace("</script", "<\\/script"), include_str!("chat_rich_text.js"))
}
thread_local! {
    static DATA_STORE: Retained<AnyObject> = unsafe {
        let cls = objc2::runtime::AnyClass::get(c"WKWebsiteDataStore").unwrap();
        msg_send![cls, nonPersistentDataStore]
    };
}
/// WKWebView는 스크롤할 것이 없어도 휠 이벤트를 삼켜 대화 목록이 멈춘다.
/// 세로 휠은 바깥 스크롤 뷰로 넘기고, 가로 휠(넓은 표·코드)만 웹뷰가 처리한다.
fn wheel_web_view_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, ClassBuilder};
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let parent = AnyClass::get(c"WKWebView").expect("system WebKit");
        let mut builder = ClassBuilder::new(c"StologyWheelWebView", parent).expect("unique class");
        extern "C" fn wheel(this: &AnyObject, _cmd: Sel, event: &NSEvent) {
            unsafe {
                if event.scrollingDeltaY().abs() >= event.scrollingDeltaX().abs() {
                    let next: Option<Retained<NSResponder>> = msg_send![this, nextResponder];
                    if let Some(next) = next {
                        let _: () = msg_send![&next, scrollWheel: event];
                        return;
                    }
                }
                let parent = AnyClass::get(c"WKWebView").expect("system WebKit");
                let _: () = msg_send![super(this, parent), scrollWheel: event];
            }
        }
        builder.add_method(sel!(scrollWheel:), wheel as extern "C" fn(_, _, _));
        builder.register()
    })
}
pub struct RichText {
    pub view: Retained<NSView>,
    sent: String,
    pub height: f64,
    pub ready: bool,
}
impl RichText {
    pub unsafe fn new() -> Self {
        let cls = wheel_web_view_class();
        let config_cls = objc2::runtime::AnyClass::get(c"WKWebViewConfiguration").unwrap();
        let config: Retained<AnyObject> = msg_send![config_cls, new];
        DATA_STORE.with(|store| { let _: () = msg_send![&config, setWebsiteDataStore:&**store]; });
        let alloc: objc2::rc::Allocated<NSView> = msg_send![cls, alloc];
        let view: Retained<NSView> = msg_send![alloc, initWithFrame:rect(0.,0.,240.,24.) configuration:&*config];
        let _: () = msg_send![&view, setValue:&*NSNumber::new_bool(false) forKey:&*NSString::from_str("drawsBackground")];
        let _: Option<Retained<AnyObject>> = msg_send![&view, loadHTMLString:&*NSString::from_str(&shell()) baseURL:std::ptr::null::<NSURL>()];
        Self { view, sent: String::new(), height: 24., ready: false }
    }
    // Called from the existing chat refresh timer; no additional native timer.
    pub unsafe fn update(&mut self, body: &str, finished: bool) -> bool {
        let title: Option<Retained<NSString>> = msg_send![&self.view, title];
        let Some(title) = title.map(|t| t.to_string()).filter(|t| t.starts_with("stology:")) else { return false };
        let mut changed = !self.ready;
        self.ready = true;
        if let Some(height) = title.strip_prefix("stology:").and_then(|v| v.parse::<f64>().ok()).filter(|h| h.is_finite() && *h > 0.) {
            if (height-self.height).abs() > 0.5 { self.height = height; changed = true; }
        }
        let key = format!("{finished}:{body}");
        if self.sent != key {
            let script = format!("window.presentAnswer({}, {})", serde_json::to_string(&content(body)).unwrap(), finished);
            let _: () = msg_send![&self.view, evaluateJavaScript:&*NSString::from_str(&script) completionHandler:std::ptr::null::<AnyObject>()];
            self.sent = key;
        }
        changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn readable_blocks_and_diagram() {
        let html = content("# 제목\n\n**강조**와 `코드`\n\n- 하나\n\n|A|B|\n|-|-|\n|1|2|\n\n```mermaid\nflowchart TD\n A --> B\n```\n");
        for expected in ["<h1>","<strong>","<code>","<ul>","<table>","diagram-source", "A --&gt; B"] { assert!(html.contains(expected),"{expected}: {html}"); }
    }
    #[test] fn untrusted_markup_and_remote_resources_are_inert() {
        let html = content("<script>alert(1)</script>\n\n![alt](https://example.org/a.png) [click](javascript:alert%281%29)");
        for forbidden in ["<script", "<img", "href=", "src="] { assert!(!html.contains(forbidden),"{html}"); }
        assert!(html.contains("&lt;script&gt;"));
    }
    #[test] fn unfinished_fence_and_unicode_survive() {
        assert!(content("```mermaid\n한글 <노드>").contains("한글 &lt;노드&gt;"));
        assert!(content("```rust\nlet a = 1;\n```").contains("language-rust"));
    }
}
