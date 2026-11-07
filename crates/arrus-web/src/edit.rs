//! Editing operations on a [`Choreography`], selection and hit-testing.
//! Pure logic: no egui painting, no web APIs.

use std::collections::BTreeSet;

use arrus_core::timeline::{self, Easing, Phase};
use arrus_core::{Choreography, DEFAULT_COLOR, Formation, Prop, PropType, Unit};

use crate::geom::{self, Bounds, P};

/// What is selected on the stage. Dancers are indices (valid in every
/// formation); a prop is an index into the current formation's props.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Selection {
    #[default]
    None,
    Dancers(BTreeSet<usize>),
    Prop(usize),
}

impl Selection {
    pub fn dancers(&self) -> Option<&BTreeSet<usize>> {
        match self {
            Selection::Dancers(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    pub fn has_dancer(&self, i: usize) -> bool {
        matches!(self, Selection::Dancers(s) if s.contains(&i))
    }

    pub fn one(i: usize) -> Self {
        Selection::Dancers(BTreeSet::from([i]))
    }

    /// Shift-click: add or remove one dancer.
    pub fn toggle(&mut self, i: usize) {
        match self {
            Selection::Dancers(s) => {
                if !s.remove(&i) {
                    s.insert(i);
                }
                if s.is_empty() {
                    *self = Selection::None;
                }
            }
            _ => *self = Selection::one(i),
        }
    }

    /// Drop indices that no longer exist.
    pub fn sanitize(&mut self, dancers: usize, props: usize) {
        match self {
            Selection::Dancers(s) => {
                s.retain(|&i| i < dancers);
                if s.is_empty() {
                    *self = Selection::None;
                }
            }
            Selection::Prop(i) if *i >= props => *self = Selection::None,
            _ => {}
        }
    }
}

/// Index of the topmost dancer whose centre is within `radius` of `p`
/// (closest wins; later dancers are drawn on top).
pub fn hit_dancer(positions: &[(usize, P)], p: P, radius: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for &(i, q) in positions.iter().rev() {
        let d = geom::len(geom::sub(p, q));
        if d <= radius && best.is_none_or(|(_, bd)| d < bd - 1e-9) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// Topmost prop containing `p`.
pub fn hit_prop(props: &[Prop], p: P, tol: f64) -> Option<usize> {
    props.iter().enumerate().rev().find(|(_, pr)| geom::prop_contains(pr, p, tol)).map(|(i, _)| i)
}

/// Dancers inside the box spanned by `a` and `b` (stage cells).
pub fn box_select(positions: &[(usize, P)], a: P, b: P) -> BTreeSet<usize> {
    let bx = Bounds::new(P::new(a.x.min(b.x), a.y.min(b.y)), P::new(a.x.max(b.x), a.y.max(b.y)));
    positions.iter().filter(|(_, q)| bx.contains(*q)).map(|(i, _)| *i).collect()
}

/// Positions of every dancer in formation `f`.
pub fn formation_positions(c: &Choreography, f: usize) -> Vec<(usize, P)> {
    c.formations.get(f).map(|f| f.units.iter().enumerate().map(|(i, u)| (i, u.position)).collect()).unwrap_or_default()
}

/// Move a group of dancers rigidly: the `anchor` dancer follows the pointer
/// (snapped), the others keep their offsets to it; everyone stays inside
/// `bounds`. `starts` are positions at the start of the drag.
pub fn drag_dancers(
    c: &mut Choreography,
    formation: usize,
    starts: &[(usize, P)],
    anchor_start: P,
    delta: P,
    snap: Option<f64>,
    bounds: Bounds,
) {
    let mut target = geom::add(anchor_start, delta);
    if let Some(step) = snap {
        target = geom::snap_pos(target, step);
    }
    let mut d = geom::sub(target, anchor_start);
    // Limit the group movement so nobody leaves the drawable area.
    for &(_, s) in starts {
        let moved = geom::add(s, d);
        let clamped = bounds.clamp(moved);
        d = geom::add(d, geom::sub(clamped, moved));
    }
    let Some(f) = c.formations.get_mut(formation) else { return };
    for &(i, s) in starts {
        if let Some(u) = f.units.get_mut(i) {
            u.position = bounds.clamp(geom::add(s, d));
        }
    }
}

/// Move dancers by a fixed offset (arrow-key nudge), clamped to `bounds`.
pub fn nudge_dancers(c: &mut Choreography, formation: usize, which: &BTreeSet<usize>, d: P, bounds: Bounds) {
    if let Some(f) = c.formations.get_mut(formation) {
        for &i in which {
            if let Some(u) = f.units.get_mut(i) {
                u.position = bounds.clamp(geom::add(u.position, d));
            }
        }
    }
}

/// Apply `edit` to dancer `index` in every formation (name, colour, shape...).
pub fn edit_dancer_everywhere(c: &mut Choreography, index: usize, mut edit: impl FnMut(&mut Unit)) {
    for f in &mut c.formations {
        if let Some(u) = f.units.get_mut(index) {
            edit(u);
        }
    }
}

/// Remove several dancers from every formation.
pub fn remove_dancers(c: &mut Choreography, which: &BTreeSet<usize>) {
    for &i in which.iter().rev() {
        c.remove_dancer(i);
    }
}

/// Distinct colours for new dancers (ARGB).
pub const PALETTE: [i64; 10] = [
    DEFAULT_COLOR,
    0xFF3D8BFD,
    0xFF2EC27E,
    0xFFF5A524,
    0xFF9B5DE5,
    0xFF00BBF9,
    0xFFFF6B35,
    0xFFE0479E,
    0xFF7BC950,
    0xFFF15BB5,
];

/// The nearest free half-cell intersection to the front centre of the stage
/// in formation `f`, for a newly added dancer.
pub fn free_spot(c: &Choreography, f: usize) -> P {
    let stage = geom::stage_bounds(c);
    let taken = formation_positions(c, f);
    let start = P::new(geom::snap(stage.center().x, 1.0), stage.min.y + 1.0f64.min(stage.height()));
    let free = |p: P| taken.iter().all(|(_, q)| geom::len(geom::sub(p, *q)) >= 0.99);
    // Spiral-ish search over whole cells, nearest first.
    let mut candidates: Vec<P> = Vec::new();
    let (w, h) = (stage.width() as i64, stage.height() as i64);
    for y in 0..=h {
        for x in 0..=w {
            candidates.push(P::new(stage.min.x + x as f64, stage.min.y + y as f64));
        }
    }
    candidates.sort_by(|a, b| {
        let da = geom::len(geom::sub(*a, start));
        let db = geom::len(geom::sub(*b, start));
        da.total_cmp(&db)
    });
    candidates.into_iter().find(|p| free(*p)).unwrap_or(start)
}

/// A new dancer with the next palette colour.
pub fn new_dancer(c: &Choreography, at: P) -> Unit {
    let mut u = Unit::at(at.x, at.y);
    u.color = PALETTE[c.dancer_count() % PALETTE.len()];
    u
}

/// Add a dancer at a free spot of formation `f` (same spot in every formation).
pub fn add_dancer(c: &mut Choreography, f: usize) -> usize {
    let at = free_spot(c, f);
    let u = new_dancer(c, at);
    c.add_dancer(u)
}

/// Lay `n` dancers out in centred rows across the front half of the stage.
pub fn line_up(c: &Choreography, n: usize) -> Vec<P> {
    let stage = geom::stage_bounds(c);
    let per_row = ((stage.width() / 1.5).floor() as usize).max(1);
    let rows = n.div_ceil(per_row).max(1);
    let mut out = Vec::with_capacity(n);
    for r in 0..rows {
        let in_row = (n - r * per_row).min(per_row);
        let y = (stage.center().y - (rows as f64 - 1.0) * 0.75 + r as f64 * 1.5).max(stage.min.y);
        for k in 0..in_row {
            let x = stage.center().x + (k as f64 - (in_row as f64 - 1.0) / 2.0) * 1.5;
            out.push(geom::snap_pos(P::new(x, y), 0.5));
        }
    }
    out
}

/// A new choreography: the app's default stage and settings, one formation,
/// `dancers` dancers lined up in the middle with distinct colours.
pub fn new_choreography(name: &str, id: &str, now: i64, dancers: usize) -> Choreography {
    let mut c = Choreography::new(name, id, now);
    for p in line_up(&c, dancers) {
        let u = new_dancer(&c, p);
        c.add_dancer(u);
    }
    c
}

/// Insert a new formation after `index` with the same positions and props,
/// default timing, no name or comments. Returns the new index.
pub fn add_formation_after(c: &mut Choreography, index: usize) -> usize {
    let src = c.formations.get(index).cloned().unwrap_or_default();
    let mut f = Formation::new();
    f.units = src.units;
    f.props = src.props.or_else(|| Some(Vec::new()));
    f.color = src.color;
    let at = (index + 1).min(c.formations.len());
    c.formations.insert(at, f);
    at
}

/// Insert an exact copy of formation `index` right after it.
pub fn duplicate_formation(c: &mut Choreography, index: usize) -> Option<usize> {
    let f = c.formations.get(index)?.clone();
    c.formations.insert(index + 1, f);
    Some(index + 1)
}

/// Delete a formation, keeping at least one. Returns the index to show next.
pub fn delete_formation(c: &mut Choreography, index: usize) -> Option<usize> {
    if c.formations.len() <= 1 || index >= c.formations.len() {
        return None;
    }
    c.formations.remove(index);
    Some(index.min(c.formations.len() - 1))
}

/// Swap a formation with its neighbour. Returns its new index.
pub fn move_formation(c: &mut Choreography, index: usize, up: bool) -> Option<usize> {
    let other = if up { index.checked_sub(1)? } else { index + 1 };
    if index >= c.formations.len() || other >= c.formations.len() {
        return None;
    }
    c.formations.swap(index, other);
    Some(other)
}

/// A new prop of `kind` in the middle of the stage.
pub fn new_prop(c: &Choreography, kind: PropType) -> Prop {
    let s = geom::stage_bounds(c);
    let (w, l) = match kind {
        PropType::Rectangle => (2.0, 1.0),
        PropType::Ellipse => (1.5, 1.5),
        PropType::Triangle => (1.5, 1.5),
    };
    Prop {
        prop_type: kind.as_str().to_owned(),
        width: w,
        length: l,
        height: 1.0,
        position: geom::snap_pos(s.center(), 0.5),
        rotation_angle: 0.0,
        color: 0xFF8D99AE,
        alpha: 1.0,
        extra: Default::default(),
    }
}

/// Add a prop to formation `f`; returns its index.
pub fn add_prop(c: &mut Choreography, f: usize, kind: PropType) -> Option<usize> {
    let p = new_prop(c, kind);
    let props = c.formations.get_mut(f)?.props.get_or_insert_with(Vec::new);
    props.push(p);
    Some(props.len() - 1)
}

pub fn props(c: &Choreography, f: usize) -> &[Prop] {
    c.formations.get(f).and_then(|f| f.props.as_deref()).unwrap_or(&[])
}

pub fn prop_mut(c: &mut Choreography, f: usize, i: usize) -> Option<&mut Prop> {
    c.formations.get_mut(f)?.props.as_mut()?.get_mut(i)
}

pub fn remove_prop(c: &mut Choreography, f: usize, i: usize) -> bool {
    match c.formations.get_mut(f).and_then(|f| f.props.as_mut()) {
        Some(p) if i < p.len() => {
            p.remove(i);
            true
        }
        _ => false,
    }
}

/// Where the playhead is: the phase at `t`, unless `pinned` (a formation picked
/// from the list or with previous/next) has its hold starting at `t`. A 0 s
/// formation shares that instant with the next transition or formation, so time
/// alone could never select it.
pub fn phase(c: &Choreography, t: f64, pinned: Option<usize>, easing: Easing) -> Option<Phase> {
    match pinned {
        Some(p) if p < c.formations.len() && timeline::formation_time(c, p) == t => Some(Phase::Hold { formation: p }),
        _ => timeline::phase_at(c, t, easing),
    }
}

/// The formation edits apply to: the one held, or the one being entered during
/// a transition.
pub fn formation_at(c: &Choreography, t: f64, pinned: Option<usize>) -> usize {
    phase(c, t, pinned, Easing::Linear).map(Phase::current).unwrap_or(0)
}

/// If the playhead is inside a transition, the formation being entered.
pub fn entered_formation(c: &Choreography, t: f64, pinned: Option<usize>) -> Option<usize> {
    match phase(c, t, pinned, Easing::Linear) {
        Some(Phase::Transition { to, .. }) => Some(to),
        _ => None,
    }
}

/// Props shown in `phase`: those of the current formation, moving smoothly
/// from the previous formation's matching prop (same index and type).
pub fn props_in(c: &Choreography, phase: Option<Phase>) -> Vec<Prop> {
    match phase {
        None => Vec::new(),
        Some(Phase::Hold { formation }) => props(c, formation).to_vec(),
        Some(Phase::Transition { from, to, progress }) => {
            let a = props(c, from);
            props(c, to)
                .iter()
                .enumerate()
                .map(|(i, pb)| match a.get(i) {
                    Some(pa) if pa.prop_type == pb.prop_type => {
                        let mut p = pb.clone();
                        let k = progress;
                        p.position = pa.position.lerp(pb.position, k);
                        p.width = pa.width + (pb.width - pa.width) * k;
                        p.length = pa.length + (pb.length - pa.length) * k;
                        p.rotation_angle =
                            pa.rotation_angle + geom::normalize_angle(pb.rotation_angle - pa.rotation_angle) * k;
                        p.alpha = pa.alpha + (pb.alpha - pa.alpha) * k;
                        p
                    }
                    _ => pb.clone(),
                })
                .collect()
        }
    }
}

/// Label of formation `i` for lists: "3 · Chorus" or "3".
pub fn formation_label(c: &Choreography, i: usize) -> String {
    match c.formations.get(i).and_then(|f| f.name.as_deref()).map(str::trim) {
        Some(n) if !n.is_empty() => format!("{} · {n}", i + 1),
        _ => format!("{}", i + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Choreography {
        let mut c = new_choreography("t", "id", 0, 4);
        add_formation_after(&mut c, 0);
        c
    }

    #[test]
    fn new_choreography_lines_up_dancers() {
        let c = new_choreography("Show", "id", 5, 12);
        assert_eq!(c.dancer_count(), 12);
        assert_eq!(c.formations.len(), 1);
        let stage = geom::stage_bounds(&c);
        let ps = formation_positions(&c, 0);
        assert!(ps.iter().all(|(_, p)| stage.contains(*p)));
        // No two dancers on the same spot.
        for (i, a) in &ps {
            for (j, b) in &ps {
                assert!(i == j || geom::len(geom::sub(*a, *b)) >= 1.0);
            }
        }
        // Distinct colours for the first ones.
        assert_ne!(c.formations[0].units[0].color, c.formations[0].units[1].color);
        assert_eq!(new_choreography("e", "id", 0, 0).dancer_count(), 0);
    }

    #[test]
    fn selection_toggle_and_sanitize() {
        let mut s = Selection::None;
        s.toggle(2);
        assert!(s.has_dancer(2));
        s.toggle(3);
        s.toggle(2);
        assert_eq!(s, Selection::one(3));
        s.toggle(3);
        assert_eq!(s, Selection::None);
        let mut s = Selection::Dancers(BTreeSet::from([1, 5]));
        s.sanitize(3, 0);
        assert_eq!(s, Selection::one(1));
        let mut s = Selection::Prop(2);
        s.sanitize(3, 2);
        assert_eq!(s, Selection::None);
    }

    #[test]
    fn hit_and_box_select() {
        let ps = vec![(0, P::new(1.0, 1.0)), (1, P::new(1.2, 1.0)), (2, P::new(5.0, 5.0))];
        assert_eq!(hit_dancer(&ps, P::new(1.15, 1.0), 0.4), Some(1));
        assert_eq!(hit_dancer(&ps, P::new(0.9, 1.0), 0.4), Some(0));
        assert_eq!(hit_dancer(&ps, P::new(3.0, 3.0), 0.4), None);
        // Equal distance: the topmost (last) wins.
        assert_eq!(hit_dancer(&ps, P::new(1.1, 1.0), 0.4), Some(1));
        assert_eq!(box_select(&ps, P::new(6.0, 6.0), P::new(0.5, 0.5)), BTreeSet::from([0, 1, 2]));
        assert_eq!(box_select(&ps, P::new(0.0, 0.0), P::new(2.0, 2.0)), BTreeSet::from([0, 1]));
    }

    #[test]
    fn group_drag_snaps_anchor_and_respects_bounds() {
        let mut c = sample();
        c.formations[1].units[0].position = P::new(2.0, 2.0);
        c.formations[1].units[1].position = P::new(3.0, 2.0);
        let starts = vec![(0, P::new(2.0, 2.0)), (1, P::new(3.0, 2.0))];
        let b = geom::drawable_bounds(&c);
        drag_dancers(&mut c, 1, &starts, P::new(2.0, 2.0), P::new(1.3, 0.2), Some(0.5), b);
        assert_eq!(c.formations[1].units[0].position, P::new(3.5, 2.0));
        assert_eq!(c.formations[1].units[1].position, P::new(4.5, 2.0));
        // Formation 0 untouched.
        assert_ne!(c.formations[0].units[0].position, P::new(3.5, 2.0));
        // Dragging far right stops the group at the edge, keeping its shape.
        drag_dancers(&mut c, 1, &starts, P::new(2.0, 2.0), P::new(100.0, 0.0), None, b);
        assert_eq!(c.formations[1].units[1].position.x, b.max.x);
        assert_eq!(c.formations[1].units[0].position.x, b.max.x - 1.0);
    }

    #[test]
    fn nudge_and_attributes() {
        let mut c = sample();
        let before = c.formations[0].units[0].position;
        let b = geom::drawable_bounds(&c);
        nudge_dancers(&mut c, 0, &BTreeSet::from([0]), P::new(0.5, -1.0), b);
        assert_eq!(c.formations[0].units[0].position, P::new(before.x + 0.5, before.y - 1.0));
        edit_dancer_everywhere(&mut c, 2, |u| u.dancer.name = "Ana".into());
        assert!(c.formations.iter().all(|f| f.units[2].dancer.name == "Ana"));
    }

    #[test]
    fn add_and_remove_dancers() {
        let mut c = sample();
        let i = add_dancer(&mut c, 1);
        assert_eq!(i, 4);
        assert!(c.formations.iter().all(|f| f.units.len() == 5));
        let p = c.formations[1].units[4].position;
        assert!(formation_positions(&c, 1).iter().filter(|(_, q)| *q == p).count() == 1);
        remove_dancers(&mut c, &BTreeSet::from([0, 2]));
        assert!(c.formations.iter().all(|f| f.units.len() == 3));
    }

    #[test]
    fn free_spot_avoids_dancers() {
        let mut c = Choreography::new("t", "id", 0);
        let first = free_spot(&c, 0);
        assert_eq!(first, P::new(7.0, 1.0));
        c.add_dancer(Unit::at(7.0, 1.0));
        let second = free_spot(&c, 0);
        assert_ne!(second, first);
        assert!(geom::len(geom::sub(second, first)) <= 1.01);
    }

    #[test]
    fn formation_operations() {
        let mut c = sample();
        c.formations[0].name = Some("Intro".into());
        c.formations[0].duration = 4.0;
        let d = duplicate_formation(&mut c, 0).unwrap();
        assert_eq!(d, 1);
        assert_eq!(c.formations[1].name.as_deref(), Some("Intro"));
        let a = add_formation_after(&mut c, 0);
        assert_eq!(a, 1);
        assert_eq!(c.formations[1].name, None);
        assert_eq!(c.formations[1].duration, Formation::new().duration);
        assert_eq!(c.formations[1].units, c.formations[0].units);
        assert_eq!(c.formations.len(), 4);
        assert_eq!(move_formation(&mut c, 0, true), None);
        assert_eq!(move_formation(&mut c, 0, false), Some(1));
        assert_eq!(c.formations[1].name.as_deref(), Some("Intro"));
        assert_eq!(move_formation(&mut c, 3, false), None);
        assert_eq!(delete_formation(&mut c, 3), Some(2));
        delete_formation(&mut c, 0);
        delete_formation(&mut c, 0);
        assert_eq!(c.formations.len(), 1);
        assert_eq!(delete_formation(&mut c, 0), None);
        c.formations[0].name = None;
        assert_eq!(formation_label(&c, 0), "1");
    }

    #[test]
    fn editing_target_snaps_into_entered_formation() {
        let c = sample(); // f0 0..10, transition 10..13, f1 13..23
        assert_eq!(formation_at(&c, 5.0, None), 0);
        assert_eq!(formation_at(&c, 11.0, None), 1);
        assert_eq!(entered_formation(&c, 11.0, None), Some(1));
        assert_eq!(entered_formation(&c, 5.0, None), None);
        assert_eq!(entered_formation(&c, 15.0, None), None);
    }

    #[test]
    fn zero_second_formation_can_be_selected() {
        let mut c = sample(); // f0 0..10, transition 10..13, f1 13..23
        c.formations.push(c.formations[1].clone()); // transition 23..26, f2 26..36
        c.formations[1].duration = 0.0; // f1 is the instant 13
        let t = timeline::formation_time(&c, 1);
        assert_eq!(t, 13.0);
        // By time alone, 13 is already the start of the transition into f2...
        assert_eq!(formation_at(&c, t, None), 2);
        assert_eq!(entered_formation(&c, t, None), Some(2));
        // ...but picked explicitly, f1 is held there.
        assert_eq!(formation_at(&c, t, Some(1)), 1);
        assert_eq!(entered_formation(&c, t, Some(1)), None);
        // A pin elsewhere in time doesn't apply.
        assert_eq!(formation_at(&c, 5.0, Some(1)), 0);
        // Nor does one with no transition into the next formation: f1 and f2 share 13.
        c.formations[2].transition_duration = 0.0;
        assert_eq!(formation_at(&c, t, None), 2);
        assert_eq!(formation_at(&c, t, Some(1)), 1);
        assert_eq!(formation_at(&c, t, Some(2)), 2);
    }

    #[test]
    fn props_crud_and_interpolation() {
        let mut c = sample();
        let i = add_prop(&mut c, 0, PropType::Rectangle).unwrap();
        assert_eq!(props(&c, 0).len(), 1);
        c.formations[1].props = Some(vec![props(&c, 0)[0].clone()]);
        prop_mut(&mut c, 1, 0).unwrap().position = P::new(0.0, 0.0);
        let start = props(&c, 0)[0].position;
        let mid = props_in(&c, timeline::phase_at(&c, 11.5, Easing::Linear));
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].position, start.lerp(P::new(0.0, 0.0), 0.5));
        assert!(remove_prop(&mut c, 0, i));
        assert!(!remove_prop(&mut c, 0, 0));
        assert!(props_in(&c, timeline::phase_at(&c, 1.0, Easing::Linear)).is_empty());
    }

    #[test]
    fn names() {
        let mut c = sample();
        c.formations[1].name = Some(" Chorus ".into());
        assert_eq!(formation_label(&c, 1), "2 · Chorus");
    }
}
