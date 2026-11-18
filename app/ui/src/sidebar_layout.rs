//! Sidebar geometry and interruptible motion, independent of AppKit and storage.
use std::time::{Duration, Instant};

pub const DURATION: Duration = Duration::from_millis(240);
pub const MIN_LEFT_WIDTH: f64 = 196.;
pub const DEFAULT_RIGHT_WIDTH: f64 = 340.;
pub const MIN_RIGHT_WIDTH: f64 = 220.;
pub const RESIZE_WIDTH: f64 = 12.;

pub fn right_width(window_width: f64, left_openness: f64, requested: f64) -> f64 {
    right_width_with_left(window_width, left_openness, MIN_LEFT_WIDTH, requested)
}
pub fn right_width_with_left(window_width: f64, left_openness: f64, left_width: f64, requested: f64) -> f64 {
    // Exclude outer margins, the workspace sidebar and the resize gutter.
    // The right panel can occupy at most half of the remaining document area.
    let available = window_width - 28. - (left_width + RESIZE_WIDTH) * left_openness.clamp(0., 1.) - RESIZE_WIDTH;
    let max = (available / 2.).max(MIN_RIGHT_WIDTH);
    requested.clamp(MIN_RIGHT_WIDTH, max)
}

/// Resize until the content minimum, then translate the whole panel/veil.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragGeometry {
    pub width: f64,
    pub offset: f64,
    pub hide: bool,
}
pub fn drag_geometry(window: f64, left: f64, requested: f64) -> DragGeometry {
    DragGeometry {
        width: right_width(window, left, requested),
        offset: (MIN_RIGHT_WIDTH - requested).clamp(0., MIN_RIGHT_WIDTH),
        hide: requested < MIN_RIGHT_WIDTH,
    }
}

pub fn left_drag_geometry(window: f64, openness: [f64; 2], requested: f64, right: f64) -> DragGeometry {
    DragGeometry {
        width: Layout::with_widths(window, openness, requested, right).left_width,
        offset: (MIN_LEFT_WIDTH - requested).clamp(0., MIN_LEFT_WIDTH),
        hide: requested < MIN_LEFT_WIDTH,
    }
}

pub const HANDLE_WIDTH: f64 = 40.;
pub const GLOW_WIDTH: f64 = 56.;

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub left_x: f64,
    pub left_width: f64,
    pub right_x: f64,
    pub right_width: f64,
    pub center_x: f64,
    pub center_width: f64,
}
impl Layout {
    pub fn new(width: f64, openness: [f64; 2], requested_width: f64) -> Self {
        Self::with_widths(width, openness, MIN_LEFT_WIDTH, requested_width)
    }
    pub fn with_widths(width: f64, openness: [f64; 2], requested_left: f64, requested_right: f64) -> Self {
        let [left, right] = openness.map(|v| v.clamp(0., 1.));
        let preliminary_right = right_width(width, left, requested_right);
        let max_left = ((width - 28. - (preliminary_right + RESIZE_WIDTH) * right - RESIZE_WIDTH) / 2.).max(MIN_LEFT_WIDTH);
        let left_width = requested_left.clamp(MIN_LEFT_WIDTH, max_left);
        let right_width = right_width_with_left(width, left, left_width, requested_right);
        Self {
            right_width,
            left_width,
            left_x: 14. - (left_width + 28.) * (1. - left),
            right_x: width - 14. - right_width + (right_width + 28.) * (1. - right),
            center_x: 14. + (left_width + RESIZE_WIDTH) * left,
            center_width: width - 28. - (left_width + RESIZE_WIDTH) * left - (right_width + RESIZE_WIDTH) * right,
        }
    }
}

pub struct Motion {
    from: [f64; 2],
    pub target: [f64; 2],
    started: Instant,
    right_offset: f64,
    left_offset: f64,
}
impl Motion {
    pub fn new(from: [f64; 2], target: [f64; 2], started: Instant) -> Self {
        Self { from, target, started, right_offset: 0., left_offset: 0. }
    }
    pub fn with_left_offset(mut self, offset: f64) -> Self {
        self.left_offset = offset;
        self
    }
    pub fn left_offset(&self, now: Instant) -> f64 {
        let t = (now.saturating_duration_since(self.started).as_secs_f64()
            / DURATION.as_secs_f64()).clamp(0., 1.);
        self.left_offset * (1. - t).powi(3)
    }
    pub fn with_right_offset(mut self, offset: f64) -> Self {
        self.right_offset = offset;
        self
    }
    pub fn right_offset(&self, now: Instant) -> f64 {
        let t = (now.saturating_duration_since(self.started).as_secs_f64()
            / DURATION.as_secs_f64()).clamp(0., 1.);
        self.right_offset * (1. - t).powi(3)
    }
    pub fn sample(&self, now: Instant) -> ([f64; 2], bool) {
        let t = (now.saturating_duration_since(self.started).as_secs_f64()
            / DURATION.as_secs_f64()).clamp(0., 1.);
        let ease = 1. - (1. - t).powi(3);
        (std::array::from_fn(|i| self.from[i] + (self.target[i] - self.from[i]) * ease), t >= 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn left_drag_clamps_content_and_release_is_continuous() {
        for (requested, offset, hide) in [(240.,0.,false),(196.,0.,false),(195.,1.,true),(100.,96.,true),(0.,196.,true),(-50.,196.,true)] {
            let g = left_drag_geometry(1280.,[1.,1.],requested,340.);
            assert_eq!(g.width, requested.max(MIN_LEFT_WIDTH));
            assert_eq!((g.offset,g.hide),(offset,hide));
        }
        let now = Instant::now();
        for offset in [1.,96.,196.] {
            let motion = Motion::new([1.,1.],[0.,1.],now).with_left_offset(offset);
            let mut previous = 14. - offset;
            for step in 0..=24 {
                let time = now + DURATION * step / 24;
                let layout = Layout::with_widths(1280., motion.sample(time).0,196.,340.);
                let x = layout.left_x - motion.left_offset(time);
                assert!(x <= previous + 0.001);
                previous = x;
                if step == 0 {assert_eq!(x,14.-offset);}
                if step == 24 {assert!(x+196.<=0.);}
            }
        }
    }
    #[test]
    fn both_resizable_panels_leave_room_for_the_document() {
        for window in [1100.,1280.,1920.] {
            for left in [196.,280.,420.,2000.] {
                for right in [220.,340.,600.,2000.] {
                    for openness in [[1.,1.],[0.,1.],[1.,0.],[0.,0.]] {
                        let l=Layout::with_widths(window,openness,left,right);
                        assert!(l.left_width>=196. && l.right_width>=220.);
                        assert!(l.center_width>=l.left_width-0.001);
                        assert!(l.center_width>=l.right_width-0.001);
                        if openness[0]==1. {assert_eq!(l.left_x+l.left_width+RESIZE_WIDTH,l.center_x);}
                        if openness[1]==1. {assert!((l.center_x+l.center_width+RESIZE_WIDTH-l.right_x).abs()<0.001);}
                    }
                }
            }
        }
    }
    #[test]
    fn drag_crosses_minimum_without_compressing_content_and_can_reverse() {
        for (request, width, offset, hide) in [
            (340.,340.,0.,false), (220.,220.,0.,false),
            (219.,220.,1.,true), (190.,220.,30.,true),
            (0.,220.,220.,true), (-500.,220.,220.,true),
            (230.,230.,0.,false),
        ] {
            assert_eq!(drag_geometry(1280.,1.,request), DragGeometry {width,offset,hide});
        }
    }
    #[test]
    fn release_motion_starts_at_drag_position_and_finishes_offscreen() {
        let now = Instant::now();
        for offset in [1.,30.,150.,220.] {
            let motion = Motion::new([1.,1.],[1.,0.],now).with_right_offset(offset);
            let start = Layout::new(1280., [1.,1.], MIN_RIGHT_WIDTH);
            let mut last = start.right_x + offset;
            for step in 0..=24 {
                let time = now + DURATION * step / 24;
                let layout = Layout::new(1280., motion.sample(time).0, MIN_RIGHT_WIDTH);
                let x = layout.right_x + motion.right_offset(time);
                assert!(x >= last - 0.001);
                last = x;
                if step == 0 { assert_eq!(x, start.right_x + offset); }
                if step == 24 { assert!(x >= 1280.); assert_eq!(motion.right_offset(time),0.); }
            }
        }
    }
    #[test]
    fn all_states_keep_document_inside_window_and_reclaim_space() {
        for width in [1100., 1280., 1920.] {
            for left in [0., 0.25, 0.5, 1.] {
                for right in [0., 0.25, 0.5, 1.] {
                    let l = Layout::new(width, [left, right], DEFAULT_RIGHT_WIDTH);
                    assert!(l.center_x >= 14. && l.center_width >= 512.);
                    assert!((l.center_x + l.center_width - (width - 14. - 352. * right)).abs() < 0.001);
                    assert!(l.center_width > 2. * (HANDLE_WIDTH + 4.));
                }
            }
            let open = Layout::new(width, [1., 1.], DEFAULT_RIGHT_WIDTH);
            assert_eq!(open.center_x, 222.);
            assert_eq!(open.center_width, width - 588.);
            let closed = Layout::new(width, [0., 0.], DEFAULT_RIGHT_WIDTH);
            assert!(closed.left_x + 196. <= 0.);
            assert!(closed.right_x >= width);
            assert_eq!(closed.center_width - open.center_width, 560.);
        }
    }
    #[test]
    fn resized_panels_preserve_gutter_and_equal_width_limit() {
        for window in [1100., 1280., 1920., 2560.] {
            for left in [0., 0.5, 1.] {
                for requested in [0., 220., 280., 340., 460., 600., 2000.] {
                    let open = Layout::new(window, [left, 1.], requested);
                    assert!(open.right_width >= MIN_RIGHT_WIDTH);
                    assert!(open.center_width >= open.right_width);
                    if requested == 2000. {
                        assert_eq!(open.center_width, open.right_width);
                    }
                    assert!((open.right_x - open.center_x - open.center_width - RESIZE_WIDTH).abs() < 0.001);
                    assert_eq!(open.right_x + open.right_width, window - 14.);
                    let closed = Layout::new(window, [left, 0.], requested);
                    assert!(closed.right_x >= window);
                    assert_eq!(closed.center_x + closed.center_width, window - 14.);
                }
            }
        }
        assert_eq!(right_width(1280., 1., 0.), 220.);
        assert_eq!(right_width(1280., 1., 280.), 280.);
        assert_eq!(right_width(1280., 1., 600.), 516.);
        assert_eq!(right_width(1100., 1., 552.), 426.);
        assert_eq!(right_width(1920., 1., 552.), 552.);
        assert_eq!(right_width(1920., 1., 2000.), 836.);
        assert_eq!(right_width(1920., 0., 2000.), 940.);
    }
    #[test]
    fn reversal_and_other_side_toggle_continue_from_visible_position() {
        let start = Instant::now();
        let motion = Motion::new([1., 1.], [0., 1.], start);
        let mid = start + DURATION / 2;
        let (current, done) = motion.sample(mid);
        assert!(!done);
        assert!(current[0] > 0. && current[0] < 1.);
        assert_eq!(current[1], 1.);
        for target in [[1., 1.], [0., 0.]] {
            let reverse = Motion::new(current, target, mid);
            assert_eq!(reverse.sample(mid), (current, false));
            assert_eq!(reverse.sample(mid + DURATION), (target, true));
        }
    }
}
