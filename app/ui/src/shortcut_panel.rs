use crate::{font, mtm, rect, scrollbar, sidebar_layout::DEFAULT_RIGHT_WIDTH, text_label, theme, view};
use objc2::rc::Retained;
use objc2::{msg_send, MainThreadOnly};
use objc2_app_kit::{NSAutoresizingMaskOptions as Mask, NSButton, NSColor, NSFontAttributeName, NSForegroundColorAttributeName, NSScrollView, NSTextAlignment, NSView};
use objc2_foundation::{NSMutableAttributedString, NSPoint, NSRange};

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    ("노트 작성", &[
        ("새 노트", "⌘N"), ("저장", "⌘S"), ("2×2 표 삽입", "⌘⇧A"),
        ("선택 영역 확장", "⌘D"), ("실행 취소", "⌘Z"), ("다시 실행", "⌘⇧Z"),
        ("잘라내기", "⌘X"), ("복사", "⌘C"), ("붙여넣기", "⌘V"), ("모두 선택", "⌘A"),
    ]),
    ("선택한 텍스트", &[
        ("굵게", "⌘B  /  **가**, __가__"), ("기울임", "⌘I  /  *가*, _가_"), ("밑줄", "⌘U"), ("취소선", "⌘⇧S  /  ~~텍스트~~"),
        ("인라인 코드", "⌘E  /  `텍스트`"), ("링크 추가", "⌘K  /  [이름](주소)"),
    ]),
    ("문단과 블록", &[
        ("일반 문단", "⌘⌥0"), ("제목 1", "⌘⌥1  /  # + Space"),
        ("제목 2", "⌘⌥2  /  ## + Space"), ("제목 3", "⌘⌥3  /  ### + Space"),
        ("체크리스트", "⌘⌥4  /  - [ ], - [x] + Space"),
        ("불릿 목록", "⌘⌥5  /  -, *, + + Space"), ("번호 목록", "⌘⌥6  /  1. + Space"),
        ("코드 블록", "⌘⌥8  /  ``` + Enter"),
        ("불릿 들여쓰기 · 내어쓰기", "Tab · ⇧Tab"), ("코드 블록에서 나가기", "⇧Enter"),
        ("표 셀 안에서 줄바꿈", "Enter"), ("다음 셀 · 이전 셀", "Tab · ⇧Tab"),
    ]),
    ("PDF 필기", &[
        ("필기 저장", "⌘S"), ("실행 취소 · 다시 실행", "⌘Z · ⌘⇧Z"),
        ("선택한 필기 삭제", "Delete"), ("선택 도구로 돌아가기", "Esc"),
    ]),
    ("지식지도", &[
        ("개념 검색", "⌘K · ⌃K"), ("검색 결과로 이동", "Enter"),
        ("확대 · 축소", "+ · −"), ("전체 보기", "0 · R"), ("선택 해제", "Esc"),
    ]),
    ("학습도우미", &[("질문 전송", "⌘Enter"), ("줄바꿈", "Enter")]),
];

pub unsafe fn new(close: &NSButton) -> Retained<NSView> {
    let width = DEFAULT_RIGHT_WIDTH;
    let height = 600.;
    let panel = view(rect(0., 0., width, height));
    theme::canvas(&panel);
    if let Some(layer) = panel.layer() {
        let _: () = msg_send![&layer, setCornerRadius: 22.0f64];
        let _: () = msg_send![&layer, setMasksToBounds: true];
    }
    let title = text_label("단축키", rect(20., height - 44., 180., 24.), 18., true);
    title.setAutoresizingMask(Mask::ViewMinYMargin);
    panel.addSubview(&title);
    close.setFrame(rect(width - 46., height - 45., 32., 32.));
    close.setAutoresizingMask(Mask::ViewMinYMargin);
    panel.addSubview(close);

    let document_width = width - 44.;
    let header_divider = theme::soft_divider(rect(16., height - 53., document_width - 8., 2.), true);
    header_divider.setAutoresizingMask(Mask::ViewMinYMargin);
    panel.addSubview(&header_divider);

    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(12., 12., width - 24., height - 65.));
    scroll.setAutoresizingMask(Mask::ViewHeightSizable);
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDrawsBackground(false);
    scrollbar::style(&scroll);
    panel.addSubview(&scroll);
    let document_height = SECTIONS.iter().map(|(_, rows)| 40. + rows.iter().map(|(_, keys)| if keys.contains('\n') { 46. } else { 31. }).sum::<f64>()).sum::<f64>();
    let document = view(rect(0., 0., document_width, document_height));
    let mut y = document_height;
    for &(heading, rows) in SECTIONS {
        y -= 40.;
        let label = text_label(heading, rect(8., y + 4., 200., 21.), 13., true);
        label.setTextColor(Some(&theme::soft_selection_ink()));
        document.addSubview(&label);
        for (index, &(action, keys)) in rows.iter().enumerate() {
            let row_height = if keys.contains('\n') { 46. } else { 31. };
            y -= row_height;
            let expanded = keys.contains("  /  ");
            let key_x = if expanded { 100. } else { 181. };
            document.addSubview(&text_label(action, rect(8., y + (row_height - 20.) / 2., key_x - 10., 20.), 13., false));
            let key = text_label(keys, rect(key_x, y + 5., document_width - key_x - 13., row_height - 10.), 13., true);
            key.setAlignment(NSTextAlignment::Right);
            if keys.contains('\n') { key.setMaximumNumberOfLines(2); }
            if expanded {
                let styled: Retained<NSMutableAttributedString> = msg_send![&key.attributedStringValue(), mutableCopy];
                for (pattern, is_separator, offset) in [("  /  ", true, 2), (" + ", false, 1), ("\n+ ", false, 1)] {
                    for (byte_index, _) in keys.match_indices(pattern) {
                        let position = keys[..byte_index].encode_utf16().count() + offset;
                        let range = NSRange::new(position, 1);
                        if is_separator {
                            let color = NSColor::colorWithSRGBRed_green_blue_alpha(0.357, 0.357, 0.839, 0.28);
                            styled.addAttribute_value_range(NSForegroundColorAttributeName, &color, range);
                            styled.addAttribute_value_range(NSFontAttributeName, &font::bold(14.), range);
                        } else {
                            styled.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::secondaryLabelColor(), range);
                            styled.addAttribute_value_range(NSFontAttributeName, &font::bold(11.5), range);
                        }
                    }
                }
                for (byte_index, _) in keys.match_indices(',') {
                    let range = NSRange::new(keys[..byte_index].encode_utf16().count(), 1);
                    styled.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::secondaryLabelColor(), range);
                    styled.addAttribute_value_range(NSFontAttributeName, &font::bold(10.), range);
                }
                key.setAttributedStringValue(&styled);
            }
            document.addSubview(&key);
            let section_end = index + 1 == rows.len();
            let thickness = if section_end { 2. } else { 1. };
            document.addSubview(&theme::soft_divider(rect(4., y, document_width - 8., thickness), section_end));
        }
    }
    scroll.setDocumentView(Some(&document));
    document.scrollPoint(NSPoint::new(0., document_height));
    panel.setHidden(true);
    panel
}
