//! Editing operations on a [`Choreography`], selection and hit-testing.
//! Pure logic: no egui painting, no web APIs.

use std::collections::BTreeSet;

use arrus_core::timeline::{self, Easing, Phase};
use arrus_core::{Choreography, DEFAULT_COLOR, Formation, Prop, PropType, Unit};

use crate::geom::{self, Bounds, P};

/// What is selected on the stage. Dancers are indices (valid in every
/// formation); props are indices into the current formation's props.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Selection {
    #[default]
    None,
    Dancers(BTreeSet<usize>),
    Props(BTreeSet<usize>),
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

    pub fn props(&self) -> Option<&BTreeSet<usize>> {
        match self {
            Selection::Props(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    pub fn has_prop(&self, i: usize) -> bool {
        matches!(self, Selection::Props(s) if s.contains(&i))
    }

    pub fn one_prop(i: usize) -> Self {
        Selection::Props(BTreeSet::from([i]))
    }

    /// The prop, when exactly one is selected (it gets the resize and rotate handles).
    pub fn single_prop(&self) -> Option<usize> {
        match self {
            Selection::Props(s) if s.len() == 1 => s.first().copied(),
            _ => None,
        }
    }

    /// Shift-click on a prop: add or remove it.
    pub fn toggle_prop(&mut self, i: usize) {
        match self {
            Selection::Props(s) => {
                if !s.remove(&i) {
                    s.insert(i);
                }
                if s.is_empty() {
                    *self = Selection::None;
                }
            }
            _ => *self = Selection::one_prop(i),
        }
    }

    /// How many things are selected.
    pub fn len(&self) -> usize {
        match self {
            Selection::Dancers(s) | Selection::Props(s) => s.len(),
            Selection::None => 0,
        }
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
            Selection::Props(s) => {
                s.retain(|&i| i < props);
                if s.is_empty() {
                    *self = Selection::None;
                }
            }
            Selection::None => {}
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

/// Props whose centre is inside the box spanned by `a` and `b`.
pub fn box_select_props(props: &[Prop], a: P, b: P) -> BTreeSet<usize> {
    let bx = Bounds::new(P::new(a.x.min(b.x), a.y.min(b.y)), P::new(a.x.max(b.x), a.y.max(b.y)));
    props.iter().enumerate().filter(|(_, p)| bx.contains(p.position)).map(|(i, _)| i).collect()
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

/// Set the colour of dancer `index` in formation `f` only, or in every
/// formation when `f` is `None`.
pub fn set_dancer_color(c: &mut Choreography, index: usize, f: Option<usize>, color: i64) {
    match f {
        Some(f) => {
            if let Some(u) = c.formations.get_mut(f).and_then(|fm| fm.units.get_mut(index)) {
                u.color = color;
            }
        }
        None => edit_dancer_everywhere(c, index, |u| u.color = color),
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
    stamp_prop_numbers(c);
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
    stamp_prop_numbers(c);
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
        number: None,
        extra: Default::default(),
    }
}

/// The number shown on a prop (the same in every formation it is in).
pub fn prop_number(p: &Prop, index: usize) -> u32 {
    p.number.unwrap_or(index as u32 + 1)
}

/// Store every prop's number, so removing or adding props cannot renumber
/// the others. Props without one get their place in the list (as before
/// numbers existed, when transitions paired props by place).
pub fn stamp_prop_numbers(c: &mut Choreography) {
    for f in &mut c.formations {
        for (i, p) in f.props.iter_mut().flatten().enumerate() {
            p.number.get_or_insert(i as u32 + 1);
        }
    }
}

/// The lowest number no prop uses in any formation.
fn next_prop_number(c: &Choreography) -> u32 {
    c.formations
        .iter()
        .flat_map(|f| f.props.iter().flatten().enumerate().map(|(i, p)| prop_number(p, i)))
        .max()
        .unwrap_or(0)
        + 1
}

/// Index of prop number `n` in formation `f`.
pub fn find_prop(c: &Choreography, f: usize, n: u32) -> Option<usize> {
    props(c, f).iter().enumerate().position(|(i, p)| prop_number(p, i) == n)
}

/// The run of formations around `f` that have prop number `n` (inclusive).
pub fn prop_span(c: &Choreography, f: usize, n: u32) -> (usize, usize) {
    let has = |i: usize| find_prop(c, i, n).is_some();
    let (mut a, mut b) = (f, f);
    while a > 0 && has(a - 1) {
        a -= 1;
    }
    while b + 1 < c.formations.len() && has(b + 1) {
        b += 1;
    }
    (a, b)
}

/// Show prop `i` of formation `f` exactly in formations `from..=to`: copied
/// (as it is in `f`) into those that lack it, removed from all others.
/// Formations that already have it keep their own version of it.
pub fn set_prop_span(c: &mut Choreography, f: usize, i: usize, from: usize, to: usize) {
    stamp_prop_numbers(c);
    let Some(p) = props(c, f).get(i).cloned() else { return };
    let n = prop_number(&p, i);
    let (from, to) = (from.min(to), from.max(to));
    for (k, fm) in c.formations.iter_mut().enumerate() {
        let list = fm.props.get_or_insert_with(Vec::new);
        let at = list.iter().position(|q| q.number == Some(n));
        match (from <= k && k <= to, at) {
            (true, None) => list.push(p.clone()),
            (false, Some(j)) => {
                list.remove(j);
            }
            _ => {}
        }
    }
}

/// Add a prop to formation `f`; returns its index.
pub fn add_prop(c: &mut Choreography, f: usize, kind: PropType) -> Option<usize> {
    stamp_prop_numbers(c);
    let mut p = new_prop(c, kind);
    p.number = Some(next_prop_number(c));
    let props = c.formations.get_mut(f)?.props.get_or_insert_with(Vec::new);
    props.push(p);
    Some(props.len() - 1)
}

/// Marks a copied prop on the system clipboard (so it can be pasted in
/// another tab or choreography).
const PROP_CLIP_PREFIX: &str = "eightcount-prop:";

/// Props as clipboard text.
pub fn props_to_clipboard(ps: &[Prop]) -> String {
    format!("{PROP_CLIP_PREFIX}{}", serde_json::to_string(ps).unwrap_or_default())
}

/// Props from clipboard text, if that is what was copied (a list, or one
/// prop as copied by an earlier version).
pub fn props_from_clipboard(s: &str) -> Option<Vec<Prop>> {
    let json = s.trim().strip_prefix(PROP_CLIP_PREFIX)?;
    let ps = serde_json::from_str::<Vec<Prop>>(json).ok().or_else(|| serde_json::from_str::<Prop>(json).ok().map(|p| vec![p]))?;
    (!ps.is_empty()).then_some(ps)
}

/// Paste copies of `ps` into formation `f`, at the same spots. If one of
/// them would land exactly on the very same prop (pasted twice), the whole
/// group moves a little. Returns their indices.
pub fn paste_props(c: &mut Choreography, f: usize, mut ps: Vec<Prop>) -> BTreeSet<usize> {
    stamp_prop_numbers(c);
    // A pasted prop stays the same prop (same number) unless the formation
    // already has that number: then it is a new prop.
    let mut taken: BTreeSet<u32> = props(c, f).iter().enumerate().map(|(i, p)| prop_number(p, i)).collect();
    let mut next = next_prop_number(c);
    for p in &mut ps {
        match p.number {
            Some(n) if !taken.contains(&n) => {
                taken.insert(n);
            }
            _ => {
                p.number = Some(next);
                taken.insert(next);
                next += 1;
            }
        }
    }
    let bounds = geom::drawable_bounds(c);
    let clash = |c: &Choreography, ps: &[Prop]| {
        ps.iter().any(|p| props(c, f).iter().any(|q| q.position == p.position && q.prop_type == p.prop_type))
    };
    for _ in 0..40 {
        if !clash(c, &ps) {
            break;
        }
        for p in &mut ps {
            p.position = bounds.clamp(geom::add(p.position, P::new(0.5, 0.5)));
        }
    }
    let Some(props) = c.formations.get_mut(f).map(|f| f.props.get_or_insert_with(Vec::new)) else {
        return BTreeSet::new();
    };
    let first = props.len();
    props.extend(ps);
    (first..props.len()).collect()
}

/// Remove several props of formation `f`.
pub fn remove_props(c: &mut Choreography, f: usize, which: &BTreeSet<usize>) {
    for &i in which.iter().rev() {
        remove_prop(c, f, i);
    }
}

/// Move props of formation `f` together: the `anchor_start` point follows
/// the pointer by `delta` (snapped), the others keep their offsets; each
/// stays inside `bounds`. `starts` are positions at the start of the drag.
pub fn drag_props(
    c: &mut Choreography,
    f: usize,
    starts: &[(usize, P)],
    anchor_start: P,
    delta: P,
    snap: Option<f64>,
    bounds: Bounds,
) {
    let mut to = geom::add(anchor_start, delta);
    if let Some(s) = snap {
        to = geom::snap_pos(to, s);
    }
    let d = geom::sub(to, anchor_start);
    for &(i, start) in starts {
        if let Some(p) = prop_mut(c, f, i) {
            p.position = bounds.clamp(geom::add(start, d));
        }
    }
}

pub fn props(c: &Choreography, f: usize) -> &[Prop] {
    c.formations.get(f).and_then(|f| f.props.as_deref()).unwrap_or(&[])
}

pub fn prop_mut(c: &mut Choreography, f: usize, i: usize) -> Option<&mut Prop> {
    c.formations.get_mut(f)?.props.as_mut()?.get_mut(i)
}

pub fn remove_prop(c: &mut Choreography, f: usize, i: usize) -> bool {
    stamp_prop_numbers(c);
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
                // The same prop is the one with the same number.
                .map(|(i, pb)| match a.iter().enumerate().find(|(j, pa)| prop_number(pa, *j) == prop_number(pb, i)).map(|(_, pa)| pa) {
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
        let mut s = Selection::one_prop(2);
        s.sanitize(3, 2);
        assert_eq!(s, Selection::None);
        let mut s = Selection::one(1);
        s.toggle_prop(0);
        s.toggle_prop(3);
        assert_eq!(s, Selection::Props(BTreeSet::from([0, 3])));
        assert_eq!(s.single_prop(), None);
        s.toggle_prop(0);
        assert_eq!(s.single_prop(), Some(3));
        s.sanitize(5, 3);
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

    #[test]
    fn dancer_colour_in_one_or_every_formation() {
        let mut c = new_choreography("t", "id", 0, 2);
        add_formation_after(&mut c, 0);
        add_formation_after(&mut c, 1);
        set_dancer_color(&mut c, 0, Some(1), 0xFF000001);
        assert_eq!(c.formations[1].units[0].color, 0xFF000001);
        assert_ne!(c.formations[0].units[0].color, 0xFF000001);
        assert_ne!(c.formations[2].units[0].color, 0xFF000001);
        set_dancer_color(&mut c, 1, None, 0xFF000002);
        assert!(c.formations.iter().all(|f| f.units[1].color == 0xFF000002));
        // A new formation starts with the colours of the one it follows.
        let n = add_formation_after(&mut c, 1);
        assert_eq!(c.formations[n].units[0].color, 0xFF000001);
    }

    #[test]
    fn props_copy_and_paste_into_another_formation() {
        let mut c = new_choreography("t", "id", 0, 1);
        add_formation_after(&mut c, 0);
        let i = add_prop(&mut c, 0, PropType::ALL[0]).unwrap();
        let mut p = props(&c, 0)[i].clone();
        p.color = 0xFF112233;
        let mut q = p.clone();
        q.position = geom::add(q.position, P::new(2.0, 0.0));
        let text = props_to_clipboard(&[p.clone(), q.clone()]);
        let back = props_from_clipboard(&text).unwrap();
        assert_eq!(back, vec![p.clone(), q.clone()]);
        assert_eq!(props_from_clipboard("#FF0000"), None);
        // One prop, as copied before several could be.
        let one = format!("eightcount-prop:{}", serde_json::to_string(&p).unwrap());
        assert_eq!(props_from_clipboard(&one), Some(vec![p.clone()]));

        let n = props(&c, 1).len();
        let got = paste_props(&mut c, 1, back.clone());
        assert_eq!(got, BTreeSet::from([n, n + 1]));
        assert_eq!(props(&c, 1)[n].position, p.position, "same spot in another formation");
        // Pasted again into the same formation: the group moves off the first copies, keeping its shape.
        let again: Vec<usize> = paste_props(&mut c, 1, back).into_iter().collect();
        let (a, b) = (props(&c, 1)[again[0]].position, props(&c, 1)[again[1]].position);
        assert_ne!(a, p.position);
        assert_eq!(geom::sub(b, a), geom::sub(q.position, p.position));

        let mut c2 = c.clone();
        remove_props(&mut c2, 1, &again.iter().copied().collect());
        assert_eq!(props(&c2, 1).len(), n + 2);
    }

    #[test]
    fn props_keep_their_numbers() {
        let mut c = new_choreography("t", "id", 0, 1);
        add_formation_after(&mut c, 0);
        add_formation_after(&mut c, 1);
        let a = add_prop(&mut c, 0, PropType::ALL[0]).unwrap();
        let b = add_prop(&mut c, 0, PropType::ALL[0]).unwrap();
        assert_eq!((prop_number(&props(&c, 0)[a], a), prop_number(&props(&c, 0)[b], b)), (1, 2));
        // Removing the first does not renumber the second.
        remove_prop(&mut c, 0, a);
        assert_eq!(prop_number(&props(&c, 0)[0], 0), 2);
        // A new prop gets a number no prop uses.
        let n = add_prop(&mut c, 1, PropType::ALL[0]).unwrap();
        assert_eq!(prop_number(&props(&c, 1)[n], n), 3);

        // Shown in formations 1 to 3 (0-based 0..=2): copied into 1 and 2.
        set_prop_span(&mut c, 0, 0, 0, 2);
        assert!((0..3).all(|f| find_prop(&c, f, 2).is_some()));
        assert_eq!(prop_span(&c, 1, 2), (0, 2));
        // Only the last two: removed from the first.
        let at = find_prop(&c, 1, 2).unwrap();
        set_prop_span(&mut c, 1, at, 1, 2);
        assert_eq!(find_prop(&c, 0, 2), None);
        assert_eq!(prop_span(&c, 2, 2), (1, 2));

        // Pasting into a formation that already has that number makes a new prop.
        let p = props(&c, 1)[find_prop(&c, 1, 2).unwrap()].clone();
        let got = paste_props(&mut c, 0, vec![p.clone()]);
        assert_eq!(props(&c, 0)[*got.first().unwrap()].number, Some(2), "same prop in another formation");
        let got = paste_props(&mut c, 1, vec![p]);
        assert_eq!(props(&c, 1)[*got.first().unwrap()].number, Some(4));
    }

    #[test]
    fn legacy_props_are_numbered_by_place() {
        let mut c = new_choreography("t", "id", 0, 1);
        let mut p = new_prop(&c, PropType::ALL[0]);
        c.formations[0].props = Some(vec![p.clone(), p.clone()]);
        assert_eq!(prop_number(&c.formations[0].props.as_ref().unwrap()[1], 1), 2);
        stamp_prop_numbers(&mut c);
        assert_eq!(c.formations[0].props.as_ref().unwrap()[1].number, Some(2));
        // Unset numbers are not written to the file.
        p.number = None;
        assert!(!serde_json::to_string(&p).unwrap().contains("number"));
    }
}

