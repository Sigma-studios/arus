//! Other people's dancers move smoothly: while they drag, their dancers
//! follow the positions streamed over the live socket; when their change
//! lands, the dancers glide there instead of jumping.
//!
//! Only what is shown changes: the document always holds the stored positions.

use std::collections::HashMap;

use arrus_api::MovingDancer;
use arrus_core::timeline::{DancerAt, Easing, Phase};

use super::editor::Editor;
use super::stage::Drag;
use crate::geom::P;

/// Following a drag: a bit longer than the time between two streamed
/// positions (0.1 s), so the motion never stalls.
const FOLLOW_SECS: f64 = 0.12;
/// Gliding to a change that arrived in one go.
const CHANGE_SECS: f64 = 0.3;
/// How long a dragged dancer stays where it was last seen while its change
/// is on its way (after that it goes back to the stored position).
const HOLD_SECS: f64 = 3.0;

#[derive(Clone, Copy)]
struct Glide {
    from: P,
    to: P,
    start: f64,
    secs: f64,
    /// Linear (following a stream) or eased (a single change).
    eased: bool,
    hold_until: f64,
}

impl Glide {
    fn at(&self, now: f64) -> P {
        let k = ((now - self.start) / self.secs).clamp(0.0, 1.0);
        self.from.lerp(self.to, if self.eased { Easing::InOut.apply(k) } else { k })
    }

    fn done(&self, now: f64) -> bool {
        now >= self.start + self.secs
    }
}

/// Dancers shown away from their stored position, by (formation, dancer).
#[derive(Default)]
pub struct Motion {
    glides: HashMap<(usize, usize), Glide>,
}

impl Editor {
    fn stored_pos(&self, f: usize, i: usize) -> Option<P> {
        self.doc.formations.get(f)?.units.get(i).map(|u| u.position)
    }

    fn shown_pos(&self, f: usize, i: usize) -> Option<P> {
        self.motion.glides.get(&(f, i)).map(|g| g.at(self.now)).or_else(|| self.stored_pos(f, i))
    }

    /// Dancers this session is dragging (they follow the pointer, not others).
    fn dragging_here(&self, i: usize) -> bool {
        matches!(&self.drag, Some(Drag::Dancers { starts, .. }) if starts.iter().any(|(d, _)| *d == i))
    }

    /// Someone is dragging these dancers of formation `f` right now.
    pub fn remote_moving(&mut self, f: usize, dancers: &[MovingDancer]) {
        let now = self.now;
        for d in dancers {
            if self.dragging_here(d.index) {
                continue;
            }
            let Some(from) = self.shown_pos(f, d.index) else { continue };
            let to = P::new(d.x, d.y);
            let g = Glide { from, to, start: now, secs: FOLLOW_SECS, eased: false, hold_until: now + HOLD_SECS };
            self.motion.glides.insert((f, d.index), g);
        }
    }

    /// Where formation `f`'s dancers are shown now (before someone else's change).
    pub fn shown_positions(&self, f: usize) -> Vec<Option<P>> {
        let n = self.doc.formations.get(f).map_or(0, |x| x.units.len());
        (0..n).map(|i| self.shown_pos(f, i)).collect()
    }

    /// After someone else's change: dancers of `f` it moved glide there
    /// from where they were shown.
    pub fn glide_to_changes(&mut self, f: usize, before: &[Option<P>]) {
        let now = self.now;
        for (i, b) in before.iter().enumerate() {
            let (Some(from), Some(to)) = (*b, self.stored_pos(f, i)) else { continue };
            if from != to && !self.dragging_here(i) {
                let g = Glide { from, to, start: now, secs: CHANGE_SECS, eased: true, hold_until: now + CHANGE_SECS };
                self.motion.glides.insert((f, i), g);
            }
        }
    }

    /// Every frame: drop finished glides. Returns whether any is still going.
    pub fn tick_motion(&mut self) -> bool {
        if self.motion.glides.is_empty() {
            return false;
        }
        let now = self.now;
        let keys: Vec<(usize, usize)> = self.motion.glides.keys().copied().collect();
        for k in keys {
            let g = self.motion.glides[&k];
            let stored = self.stored_pos(k.0, k.1);
            if !g.done(now) {
                continue;
            }
            match stored {
                Some(s) if s == g.to => {
                    self.motion.glides.remove(&k);
                }
                Some(s) if now >= g.hold_until => {
                    // The change never came (a cancelled drag): back to the stored place.
                    let back = Glide { from: g.to, to: s, start: now, secs: CHANGE_SECS, eased: true, hold_until: now + CHANGE_SECS };
                    self.motion.glides.insert(k, back);
                }
                Some(_) => {}
                None => {
                    self.motion.glides.remove(&k);
                }
            }
        }
        !self.motion.glides.is_empty()
    }

    /// Show the gliding dancers where they are now (holds only).
    pub fn apply_motion(&self, phase: Option<Phase>, dancers: &mut [DancerAt]) {
        let Some(Phase::Hold { formation }) = phase else { return };
        if self.motion.glides.is_empty() {
            return;
        }
        for d in dancers {
            if let Some(g) = self.motion.glides.get(&(formation, d.index))
                && !self.dragging_here(d.index)
            {
                d.position = g.at(self.now);
            }
        }
    }
}
