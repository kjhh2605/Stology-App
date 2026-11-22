//! The editor thumbnail and the canvas use identical AppKit paths and fonts.
use objc2::{
    rc::Retained,
    runtime::{AnyObject, Bool},
};
use objc2_app_kit::*;
use objc2_foundation::*;
use stology_core::diagram_svg::*;

pub fn color(hex: &str) -> Retained<NSColor> {
    let rgb = u32::from_str_radix(&hex[1..], 16).unwrap();
    NSColor::colorWithSRGBRed_green_blue_alpha(
        ((rgb >> 16) & 255) as f64 / 255.,
        ((rgb >> 8) & 255) as f64 / 255.,
        (rgb & 255) as f64 / 255.,
        1.,
    )
}
pub fn region(a: Point, b: Point) -> NSRect {
    NSRect::new(
        NSPoint::new(a[0].min(b[0]), a[1].min(b[1])),
        NSSize::new((b[0] - a[0]).abs(), (b[1] - a[1]).abs()),
    )
}
fn point([x, y]: Point) -> NSPoint {
    NSPoint::new(x, y)
}
// Measure the same paths and font layout that we paint. In particular, an
// attached arrow's stored endpoints need not coincide with its visible ends.
unsafe fn render_element(e: &Element, diagram: &Diagram, drawing: bool, line_width: f64) -> Option<NSRect> {
    let mut extents: Option<NSRect> = None;
    let mut include = |r: NSRect| {
        extents = Some(extents.map_or(r, |bounds| NSUnionRect(bounds, r)));
    };
    let attrs = (!e.text.is_empty()).then(|| {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[
                &*crate::font::regular(e.font_size) as &AnyObject,
                &*color(e.label_color()),
            ],
        )
    });
    let attrs = attrs.as_deref();
    let mut label = Vec::new();
    if attrs.is_some() {
        let [cx, cy] = e.label_point(&diagram.0);
        for (line, offset) in e.label_lines() {
            let text = NSString::from_str(line);
            let size = text.sizeWithAttributes(attrs);
            let x = if e.kind == Kind::Text {
                e.a[0]
            } else {
                cx - size.width / 2.
            };
            let origin = NSPoint::new(x, cy - size.height / 2. + offset);
            label.push((text, NSRect::new(origin, size)));
        }
    }
    let path = outline(e);
    // Arrow heads stay solid and may be filled, so they are stroked apart from the line.
    let heads = NSBezierPath::bezierPath();
    let mut filled = false;
    match e.kind {
        Kind::Arrow => {
            let gap = label
                .iter()
                .map(|(_, r)| NSInsetRect(*r, -4., 0.))
                .reduce(|a, b| NSUnionRect(a, b))
                .map(|r| ([r.origin.x, r.origin.y], [r.max().x, r.max().y]));
            let shape = e.arrow_shape(&diagram.0, gap);
            for run in &shape.lines {
                path.moveToPoint(point(run[0]));
                for p in &run[1..] {
                    path.lineToPoint(point(*p));
                }
            }
            if e.dashed {
                let dash = [6., 4.];
                path.setLineDash_count_phase(dash.as_ptr(), 2, 0.);
            }
            for (fill, [l, tip, r]) in shape.heads {
                heads.moveToPoint(point(l));
                heads.lineToPoint(point(tip));
                heads.lineToPoint(point(r));
                if fill {
                    heads.closePath();
                    filled = true;
                }
            }
        }
        Kind::Pen => {
            if let Some((first, rest)) = e.points.split_first() {
                path.moveToPoint(point(*first));
                for p in rest {
                    path.lineToPoint(point(*p));
                }
            }
        }
        _ => (),
    }
    if drawing && e.kind.shape() && e.fill != "none" {
        color(&e.fill).setFill();
        path.fill();
    }
    if drawing && filled {
        color(&e.ink).setFill();
        heads.fill();
    }
    for path in [path, heads] {
        if path.isEmpty() {
            continue;
        }
        path.setLineWidth(line_width);
        path.setLineJoinStyle(NSLineJoinStyle::Round);
        path.setLineCapStyle(NSLineCapStyle::Round);
        if drawing {
            color(&e.ink).setStroke();
            path.stroke();
        } else {
            include(NSInsetRect(path.bounds(), -0.85, -0.85));
        }
    }
    for (text, frame) in label {
        if drawing {
            text.drawAtPoint_withAttributes(frame.origin, attrs);
        } else {
            include(frame);
        }
    }
    extents
}
/// The closed outline of a shape; empty for every other kind.
pub unsafe fn outline(e: &Element) -> Retained<NSBezierPath> {
    let bounds = region(e.a, e.b);
    match e.kind {
        Kind::Rect => NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 6., 6.),
        Kind::Ellipse => NSBezierPath::bezierPathWithOvalInRect(bounds),
        Kind::Diamond => {
            let path = NSBezierPath::bezierPath();
            let [x, y] = e.center();
            path.moveToPoint(NSPoint::new(x, e.a[1]));
            for p in [[e.b[0], y], [x, e.b[1]], [e.a[0], y]] {
                path.lineToPoint(point(p));
            }
            path.closePath();
            path
        }
        _ => NSBezierPath::bezierPath(),
    }
}
pub unsafe fn element_bounds(e: &Element, diagram: &Diagram) -> NSRect {
    render_element(e, diagram, false, 1.7).unwrap_or_else(|| region(e.a, e.b))
}
pub unsafe fn paint(diagram: &Diagram) {
    paint_with_line_width(diagram, 1.7);
}
pub unsafe fn paint_with_line_width(diagram: &Diagram, width: f64) {
    for e in &diagram.0 {
        render_element(e, diagram, true, width);
    }
}
pub unsafe fn content_bounds(diagram: &Diagram) -> NSRect {
    let bounds = diagram
        .0
        .iter()
        .filter_map(|e| render_element(e, diagram, false, 1.7))
        .reduce(|a, b| NSUnionRect(a, b))
        .unwrap_or_else(|| NSRect::new(NSPoint::ZERO, NSSize::new(80., 48.)));
    NSIntegralRect(bounds)
}
pub unsafe fn image(diagram: Diagram) -> Retained<NSImage> {
    let bounds = content_bounds(&diagram);
    let NSPoint { x, y } = bounds.origin;
    let block = block2::RcBlock::new(move |_: NSRect| {
        NSGraphicsContext::saveGraphicsState_class();
        let transform = NSAffineTransform::transform();
        transform.translateXBy_yBy(-x, -y);
        transform.concat();
        paint(&diagram);
        NSGraphicsContext::restoreGraphicsState_class();
        Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(bounds.size, true, &block)
}
