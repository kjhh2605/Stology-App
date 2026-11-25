//! Exercise native tool actions, pointer events, label editing and SVG persistence.
#![allow(dead_code, deprecated)]
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
    sel, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::cell::RefCell;
#[path = "../src/diagram_paint.rs"]
pub mod diagram_paint;
#[path = "../src/font.rs"]
mod font;
mod editor {
    pub use crate::diagram_paint;
}
#[path = "../src/pdf_annotation_tooltip.rs"]
pub mod tooltip;
mod pdf_annotations {
    pub use crate::tooltip;
}
#[path = "../src/diagram_canvas.rs"]
mod diagram_canvas;
#[path = "support/controls.rs"]
#[allow(dead_code)]
mod controls;
use controls::{button, choose, key};
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().unwrap()
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
unsafe fn view(frame: NSRect) -> Retained<NSView> {
    NSView::initWithFrame(NSView::alloc(mtm()), frame)
}
unsafe fn find_view<T: objc2::ClassType + objc2::DowncastTarget>(root: &NSView, name: &str) -> Option<Retained<T>> {
    if root.accessibilityLabel().is_some_and(|v| v.to_string() == name) {
        return Retained::retain(root as *const NSView as *mut NSView)?.downcast::<T>().ok();
    }
    root.subviews().iter().find_map(|child| find_view::<T>(&child, name))
}
unsafe fn palette_control<T: objc2::ClassType + objc2::DowncastTarget>(name: &str) -> Retained<T> {
    NSApplication::sharedApplication(mtm()).windows().iter()
        .filter(|w| w.isVisible()).filter_map(|w| w.contentView())
        .find_map(|root| find_view::<T>(&root, name)).unwrap_or_else(|| panic!("missing palette control {name}"))
}
unsafe fn set_color(root: &NSView, window: &NSWindow, tag:isize, code:&str) {
    assert!(!button(root,tag).isHidden());
    choose(root,tag);
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.15));
    palette_control::<NSButton>(code).performClick(None);
    choose(root,tag);
    window.makeKeyAndOrderFront(None);
}
unsafe fn mouse(overlay: &NSView, window: &NSWindow, kind: NSEventType, location: NSPoint) {
    mouse_with(overlay, window, kind, location, NSEventModifierFlags::empty());
}
unsafe fn mouse_with(overlay: &NSView, window: &NSWindow, kind: NSEventType, location: NSPoint, flags: NSEventModifierFlags) {
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, location, flags, 0., window.windowNumber(), None, 0, 1, 1.).unwrap();
    let _: () = match kind {
        NSEventType::LeftMouseDown => msg_send![overlay,mouseDown:&*event],
        NSEventType::LeftMouseDragged => msg_send![overlay,mouseDragged:&*event],
        _ => msg_send![overlay,mouseUp:&*event],
    };
}
// A real press: the button's own tracking loop takes the queued mouse-up.
unsafe fn press(app: &NSApplication, window: &NSWindow, button: &NSButton, clicks: isize) {
    let at = button.convertPoint_toView(button.bounds().mid(), None);
    let event = |kind| NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, at, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, clicks, 1.).unwrap();
    app.postEvent_atStart(&event(NSEventType::LeftMouseUp), true);
    button.mouseDown(&event(NSEventType::LeftMouseDown));
}
unsafe fn menu_item(tag: isize) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm());
    item.setTag(tag);
    item
}
unsafe fn item_title(menu: &NSMenu, index: isize) -> String {
    menu.itemAtIndex(index).unwrap().title().to_string()
}

fn main() {
    unsafe {
        let app = NSApplication::sharedApplication(mtm());
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        app.finishLaunching();
        let frame = rect(0., 0., 760., 440.);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm()),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        let canvas = diagram_canvas::Canvas::new(Default::default(), "제목 없음".into(), NSRange::new(0, 0), None);
        let divider = |frame: NSRect| rect(28., frame.max().y + 1., frame.size.width - 42., 1.);
        canvas.layout(frame, divider(frame));
        window.setContentView(Some(&canvas.root));
        window.makeKeyAndOrderFront(None);
        let scroll = canvas
            .root
            .subviews()
            .objectAtIndex(0)
            .downcast::<NSScrollView>()
            .unwrap();
        assert!(!scroll.hasHorizontalScroller() && !scroll.hasVerticalScroller());
        let surface = scroll.documentView().unwrap();
        canvas.focus();
        let drag = |a: [f64; 2], b: [f64; 2]| {
            for (kind, [x, y]) in [
                (NSEventType::LeftMouseDown, a),
                (NSEventType::LeftMouseDragged, b),
                (NSEventType::LeftMouseUp, b),
            ] {
                mouse(
                    &surface,
                    &window,
                    kind,
                    surface.convertPoint_toView(NSPoint::new(x, y), None),
                );
            }
        };
        let zoom = |key: &str| {
            let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown,NSPoint::ZERO,NSEventModifierFlags::Command,0.,window.windowNumber(),None,&NSString::from_str(key),&NSString::from_str(key),false,0).unwrap();
            assert!(surface.performKeyEquivalent(&event));
        };
        zoom("+");
        assert_eq!(scroll.magnification(), 1.25);
        // The grabbed canvas point follows the pointer even after the first scroll.
        let original_origin = scroll.contentView().bounds().origin;
        let anchor = surface.visibleRect().mid();
        let start = surface.convertPoint_toView(anchor, None);
        mouse(&surface, &window, NSEventType::LeftMouseDown, start);
        for delta in [25., 50., 10.] {
            let pointer = NSPoint::new(start.x - delta, start.y + delta);
            mouse(&surface, &window, NSEventType::LeftMouseDragged, pointer);
            let actual = surface.convertPoint_toView(anchor, None);
            assert!((actual.x - pointer.x).abs() < 1. && (actual.y - pointer.y).abs() < 1.);
        }
        mouse(&surface, &window, NSEventType::LeftMouseUp, NSPoint::new(start.x - 10., start.y + 10.));
        assert!(!canvas.changed() && !button(&canvas.root, 7).isEnabled());
        let map = canvas.root.subviews().iter().find(|v| {
            v.accessibilityLabel().is_some_and(|s| s.to_string() == "다이어그램 미니맵")
        }).expect("minimap in canvas");
        for layout in [rect(0., 0., 512., 528.), frame] {
            let line = divider(layout);
            canvas.layout(layout, line);
            let first_tool = button(&canvas.root, 0).frame();
            assert_eq!(first_tool.origin.x, line.origin.x);
            assert_eq!(line.origin.y - first_tool.max().y, 8.);
            assert!(!map.isHidden());
            assert_eq!(map.frame().max().x, line.max().x);
            assert_eq!(map.frame().origin.y, layout.size.width - line.max().x);
            assert!(map.frame().max().y < button(&canvas.root, 22).frame().origin.y);
        }
        let click = map.convertPoint_toView(map.bounds().mid(), None);
        mouse(&map, &window, NSEventType::LeftMouseDown, click);
        let center = surface.visibleRect().mid();
        assert!((center.x - 1200.).abs() < 3.);
        let end = NSPoint::new(click.x + 12., click.y - 8.);
        mouse(&map, &window, NSEventType::LeftMouseDragged, end);
        mouse(&map, &window, NSEventType::LeftMouseUp, end);
        let moved = surface.visibleRect().mid();
        assert!(moved.x > center.x + 100. && moved.y > center.y + 60.);
        assert!(!canvas.changed() && !button(&canvas.root, 7).isEnabled());
        scroll.contentView().scrollToPoint(original_origin);
        choose(&canvas.root, 1);
        drag([20., 30.], [20., 30.]);
        assert!(canvas.document().0.is_empty());
        drag([20., 30.], [120., 100.]);
        let zoomed = canvas.document();
        assert!((zoomed.0[0].b[0] - 120.).abs() < 1e-8 && (zoomed.0[0].b[1] - 100.).abs() < 1e-8);
        assert_eq!(
            diagram_paint::image(zoomed.clone()).size(),
            NSSize::new(102., 72.)
        );
        zoom("-");
        assert_eq!(scroll.magnification(), 1.);
        assert_eq!(zoomed, canvas.document());
        key(&window, "\r", 36);
        let field = surface
            .subviews()
            .objectAtIndex(0)
            .downcast::<NSTextField>()
            .unwrap();
        // Simulate the active IME field editor, including a provisional final consonant.
        let input = field
            .currentEditor()
            .unwrap()
            .downcast::<NSTextView>()
            .unwrap();
        let marked = NSString::from_str("한");
        let _: () = msg_send![&input,setMarkedText:&*marked,selectedRange:NSRange::new(1,0),replacementRange:NSRange::new(usize::MAX,0)];
        for command in [sel!(insertNewline:), sel!(cancelOperation:)] {
            let handled: bool =
                msg_send![&*surface,control:&*field,textView:&*input,doCommandBySelector:command];
            assert!(!handled && field.superview().is_some());
        }
        let _: () = msg_send![&input, unmarkText];
        let press_return = |flags| {
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown, NSPoint::ZERO, flags, 0.,
            window.windowNumber(), None, &NSString::from_str("\r"), &NSString::from_str("\r"), false, 36).unwrap();
        app.postEvent_atStart(&event, true);
        let event = window.nextEventMatchingMask(NSEventMask::KeyDown).unwrap();
        app.sendEvent(&event);
        };
        press_return(NSEventModifierFlags::Shift);
        assert!(field.superview().is_some());
        let _: () = msg_send![&input,insertText:&*NSString::from_str("글 <도형>"),replacementRange:NSRange::new(usize::MAX,0)];
        press_return(NSEventModifierFlags::empty());
        assert!(field.superview().is_none());
        assert_eq!(canvas.document().0[0].text, "한\n글 <도형>");
        choose(&canvas.root, 2);
        drag([210., 30.], [330., 110.]);
        key(&window, "a", 0);
        drag([60., 60.], [240., 60.]);
        let original = canvas.document();
        let arrow = &original.0[2];
        assert!(arrow.from.is_some() && arrow.to.is_some());
        // Dragging the body of an arrow attached at both ends changes nothing.
        let (tail, tip) = arrow.ends(&original.0);
        drag([(tail[0] + tip[0]) / 2., (tail[1] + tip[1]) / 2.], [165., 130.]);
        assert_eq!(canvas.document(), original);
        drag([50., 60.], [80., 90.]);
        drag([150., 130.], [160., 155.]);
        choose(&canvas.root, 20);
        let panel = palette_control::<NSButton>("옅은 무채색 #FFFFFF").superview().unwrap();
        let anchored = panel.frame();
        assert_eq!(anchored.max().y, surface.enclosingScrollView().unwrap().frame().max().y);
        for tag in [21, 22, 20] {
            choose(&canvas.root, tag);
            assert!(!panel.isHidden());
            assert_eq!(panel.frame(), anchored);
            if tag == 21 {
                palette_control::<NSButton>("채우기 없음").performClick(None);
                assert_eq!(canvas.document().0[0].fill, "none");
            } else {
                assert!(!palette_control::<NSButton>("옅은 무채색 #FFFFFF").isHidden());
            }
        }
        choose(&canvas.root, 20);
        set_color(&canvas.root, &window, 20, "선명한 빨강 #D6455D");
        set_color(&canvas.root, &window, 21, "옅은 보라 #E7E5FF");
        set_color(&canvas.root, &window, 22, "짙은 파랑 #2D4D80");
        let current = canvas.document();
        assert_eq!(current.0[0].b, [160., 155.]);
        assert_ne!(arrow.ends(&original.0), current.0[2].ends(&current.0));
        choose(&canvas.root, 9);
        assert_eq!(canvas.document().0.len(), 1);
        choose(&canvas.root, 7);
        assert_eq!(canvas.document(), current);
        // A reverse marquee includes both shapes and their arrow, but only
        // part of the pen stroke. Selecting alone must preserve redo history.
        choose(&canvas.root, 5);
        drag([310., 140.], [420., 170.]);
        let with_pen = canvas.document();
        zoom("+");
        key(&window, "m", 46);
        assert_eq!(button(&canvas.root, 18).state(), 1);
        drag([345., 160.], [10., 10.]);
        assert_eq!(canvas.document(), with_pen);
        drag([75., 90.], [95., 130.]);
        let moved = canvas.document();
        for i in 0..2 {
            assert!((moved.0[i].a[0] - with_pen.0[i].a[0] - 20.).abs() < 1e-8);
            assert!((moved.0[i].a[1] - with_pen.0[i].a[1] - 40.).abs() < 1e-8);
        }
        // The arrow follows its shapes rather than being moved itself.
        let (was, now) = (with_pen.0[2].ends(&with_pen.0), moved.0[2].ends(&moved.0));
        assert!((now.0[0] - was.0[0] - 20.).abs() < 1e-8 && (now.1[1] - was.1[1] - 40.).abs() < 1e-8);
        assert_eq!(moved.0[3], with_pen.0[3]);
        assert_eq!(moved.0[2].from, with_pen.0[2].from);
        assert_eq!(moved.0[2].to, with_pen.0[2].to);
        set_color(&canvas.root, &window, 20, "선명한 초록 #1F8A5B");
        set_color(&canvas.root, &window, 21, "옅은 노랑 #FFF1C2");
        let styled = canvas.document();
        assert!(styled.0[..3].iter().all(|e| e.ink == "#1F8A5B"));
        assert!(styled.0[..2].iter().all(|e| e.fill == "#FFF1C2"));
        assert_eq!(styled.0[2].fill, "none");
        assert_eq!(styled.0[0].label_color(), "#2D4D80");
        assert_eq!(styled.0[3], with_pen.0[3]);
        key(&window, "\u{7f}", 51);
        assert_eq!(canvas.document().0, vec![with_pen.0[3].clone()]);
        choose(&canvas.root, 7);
        assert_eq!(canvas.document(), styled);
        drag([5., 5.], [5., 5.]);
        assert!(!button(&canvas.root, 9).isEnabled());
        assert!(button(&canvas.root, 8).isEnabled());
        choose(&canvas.root, 8);
        assert_eq!(canvas.document().0.len(), 1);
        choose(&canvas.root, 6);
        drag([90., 170.], [90., 170.]);
        let label = surface.subviews().objectAtIndex(0).downcast::<NSTextField>().unwrap();
        label.setStringValue(&NSString::from_str("독립 텍스트"));
        let before_color = canvas.document();
        assert!(button(&canvas.root, 20).isHidden() && button(&canvas.root, 21).isHidden());
        
        assert_eq!(canvas.document(), before_color);
        set_color(&canvas.root, &window, 22, "연한 파랑 #A6BFF1");
        let text_only = canvas.document();
        assert_eq!(text_only.0.last().unwrap().label_color(), "#A6BFF1");
        choose(&canvas.root, 7);
        assert_eq!(canvas.document(), before_color);
        choose(&canvas.root, 8);
        assert_eq!(canvas.document(), text_only);
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("diagram.svg");
        std::fs::write(&file, current.svg("개념 연결").unwrap()).unwrap();
        let (restored, title) =
            stology_core::diagram_svg::Diagram::read(&std::fs::read_to_string(&file).unwrap())
                .unwrap();
        let reopened = diagram_canvas::Canvas::new(restored, title, NSRange::new(0, 0), None);
        assert_eq!(reopened.document(), current);
        assert_eq!(reopened.title, "개념 연결");
        assert!(diagram_paint::image(current).TIFFRepresentation().is_some());

        // Arrow kinds, pinned sides, the held angle, the tool lock and the arrow menu on a fresh canvas.
        let canvas = diagram_canvas::Canvas::new(Default::default(), "화살표".into(), NSRange::new(0, 0), None);
        canvas.layout(frame, divider(frame));
        window.setContentView(Some(&canvas.root));
        window.makeKeyAndOrderFront(None);
        let surface = canvas.root.subviews().objectAtIndex(0).downcast::<NSScrollView>().unwrap().documentView().unwrap();
        canvas.focus();
        let pull = |a: [f64; 2], b: [f64; 2], flags: NSEventModifierFlags| {
            for (kind, [x, y]) in [
                (NSEventType::LeftMouseDown, a),
                (NSEventType::LeftMouseDragged, b),
                (NSEventType::LeftMouseUp, b),
            ] {
                mouse_with(&surface, &window, kind, surface.convertPoint_toView(NSPoint::new(x, y), None), flags);
            }
        };
        let name = |tag: isize| NSAccessibility::accessibilityLabel(&*button(&canvas.root, tag)).unwrap().to_string();
        let plain = NSEventModifierFlags::empty();
        use stology_core::diagram_svg::{Head, Side};
        // Double-clicking a tool keeps it for several drawings; one click draws once.
        press(&app, &window, &button(&canvas.root, 1), 2);
        assert_eq!(name(1), "사각형 (R) · 고정");
        for (a, b) in [([40., 40.], [140., 100.]), ([300., 200.], [400., 260.])] {
            pull(a, b, plain);
        }
        assert!(button(&canvas.root, 1).state() == 1 && canvas.document().0.len() == 2);
        press(&app, &window, &button(&canvas.root, 2), 1);
        pull([600., 40.], [700., 100.], plain);
        assert!(button(&canvas.root, 0).state() == 1 && name(1) == "사각형 (R)");
        choose(&canvas.root, 9);
        // The arrow tool's menu keeps the tool the same way.
        let _: () = msg_send![&*surface, arrowKeep: &*menu_item(0)];
        assert!(button(&canvas.root, 4).state() == 1 && name(4) == "화살표 (A) · 고정");
        pull([90., 70.], [350., 230.], plain);
        pull([350., 201.], [92., 104.], plain);
        // Starting and ending in one shape draws nothing, and Shift levels a free end.
        pull([50., 50.], [130., 90.], plain);
        pull([500., 300.], [600., 310.], NSEventModifierFlags::Shift);
        assert_eq!(button(&canvas.root, 4).state(), 1);
        key(&window, "\u{1b}", 53);
        assert!(button(&canvas.root, 0).state() == 1 && name(4) == "화살표 (A)");
        let drawn = canvas.document();
        assert_eq!(drawn.0.len(), 5);
        assert_eq!((drawn.0[2].from, drawn.0[2].to, drawn.0[2].to_side), (Some(1), Some(2), None));
        assert_eq!((drawn.0[3].from_side, drawn.0[3].to_side), (Some(Side::Top), Some(Side::Bottom)));
        assert_eq!(drawn.0[3].ends(&drawn.0), ([350., 200.], [90., 100.]));
        assert!((drawn.0[4].b[0] - 600.).abs() < 1e-8 && (drawn.0[4].b[1] - 300.).abs() < 1e-8);
        assert!(drawn.0[2..].iter().all(|e| (e.start_head, e.end_head) == (Head::None, Head::Open)));

        // Choosing a kind draws it at once, and the A key repeats the kind chosen last.
        let _: () = msg_send![&*surface, arrowKind: &*menu_item(1)];
        assert_eq!(button(&canvas.root, 4).state(), 1);
        pull([500., 340.], [600., 340.], plain);
        assert_eq!(button(&canvas.root, 0).state(), 1);
        key(&window, "a", 0);
        pull([500., 380.], [600., 380.], plain);
        let _: () = msg_send![&*surface, arrowKind: &*menu_item(2)];
        pull([500., 420.], [600., 420.], plain);
        let kinds = canvas.document();
        assert!(kinds.0[5..7].iter().all(|e| (e.start_head, e.end_head) == (Head::Open, Head::Open)));
        assert_eq!((kinds.0[7].start_head, kinds.0[7].end_head), (Head::None, Head::None));
        assert!(kinds.0[5..].iter().all(|e| !e.dashed && !e.elbow));
        let _: () = msg_send![&*surface, arrowKind: &*menu_item(0)];
        key(&window, "\u{1b}", 53);
        for _ in 0..3 {
            choose(&canvas.root, 7);
        }
        assert_eq!(canvas.document(), drawn);

        // Right-clicking an arrow selects it and offers its line, route and direction.
        let right = |[x, y]: [f64; 2]| -> Option<Retained<NSMenu>> {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::RightMouseDown, surface.convertPoint_toView(NSPoint::new(x, y), None), plain, 0.,
                window.windowNumber(), None, 0, 1, 1.).unwrap();
            msg_send![&*surface, menuForEvent: &*event]
        };
        assert!(right([700., 400.]).is_none() && right([90., 70.]).is_none());
        assert!(!button(&canvas.root, 9).isEnabled());
        // The first arrow, away from where the second one crosses it.
        let (tail, tip) = drawn.0[2].ends(&drawn.0);
        let near_tail = [tail[0] + (tip[0] - tail[0]) * 0.1, tail[1] + (tip[1] - tail[1]) * 0.1];
        let menu = right(near_tail).unwrap();
        assert!(button(&canvas.root, 9).isEnabled());
        assert_eq!(menu.numberOfItems(), 6);
        assert_eq!((item_title(&menu, 0), item_title(&menu, 1)), ("점선으로 전환".into(), "꺾인 선으로 변환".into()));
        assert_eq!((item_title(&menu, 3), menu.itemAtIndex(3).unwrap().state()), ("단방향 화살표".into(), 1));
        menu.performActionForItemAtIndex(0);
        let menu = right(near_tail).unwrap();
        assert_eq!(item_title(&menu, 0), "실선으로 전환");
        menu.performActionForItemAtIndex(1);
        // The bent arrow no longer passes that point; its middle run is clear of the other arrow.
        let on_bend = |d: &stology_core::diagram_svg::Diagram| {
            let r = d.0[2].route(&d.0);
            [r[1][0] + (r[2][0] - r[1][0]) * 0.25, r[1][1] + (r[2][1] - r[1][1]) * 0.25]
        };
        let menu = right(on_bend(&canvas.document())).unwrap();
        assert_eq!(item_title(&menu, 1), "직선으로 변환");
        menu.performActionForItemAtIndex(4);
        let styled = canvas.document();
        let arrow = &styled.0[2];
        assert_eq!((arrow.start_head, arrow.end_head), (Head::Open, Head::Open));
        assert!(arrow.dashed && arrow.elbow && arrow.route(&styled.0).len() == 4);
        assert_eq!(styled.0[3], drawn.0[3]);
        let menu = right(on_bend(&styled)).unwrap();
        assert_eq!(menu.itemAtIndex(4).unwrap().state(), 1);
        menu.performActionForItemAtIndex(5);
        assert_eq!(canvas.document().0[2].end_head, Head::None);
        choose(&canvas.root, 7);
        assert_eq!(canvas.document(), styled);
        for _ in 0..3 {
            choose(&canvas.root, 7);
        }
        assert_eq!(canvas.document(), drawn);
        choose(&canvas.root, 8);
        assert!(canvas.document().0[2].dashed);
        // The arrow options that used to sit beside the colors are gone.
        assert!(canvas.root.subviews().iter().all(|v| {
            let tag: isize = msg_send![&**v, tag];
            tag != 23
        }));

        // A label breaks the line it sits on, and every option survives the file.
        let mut labelled = styled.clone();
        labelled.0[3].text = "이름".into();
        let bare = diagram_paint::image(styled.clone());
        let with_label = diagram_paint::image(labelled.clone());
        assert!(bare.TIFFRepresentation().is_some() && with_label.TIFFRepresentation().is_some());
        let svg = labelled.svg("화살표").unwrap();
        assert!(svg.contains("stroke-dasharray") && svg.matches("<text").count() == 1);
        assert_eq!(stology_core::diagram_svg::Diagram::read(&svg).unwrap().0, labelled);
        println!("diagram-smoke passed");
    }
}
