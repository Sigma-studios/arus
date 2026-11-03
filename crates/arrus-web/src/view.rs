//! Stage <-> screen transform: zoom, pan, fit, and the 180° "view from
//! backstage" rotation. Screen coordinates are egui points.

use egui::{Pos2, Rect, Vec2, vec2};

use crate::geom::{Bounds, P};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Stage point shown at `screen_center`.
    pub center: P,
    /// Screen points per cell.
    pub scale: f32,
    /// Rotated 180°: audience edge at the top, x to the left.
    pub inverted: bool,
    pub screen_center: Pos2,
}

pub const MIN_SCALE: f32 = 4.0;
pub const MAX_SCALE: f32 = 400.0;

impl View {
    /// Fit `bounds` (stage cells) into `rect` (screen), leaving `margin` points.
    pub fn fit(rect: Rect, bounds: Bounds, margin: f32, inverted: bool) -> Self {
        let avail = (rect.size() - Vec2::splat(2.0 * margin)).max(Vec2::splat(10.0));
        let sx = avail.x / bounds.width().max(0.1) as f32;
        let sy = avail.y / bounds.height().max(0.1) as f32;
        Self {
            center: bounds.center(),
            scale: sx.min(sy).clamp(MIN_SCALE, MAX_SCALE),
            inverted,
            screen_center: rect.center(),
        }
    }

    fn sign(&self) -> f32 {
        if self.inverted { -1.0 } else { 1.0 }
    }

    pub fn to_screen(self, p: P) -> Pos2 {
        let d = vec2((p.x - self.center.x) as f32, (p.y - self.center.y) as f32) * self.scale * self.sign();
        self.screen_center + d
    }

    pub fn stage_at(self, s: Pos2) -> P {
        let d = (s - self.screen_center) / (self.scale * self.sign());
        P::new(self.center.x + d.x as f64, self.center.y + d.y as f64)
    }

    /// A stage-space direction/offset (cells) on screen.
    pub fn vec_to_screen(&self, v: P) -> Vec2 {
        vec2(v.x as f32, v.y as f32) * self.scale * self.sign()
    }

    /// A screen-space offset in stage cells.
    pub fn vec_from_screen(&self, v: Vec2) -> P {
        let d = v / (self.scale * self.sign());
        P::new(d.x as f64, d.y as f64)
    }

    /// Stage rectangle on screen.
    pub fn bounds_to_screen(&self, b: Bounds) -> Rect {
        Rect::from_two_pos(self.to_screen(b.min), self.to_screen(b.max))
    }

    /// Zoom by `factor`, keeping the stage point under `anchor` (screen) fixed.
    pub fn zoom_at(&mut self, anchor: Pos2, factor: f32) {
        let before = self.stage_at(anchor);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let after = self.stage_at(anchor);
        self.center = P::new(self.center.x + before.x - after.x, self.center.y + before.y - after.y);
    }

    /// Move the content by a screen offset.
    pub fn pan(&mut self, screen_delta: Vec2) {
        let d = self.vec_from_screen(screen_delta);
        self.center = P::new(self.center.x - d.x, self.center.y - d.y);
    }

    /// Keep the same stage point centred when the widget moves or resizes.
    pub fn set_screen_center(&mut self, c: Pos2) {
        self.screen_center = c;
    }

    /// Flip the orientation, keeping the same stage point in the centre.
    pub fn set_inverted(&mut self, inverted: bool) {
        self.inverted = inverted;
    }
}

/// Map a time to an x coordinate in a timeline `rect` covering `0..=total`.
pub fn time_to_x(t: f64, total: f64, rect: Rect) -> f32 {
    if total <= 0.0 {
        return rect.left();
    }
    rect.left() + (t / total).clamp(0.0, 1.0) as f32 * rect.width()
}

pub fn x_to_time(x: f32, total: f64, rect: Rect) -> f64 {
    if rect.width() <= 0.0 {
        return 0.0;
    }
    (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64) * total
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn close(a: P, b: P) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4
    }

    fn stage() -> Bounds {
        Bounds::new(P::new(0.0, 0.0), P::new(14.0, 8.0))
    }

    #[test]
    fn default_orientation_puts_audience_at_bottom() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 800.0));
        let v = View::fit(rect, stage(), 0.0, false);
        assert_eq!(v.scale, 100.0);
        assert_eq!(v.to_screen(P::new(0.0, 0.0)), pos2(0.0, 0.0));
        assert_eq!(v.to_screen(P::new(14.0, 8.0)), pos2(1400.0, 800.0));
        // y = 0 (backstage) is above y = 8 (audience).
        assert!(v.to_screen(P::new(7.0, 0.0)).y < v.to_screen(P::new(7.0, 8.0)).y);
    }

    #[test]
    fn inverted_rotates_180() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 800.0));
        let v = View::fit(rect, stage(), 0.0, true);
        assert_eq!(v.to_screen(P::new(0.0, 0.0)), pos2(1400.0, 800.0));
        assert_eq!(v.to_screen(P::new(14.0, 8.0)), pos2(0.0, 0.0));
        let p = P::new(3.5, 2.0);
        assert!(close(v.stage_at(v.to_screen(p)), p));
        assert_eq!(v.vec_to_screen(P::new(1.0, 0.0)), vec2(-100.0, 0.0));
        assert!(close(v.vec_from_screen(vec2(-100.0, 0.0)), P::new(1.0, 0.0)));
    }

    #[test]
    fn fit_uses_the_tighter_axis_and_margin() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 1000.0));
        let v = View::fit(rect, stage(), 50.0, false);
        assert!((v.scale - 900.0 / 14.0).abs() < 1e-4);
        assert_eq!(v.to_screen(stage().center()), pos2(500.0, 500.0));
    }

    #[test]
    fn zoom_keeps_anchor_and_pan_moves_content() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 800.0));
        for inverted in [false, true] {
            let mut v = View::fit(rect, stage(), 0.0, inverted);
            let anchor = pos2(300.0, 200.0);
            let under = v.stage_at(anchor);
            v.zoom_at(anchor, 2.0);
            assert_eq!(v.scale, 200.0);
            assert!(close(v.stage_at(anchor), under));
            let before = v.to_screen(P::new(1.0, 1.0));
            v.pan(vec2(10.0, -5.0));
            let after = v.to_screen(P::new(1.0, 1.0));
            assert!((after - before - vec2(10.0, -5.0)).length() < 1e-3);
        }
        let mut v = View::fit(rect, stage(), 0.0, false);
        v.zoom_at(pos2(0.0, 0.0), 1e6);
        assert_eq!(v.scale, MAX_SCALE);
    }

    #[test]
    fn timeline_mapping() {
        let r = Rect::from_min_size(pos2(100.0, 0.0), vec2(200.0, 10.0));
        assert_eq!(time_to_x(5.0, 10.0, r), 200.0);
        assert_eq!(time_to_x(50.0, 10.0, r), 300.0);
        assert_eq!(x_to_time(150.0, 10.0, r), 2.5);
        assert_eq!(x_to_time(0.0, 10.0, r), 0.0);
        assert_eq!(time_to_x(1.0, 0.0, r), 100.0);
    }
}
