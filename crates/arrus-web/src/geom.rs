//! Stage geometry in grid cells: the drawable area, snapping, dancer shapes and
//! prop outlines / hit-testing / handles. No egui painting, no web APIs.

use arrus_core::{Choreography, DancerShape, Position, Prop, PropType};

/// A point or vector in stage cells (f64, like the model).
pub type P = Position;

pub fn add(a: P, b: P) -> P {
    P::new(a.x + b.x, a.y + b.y)
}
pub fn sub(a: P, b: P) -> P {
    P::new(a.x - b.x, a.y - b.y)
}
pub fn scale(a: P, s: f64) -> P {
    P::new(a.x * s, a.y * s)
}
pub fn len(a: P) -> f64 {
    a.x.hypot(a.y)
}

/// Axis-aligned box in stage cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: P,
    pub max: P,
}

impl Bounds {
    pub fn new(min: P, max: P) -> Self {
        Self { min, max }
    }
    pub fn contains(&self, p: P) -> bool {
        p.x >= self.min.x - 1e-9 && p.x <= self.max.x + 1e-9 && p.y >= self.min.y - 1e-9 && p.y <= self.max.y + 1e-9
    }
    pub fn clamp(&self, p: P) -> P {
        P::new(p.x.clamp(self.min.x, self.max.x), p.y.clamp(self.min.y, self.max.y))
    }
    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }
    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }
    pub fn center(&self) -> P {
        P::new((self.min.x + self.max.x) / 2.0, (self.min.y + self.max.y) / 2.0)
    }
}

/// The stage proper: `x` in `0..=rows`, `y` in `0..=columns`.
pub fn stage_bounds(c: &Choreography) -> Bounds {
    Bounds::new(P::new(0.0, 0.0), P::new(c.grid_size.width().max(1.0), c.grid_size.depth().max(1.0)))
}

/// Stage plus backstage: `sides` beyond both x ends, `back` before y = 0,
/// `front` (audience side) beyond y = columns.
pub fn drawable_bounds(c: &Choreography) -> Bounds {
    let s = stage_bounds(c);
    let b = &c.grid_size.backstage;
    let sides = b.sides.max(0) as f64;
    Bounds::new(
        P::new(-sides, -(b.back.max(0) as f64)),
        P::new(s.max.x + sides, s.max.y + b.front.max(0) as f64),
    )
}

/// Whether any dancer of any formation stands outside the stage proper.
pub fn anyone_backstage(c: &Choreography) -> bool {
    let s = stage_bounds(c);
    c.formations.iter().flat_map(|f| &f.units).any(|u| !s.contains(u.position))
}

/// Round to the nearest multiple of `step`.
pub fn snap(v: f64, step: f64) -> f64 {
    if step <= 0.0 { v } else { (v / step).round() * step }
}

pub fn snap_pos(p: P, step: f64) -> P {
    P::new(snap(p.x, step), snap(p.y, step))
}

/// Grid snapping step used by the editor.
pub const SNAP_STEP: f64 = 0.5;
/// Rotation snapping step (degrees) when grid snapping is on.
pub const ROTATION_SNAP_DEG: f64 = 15.0;

/// Outline of a dancer shape, in "unit" coordinates: the shape fits the circle
/// of radius 1 around the origin, `-y` points up in the default view (towards
/// the backstage). `None` = a circle.
pub fn dancer_shape_outline(shape: DancerShape) -> Option<Vec<P>> {
    use DancerShape::*;
    let s = 0.82; // half side of the squares, so they look as heavy as the circle
    let t = 0.95;
    let pts: &[(f64, f64)] = match shape {
        Circle => return None,
        Square => &[(-s, -s), (s, -s), (s, s), (-s, s)],
        RotatedSquare => &[(0.0, -1.05), (1.05, 0.0), (0.0, 1.05), (-1.05, 0.0)],
        Triangle => &[(0.0, -t), (t, 0.75), (-t, 0.75)],
        UpsideDownTriangle => &[(-t, -0.75), (t, -0.75), (0.0, t)],
        LeftTriangle => &[(-t, 0.0), (0.75, -t), (0.75, t)],
        RightTriangle => &[(t, 0.0), (-0.75, t), (-0.75, -t)],
        // Right-angled triangles named after the corner holding the right angle.
        TopLeftTriangle => &[(-s, -s), (s, -s), (-s, s)],
        TopRightTriangle => &[(-s, -s), (s, -s), (s, s)],
        BottomLeftTriangle => &[(-s, -s), (s, s), (-s, s)],
        BottomRightTriangle => &[(s, -s), (s, s), (-s, s)],
    };
    Some(pts.iter().map(|&(x, y)| P::new(x, y)).collect())
}

/// Human name of a dancer shape.
pub fn shape_label(shape: DancerShape) -> &'static str {
    use DancerShape::*;
    match shape {
        Circle => "Circle",
        Square => "Square",
        RotatedSquare => "Diamond",
        Triangle => "Triangle",
        UpsideDownTriangle => "Upside-down triangle",
        LeftTriangle => "Left triangle",
        RightTriangle => "Right triangle",
        TopLeftTriangle => "Top-left corner",
        TopRightTriangle => "Top-right corner",
        BottomLeftTriangle => "Bottom-left corner",
        BottomRightTriangle => "Bottom-right corner",
    }
}

pub fn prop_label(t: PropType) -> &'static str {
    match t {
        PropType::Rectangle => "Rectangle",
        PropType::Ellipse => "Ellipse",
        PropType::Triangle => "Triangle",
    }
}

/// Smallest prop extent, in cells.
pub const MIN_PROP_SIZE: f64 = 0.25;

/// A prop's placement: centred on `position`, `width` along x, `length` along
/// y, rotated by `rotation_angle` radians (positive = from +x towards +y).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropFrame {
    pub center: P,
    pub half: P,
    pub angle: f64,
}

impl PropFrame {
    pub fn of(p: &Prop) -> Self {
        Self {
            center: p.position,
            half: P::new(p.width.abs().max(0.01) / 2.0, p.length.abs().max(0.01) / 2.0),
            angle: p.rotation_angle,
        }
    }

    /// Rotate a local vector into stage orientation.
    pub fn rotate(&self, v: P) -> P {
        let (s, c) = self.angle.sin_cos();
        P::new(v.x * c - v.y * s, v.x * s + v.y * c)
    }

    pub fn unrotate(&self, v: P) -> P {
        let (s, c) = self.angle.sin_cos();
        P::new(v.x * c + v.y * s, -v.x * s + v.y * c)
    }

    pub fn to_world(self, local: P) -> P {
        add(self.center, self.rotate(local))
    }

    pub fn to_local(self, world: P) -> P {
        self.unrotate(sub(world, self.center))
    }

    /// Corners of the bounding box, in local sign order
    /// `(-,-) (+,-) (+,+) (-,+)` (world coordinates).
    pub fn corners(&self) -> [P; 4] {
        CORNER_SIGNS.map(|(sx, sy)| self.to_world(P::new(sx * self.half.x, sy * self.half.y)))
    }

    /// Where the rotation handle sits: beyond the "front" edge (local -y),
    /// `offset` cells away from it.
    pub fn rotation_handle(&self, offset: f64) -> P {
        self.to_world(P::new(0.0, -self.half.y - offset))
    }
}

/// Local signs of the four corners, matching [`PropFrame::corners`].
pub const CORNER_SIGNS: [(f64, f64); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];

/// Outline of a prop in world coordinates (convex polygon).
pub fn prop_outline(p: &Prop) -> Vec<P> {
    let f = PropFrame::of(p);
    let (hx, hy) = (f.half.x, f.half.y);
    let local: Vec<P> = match p.kind() {
        PropType::Rectangle => vec![P::new(-hx, -hy), P::new(hx, -hy), P::new(hx, hy), P::new(-hx, hy)],
        PropType::Triangle => vec![P::new(0.0, -hy), P::new(hx, hy), P::new(-hx, hy)],
        PropType::Ellipse => (0..48)
            .map(|i| {
                let a = i as f64 / 48.0 * std::f64::consts::TAU;
                P::new(hx * a.cos(), hy * a.sin())
            })
            .collect(),
    };
    local.into_iter().map(|v| f.to_world(v)).collect()
}

/// Whether `world` lies inside the prop (with `tol` cells of slack).
pub fn prop_contains(p: &Prop, world: P, tol: f64) -> bool {
    let f = PropFrame::of(p);
    let l = f.to_local(world);
    let (hx, hy) = (f.half.x + tol, f.half.y + tol);
    match p.kind() {
        PropType::Rectangle => l.x.abs() <= hx && l.y.abs() <= hy,
        PropType::Ellipse => (l.x / hx).powi(2) + (l.y / hy).powi(2) <= 1.0,
        PropType::Triangle => {
            // Apex (0,-hy), base corners (±hx, hy).
            if l.y < -hy || l.y > hy {
                return false;
            }
            let frac = (l.y + hy) / (2.0 * hy); // 0 at apex, 1 at base
            l.x.abs() <= hx * frac + tol
        }
    }
}

/// Result of dragging one corner of a prop while the opposite corner stays put.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resized {
    pub center: P,
    pub width: f64,
    pub length: f64,
}

/// Resize from corner `signs` (one of [`CORNER_SIGNS`]); `fixed` is the world
/// position of the opposite corner, `angle` the prop's rotation.
pub fn resize_from_corner(signs: (f64, f64), fixed: P, pointer: P, angle: f64, snap_step: Option<f64>) -> Resized {
    let frame = PropFrame { center: fixed, half: P::new(0.0, 0.0), angle };
    let d = frame.unrotate(sub(pointer, fixed));
    let mut w = (signs.0 * d.x).max(MIN_PROP_SIZE);
    let mut l = (signs.1 * d.y).max(MIN_PROP_SIZE);
    if let Some(step) = snap_step {
        w = snap(w, step).max(step);
        l = snap(l, step).max(step);
    }
    let center = add(fixed, frame.rotate(P::new(signs.0 * w / 2.0, signs.1 * l / 2.0)));
    Resized { center, width: w, length: l }
}

/// Rotation that points the prop's rotation handle (local -y) at `pointer`.
pub fn rotation_towards(center: P, pointer: P, snap_deg: Option<f64>) -> f64 {
    let d = sub(pointer, center);
    // local (0,-1) rotated by a = (sin a, -cos a)
    let mut a = d.x.atan2(-d.y);
    if let Some(step) = snap_deg {
        a = snap(a.to_degrees(), step).to_radians();
    }
    normalize_angle(a)
}

/// Wrap radians into (-pi, pi].
pub fn normalize_angle(a: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let mut a = a % TAU;
    if a <= -PI {
        a += TAU;
    } else if a > PI {
        a -= TAU;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    fn prop(kind: PropType, w: f64, l: f64, angle: f64) -> Prop {
        Prop {
            prop_type: kind.as_str().into(),
            width: w,
            length: l,
            height: 1.0,
            position: P::new(5.0, 3.0),
            rotation_angle: angle,
            color: 0,
            alpha: 1.0,
            extra: Default::default(),
        }
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(3.26, 0.5), 3.5);
        assert_eq!(snap(3.24, 0.5), 3.0);
        assert_eq!(snap(-0.74, 0.5), -0.5);
        assert_eq!(snap(1.3, 0.0), 1.3);
        assert_eq!(snap_pos(P::new(1.1, 2.8), 1.0), P::new(1.0, 3.0));
    }

    #[test]
    fn bounds_include_backstage() {
        let mut c = Choreography::new("t", "id", 0);
        c.grid_size.backstage.front = 1;
        c.grid_size.backstage.back = 3;
        let d = drawable_bounds(&c);
        assert_eq!(d.min, P::new(-2.0, -3.0));
        assert_eq!(d.max, P::new(16.0, 9.0));
        assert!(!anyone_backstage(&c));
        c.add_dancer(arrus_core::Unit::at(-1.0, 2.0));
        assert!(anyone_backstage(&c));
        assert_eq!(d.clamp(P::new(-9.0, 20.0)), P::new(-2.0, 9.0));
    }

    #[test]
    fn every_shape_fits_the_unit_circle() {
        for &s in DancerShape::ALL {
            if let Some(pts) = dancer_shape_outline(s) {
                assert!(pts.len() >= 3, "{s:?}");
                assert!(pts.iter().all(|p| len(*p) <= 1.5), "{s:?}");
            }
        }
        assert!(dancer_shape_outline(DancerShape::Circle).is_none());
    }

    #[test]
    fn prop_frame_roundtrip_and_corners() {
        let p = prop(PropType::Rectangle, 4.0, 2.0, 0.3);
        let f = PropFrame::of(&p);
        let w = P::new(6.2, 1.7);
        assert!(close(f.to_world(f.to_local(w)), w));
        let unrot = PropFrame::of(&prop(PropType::Rectangle, 4.0, 2.0, 0.0));
        assert_eq!(unrot.corners()[0], P::new(3.0, 2.0));
        assert_eq!(unrot.corners()[2], P::new(7.0, 4.0));
        assert_eq!(unrot.rotation_handle(1.0), P::new(5.0, 1.0));
    }

    #[test]
    fn prop_hit_testing() {
        let r = prop(PropType::Rectangle, 4.0, 2.0, 0.0);
        assert!(prop_contains(&r, P::new(6.9, 3.9), 0.0));
        assert!(!prop_contains(&r, P::new(7.1, 3.0), 0.0));
        assert!(prop_contains(&r, P::new(7.1, 3.0), 0.2));
        // Rotated 90 degrees: now 2 wide (x) and 4 long (y).
        let r90 = prop(PropType::Rectangle, 4.0, 2.0, std::f64::consts::FRAC_PI_2);
        assert!(prop_contains(&r90, P::new(5.0, 4.9), 0.0));
        assert!(!prop_contains(&r90, P::new(6.5, 3.0), 0.0));
        let e = prop(PropType::Ellipse, 4.0, 2.0, 0.0);
        assert!(prop_contains(&e, P::new(6.9, 3.0), 0.0));
        assert!(!prop_contains(&e, P::new(6.9, 3.9), 0.0));
        let t = prop(PropType::Triangle, 4.0, 2.0, 0.0);
        assert!(prop_contains(&t, P::new(5.0, 2.1), 0.0)); // near apex
        assert!(!prop_contains(&t, P::new(6.5, 2.2), 0.0)); // outside a slanted side
        assert!(prop_contains(&t, P::new(6.9, 3.95), 0.0)); // near a base corner
    }

    #[test]
    fn outlines_are_in_world_space() {
        let r = prop(PropType::Rectangle, 4.0, 2.0, 0.0);
        let o = prop_outline(&r);
        assert_eq!(o.len(), 4);
        assert!(o.iter().all(|p| prop_contains(&r, *p, 1e-6)));
        assert_eq!(prop_outline(&prop(PropType::Ellipse, 1.0, 1.0, 0.0)).len(), 48);
    }

    #[test]
    fn resize_keeps_opposite_corner() {
        // Unrotated 4x2 at (5,3): drag the (+,+) corner from (7,4) to (8,5).
        let fixed = P::new(3.0, 2.0);
        let r = resize_from_corner((1.0, 1.0), fixed, P::new(8.0, 5.0), 0.0, None);
        assert_eq!((r.width, r.length), (5.0, 3.0));
        assert!(close(r.center, P::new(5.5, 3.5)));
        // Dragging past the fixed corner clamps to the minimum size.
        let r = resize_from_corner((1.0, 1.0), fixed, P::new(0.0, 0.0), 0.0, None);
        assert_eq!((r.width, r.length), (MIN_PROP_SIZE, MIN_PROP_SIZE));
        // Snapped.
        let r = resize_from_corner((1.0, 1.0), fixed, P::new(7.3, 4.6), 0.0, Some(0.5));
        assert_eq!((r.width, r.length), (4.5, 2.5));
        // Rotated: the fixed corner really stays put.
        let p = prop(PropType::Rectangle, 4.0, 2.0, 0.7);
        let f = PropFrame::of(&p);
        let cs = f.corners();
        let (c0, c2) = (cs[0], cs[2]);
        let r = resize_from_corner((1.0, 1.0), c0, add(c2, P::new(0.5, 0.2)), 0.7, None);
        let nf = PropFrame { center: r.center, half: P::new(r.width / 2.0, r.length / 2.0), angle: 0.7 };
        assert!(close(nf.corners()[0], c0));
    }

    #[test]
    fn rotation_handle_follows_pointer() {
        let c = P::new(0.0, 0.0);
        assert!((rotation_towards(c, P::new(0.0, -1.0), None)).abs() < 1e-12);
        assert!((rotation_towards(c, P::new(1.0, 0.0), None) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let a = rotation_towards(c, P::new(1.0, -0.95), Some(15.0));
        assert!((a.to_degrees() - 45.0).abs() < 1e-9);
        // The handle of a prop rotated that way points at the pointer.
        let mut p = prop(PropType::Rectangle, 2.0, 2.0, 0.0);
        p.position = c;
        p.rotation_angle = rotation_towards(c, P::new(3.0, 3.0), None);
        let h = PropFrame::of(&p).rotation_handle(0.0);
        assert!((h.x - h.y).abs() < 1e-9 && h.x > 0.0);
    }

    #[test]
    fn angles_wrap() {
        use std::f64::consts::PI;
        assert!((normalize_angle(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12);
        assert!((normalize_angle(-PI) - PI).abs() < 1e-12);
        assert_eq!(normalize_angle(0.5), 0.5);
    }
}
