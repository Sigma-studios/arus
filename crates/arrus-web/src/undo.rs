//! Snapshot undo/redo with coalescing of rapid edits to the same field.

/// Default number of undo steps kept.
pub const UNDO_LIMIT: usize = 200;

/// Edits with the same key within this many seconds share one undo step
/// (typing in a text field, dragging a slider).
pub const COALESCE_SECS: f64 = 1.5;

#[derive(Debug, Clone)]
pub struct UndoStack<T> {
    undo: Vec<T>,
    redo: Vec<T>,
    limit: usize,
    last_key: Option<String>,
    last_time: f64,
}

impl<T> UndoStack<T> {
    pub fn new(limit: usize) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: limit.max(1), last_key: None, last_time: f64::NEG_INFINITY }
    }

    /// Record `before` (the state prior to an edit). With a `key`, repeated
    /// edits of the same key in quick succession are merged into one step.
    /// Returns whether a new step was recorded.
    pub fn record(&mut self, before: impl FnOnce() -> T, key: Option<&str>, now: f64) -> bool {
        let merge = match (key, &self.last_key) {
            (Some(k), Some(last)) => k == last && now - self.last_time <= COALESCE_SECS,
            _ => false,
        };
        self.last_key = key.map(str::to_owned);
        self.last_time = now;
        if merge {
            return false;
        }
        self.undo.push(before());
        if self.undo.len() > self.limit {
            let excess = self.undo.len() - self.limit;
            self.undo.drain(..excess);
        }
        self.redo.clear();
        true
    }

    /// End coalescing: the next keyed edit starts a new step.
    pub fn break_merge(&mut self) {
        self.last_key = None;
    }

    pub fn undo(&mut self, current: T) -> Result<T, T> {
        self.last_key = None;
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(current);
                Ok(prev)
            }
            None => Err(current),
        }
    }

    pub fn redo(&mut self, current: T) -> Result<T, T> {
        self.last_key = None;
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(current);
                Ok(next)
            }
            None => Err(current),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_roundtrip() {
        let mut s = UndoStack::new(10);
        let mut v = 1;
        s.record(|| v, None, 0.0);
        v = 2;
        s.record(|| v, None, 0.1);
        v = 3;
        v = s.undo(v).unwrap();
        assert_eq!(v, 2);
        v = s.undo(v).unwrap();
        assert_eq!(v, 1);
        assert_eq!(s.undo(v), Err(1));
        v = s.redo(v).unwrap();
        assert_eq!(v, 2);
        v = s.redo(v).unwrap();
        assert_eq!(v, 3);
        assert!(s.redo(v).is_err());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut s = UndoStack::new(10);
        s.record(|| 1, None, 0.0);
        let v = s.undo(2).unwrap();
        assert!(s.can_redo());
        s.record(|| v, None, 1.0);
        assert!(!s.can_redo());
    }

    #[test]
    fn coalesces_same_key_quickly() {
        let mut s = UndoStack::new(10);
        assert!(s.record(|| "a", Some("name"), 0.0));
        assert!(!s.record(|| "ab", Some("name"), 0.5));
        assert!(!s.record(|| "abc", Some("name"), 1.0));
        assert!(s.record(|| "abcd", Some("name"), 5.0)); // too late
        assert!(s.record(|| "x", Some("other"), 5.1)); // other key
        assert!(s.record(|| "y", None, 5.2)); // unkeyed never merges
        s.break_merge();
        assert!(s.record(|| "z", Some("name"), 5.3));
        assert!(!s.record(|| "z", Some("name"), 5.4));
        s.break_merge();
        assert!(s.record(|| "z", Some("name"), 5.5));
        assert_eq!(s.len(), 6);
    }

    #[test]
    fn limit_drops_oldest() {
        let mut s = UndoStack::new(3);
        for i in 0..10 {
            s.record(|| i, None, i as f64);
        }
        assert_eq!(s.len(), 3);
        let a = s.undo(10).unwrap();
        let b = s.undo(a).unwrap();
        let c = s.undo(b).unwrap();
        assert_eq!((a, b, c), (9, 8, 7));
        assert!(!s.can_undo());
    }
}
