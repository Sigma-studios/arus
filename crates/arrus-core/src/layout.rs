//! Stage layout shared by the web editor and the PDF export: the grid, the
//! backstage taken from it, and the dancer shapes.

use crate::{BackstageSize, Choreography, DancerShape};

/// An axis-aligned area in grid cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Area {
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
}

/// The whole grid: `x` in `0..=rows`, `y` in `0..=columns`. As in the
/// ArrangeUs app, the backstage is part of it (see [`stage_area`]), and
/// dancers may stand anywhere in it.
pub fn grid_area(c: &Choreography) -> Area {
    Area { x0: 0.0, y0: 0.0, x1: c.grid_size.width().max(1.0), y1: c.grid_size.depth().max(1.0) }
}

/// The performing area. With the backstage on, it is the grid minus its outer
/// cells: `sides` on both x ends, `back` from y = 0, `front` (audience side)
/// up to y = columns. Otherwise the whole grid.
pub fn stage_area(c: &Choreography) -> Area {
    let g = grid_area(c);
    if !backstage_on(c) {
        return g;
    }
    let b = &c.grid_size.backstage;
    let sides = b.sides.max(0) as f64;
    let (back, front) = (b.back.max(0) as f64, b.front.max(0) as f64);
    let s = Area { x0: g.x0 + sides, y0: g.y0 + back, x1: g.x1 - sides, y1: g.y1 - front };
    // A backstage that eats the whole grid leaves no stage: ignore it.
    if s.width() < 1.0 || s.height() < 1.0 { g } else { s }
}

/// Whether the choreography sets up a backstage beyond the app's default
/// (`sides: 2, back: 0, front: 0`). The ArrangeUs app shows such a backstage
/// even when `isBackstageShown` is false in the file.
pub fn backstage_configured(c: &Choreography) -> bool {
    let b = &c.grid_size.backstage;
    let d = BackstageSize::default();
    b.back > d.back || b.front > d.front || b.sides > d.sides
}

/// Whether the backstage is on: toggled on, or set up in the file (as the
/// ArrangeUs app shows it).
pub fn backstage_on(c: &Choreography) -> bool {
    c.settings.is_backstage_shown || backstage_configured(c)
}

/// Outline of a dancer shape in "unit" coordinates: it fits the circle of
/// radius 1 around the origin, with `-y` pointing up on screen / paper.
/// `None` = a circle.
pub fn shape_outline(shape: DancerShape) -> Option<&'static [(f64, f64)]> {
    use DancerShape::*;
    // Squares: half side 0.82, so they look as heavy as the circle.
    Some(match shape {
        Circle => return None,
        Square => &[(-0.82, -0.82), (0.82, -0.82), (0.82, 0.82), (-0.82, 0.82)],
        RotatedSquare => &[(0.0, -1.05), (1.05, 0.0), (0.0, 1.05), (-1.05, 0.0)],
        Triangle => &[(0.0, -0.95), (0.95, 0.75), (-0.95, 0.75)],
        UpsideDownTriangle => &[(-0.95, -0.75), (0.95, -0.75), (0.0, 0.95)],
        LeftTriangle => &[(-0.95, 0.0), (0.75, -0.95), (0.75, 0.95)],
        RightTriangle => &[(0.95, 0.0), (-0.75, 0.95), (-0.75, -0.95)],
        // Right-angled triangles named after the corner holding the right angle.
        TopLeftTriangle => &[(-0.82, -0.82), (0.82, -0.82), (-0.82, 0.82)],
        TopRightTriangle => &[(-0.82, -0.82), (0.82, -0.82), (0.82, 0.82)],
        BottomLeftTriangle => &[(-0.82, -0.82), (0.82, 0.82), (-0.82, 0.82)],
        BottomRightTriangle => &[(0.82, -0.82), (0.82, 0.82), (-0.82, 0.82)],
    })
}
