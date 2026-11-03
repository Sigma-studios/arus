//! Playback timing: where every dancer is at time `t`.
//!
//! Formations play in order. The first is held for its `duration`; each later one
//! is reached over its `transition_duration`, then held for its `duration`.
//! Dancers are matched across formations by index.

use crate::model::{Choreography, Position};

/// Time span of one formation on the timeline (seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub formation: usize,
    /// Transition into this formation starts here (equal to `hold_start` for the first).
    pub start: f64,
    pub hold_start: f64,
    pub end: f64,
}

/// Every formation's span, in order.
pub fn spans(c: &Choreography) -> Vec<Span> {
    let mut t = 0.0;
    c.formations
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let trans = if i == 0 { 0.0 } else { f.transition_duration.max(0.0) };
            let start = t;
            let hold_start = start + trans;
            t = hold_start + f.duration.max(0.0);
            Span { formation: i, start, hold_start, end: t }
        })
        .collect()
}

pub fn total_duration(c: &Choreography) -> f64 {
    spans(c).last().map(|s| s.end).unwrap_or(0.0)
}

/// Start of formation `index`'s hold (where a "go to formation" should seek).
pub fn formation_time(c: &Choreography, index: usize) -> f64 {
    spans(c).get(index).map(|s| s.hold_start).unwrap_or(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Standing in a formation.
    Hold { formation: usize },
    /// Moving from `from` to `to`; `progress` in 0..=1 (eased).
    Transition { from: usize, to: usize, progress: f64 },
}

impl Phase {
    /// The formation being held, or being moved into.
    pub fn current(self) -> usize {
        match self {
            Phase::Hold { formation } => formation,
            Phase::Transition { to, .. } => to,
        }
    }
}

/// Transition easing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    #[default]
    Linear,
    /// Smoothstep: slow start and end.
    InOut,
}

impl Easing {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::InOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

pub fn phase_at(c: &Choreography, t: f64, easing: Easing) -> Option<Phase> {
    let spans = spans(c);
    let last = spans.last()?;
    let t = t.clamp(0.0, last.end);
    for s in &spans {
        if t < s.end || s.formation == last.formation {
            if t < s.hold_start && s.formation > 0 {
                let len = s.hold_start - s.start;
                let p = if len > 0.0 { (t - s.start) / len } else { 1.0 };
                return Some(Phase::Transition { from: s.formation - 1, to: s.formation, progress: easing.apply(p) });
            }
            return Some(Phase::Hold { formation: s.formation });
        }
    }
    None
}

/// A dancer at a moment of the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DancerAt {
    pub index: usize,
    pub position: Position,
    /// 1 when present in the current formation; fades in or out when the dancer
    /// only exists on one side of a transition.
    pub opacity: f64,
}

/// Where every dancer is at time `t`.
pub fn dancers_at(c: &Choreography, t: f64, easing: Easing) -> Vec<DancerAt> {
    let Some(phase) = phase_at(c, t, easing) else { return Vec::new() };
    match phase {
        Phase::Hold { formation } => c.formations[formation]
            .units
            .iter()
            .enumerate()
            .map(|(index, u)| DancerAt { index, position: u.position, opacity: 1.0 })
            .collect(),
        Phase::Transition { from, to, progress } => {
            let a = &c.formations[from].units;
            let b = &c.formations[to].units;
            (0..a.len().max(b.len()))
                .map(|index| match (a.get(index), b.get(index)) {
                    (Some(ua), Some(ub)) => DancerAt {
                        index,
                        position: ua.position.lerp(ub.position, progress),
                        opacity: 1.0,
                    },
                    (Some(ua), None) => DancerAt { index, position: ua.position, opacity: 1.0 - progress },
                    (None, Some(ub)) => DancerAt { index, position: ub.position, opacity: progress },
                    (None, None) => unreachable!(),
                })
                .collect()
        }
    }
}
