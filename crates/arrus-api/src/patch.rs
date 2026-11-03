//! Changes to a choreography as small JSON patches, for live co-editing.
//!
//! A client compares the document it last sent with its current one
//! ([`diff`]) and sends the resulting operations; the server applies them to
//! the latest stored version ([`apply`]) and forwards them to everyone else
//! viewing the choreography.
//!
//! Every operation sets one value at a path (or removes an object key), so
//! concurrent edits merge field by field and the last write to the same
//! field wins, like two people typing into the same spreadsheet cell.
//! Applying an operation twice gives the same result, so a batch can be
//! resent safely after a dropped connection.
//!
//! Arrays whose length changed (a dancer or formation added or removed) are
//! sent whole; arrays of the same length are compared item by item. An
//! operation whose path no longer exists (someone else removed that part) is
//! skipped.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One step of a path: an object key or an array index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Seg {
    Index(usize),
    Key(String),
}

impl From<&str> for Seg {
    fn from(k: &str) -> Self {
        Seg::Key(k.to_owned())
    }
}

impl From<usize> for Seg {
    fn from(i: usize) -> Self {
        Seg::Index(i)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    /// Replace the value at `path` (or add the key, for an object).
    Set { path: Vec<Seg>, value: Value },
    /// Remove an object key.
    Remove { path: Vec<Seg> },
}

impl PatchOp {
    pub fn path(&self) -> &[Seg] {
        match self {
            PatchOp::Set { path, .. } | PatchOp::Remove { path } => path,
        }
    }
}

/// Where the (possibly multi-megabyte) custom background image lives.
pub const IMAGE_PATH: [&str; 3] = ["stageBackground", "file", "imageFileData"];

/// Whether applying `op` may change the background image.
pub fn touches_image(op: &PatchOp) -> bool {
    let p = op.path();
    p.len() <= IMAGE_PATH.len() && p.iter().zip(IMAGE_PATH).all(|(s, k)| matches!(s, Seg::Key(x) if x == k))
}

/// Take the background image out of a choreography value (leaving `null`).
pub fn take_image(doc: &mut Value) -> Option<String> {
    let slot = doc.get_mut(IMAGE_PATH[0])?.get_mut(IMAGE_PATH[1])?.get_mut(IMAGE_PATH[2])?;
    match slot.take() {
        Value::String(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// The operations that turn `old` into `new`.
pub fn diff(old: &Value, new: &Value) -> Vec<PatchOp> {
    let mut out = Vec::new();
    diff_into(&mut Vec::new(), old, new, &mut out);
    out
}

fn diff_into(path: &mut Vec<Seg>, old: &Value, new: &Value, out: &mut Vec<PatchOp>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, nv) in b {
                path.push(Seg::Key(k.clone()));
                match a.get(k) {
                    Some(ov) => diff_into(path, ov, nv, out),
                    None => out.push(PatchOp::Set { path: path.clone(), value: nv.clone() }),
                }
                path.pop();
            }
            for k in a.keys().filter(|k| !b.contains_key(*k)) {
                path.push(Seg::Key(k.clone()));
                out.push(PatchOp::Remove { path: path.clone() });
                path.pop();
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (ov, nv)) in a.iter().zip(b).enumerate() {
                path.push(Seg::Index(i));
                diff_into(path, ov, nv, out);
                path.pop();
            }
        }
        _ => out.push(PatchOp::Set { path: path.clone(), value: new.clone() }),
    }
}

/// Apply `ops` in order; returns how many applied (the others' paths no
/// longer exist).
pub fn apply(doc: &mut Value, ops: &[PatchOp]) -> usize {
    ops.iter().filter(|op| apply_one(doc, op)).count()
}

fn apply_one(doc: &mut Value, op: &PatchOp) -> bool {
    let path = op.path();
    let Some((last, parent_path)) = path.split_last() else {
        return match op {
            PatchOp::Set { value, .. } => {
                *doc = value.clone();
                true
            }
            PatchOp::Remove { .. } => false,
        };
    };
    let mut parent = doc;
    for seg in parent_path {
        let next = match (seg, parent) {
            (Seg::Key(k), Value::Object(m)) => m.get_mut(k),
            (Seg::Index(i), Value::Array(a)) => a.get_mut(*i),
            _ => None,
        };
        match next {
            Some(n) => parent = n,
            None => return false,
        }
    }
    match (op, last, parent) {
        (PatchOp::Set { value, .. }, Seg::Key(k), Value::Object(m)) => {
            m.insert(k.clone(), value.clone());
            true
        }
        (PatchOp::Set { value, .. }, Seg::Index(i), Value::Array(a)) if *i < a.len() => {
            a[*i] = value.clone();
            true
        }
        (PatchOp::Remove { .. }, Seg::Key(k), Value::Object(m)) => m.remove(k).is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roundtrip(old: Value, new: Value) {
        let ops = diff(&old, &new);
        let mut v = old;
        apply(&mut v, &ops);
        assert_eq!(v, new);
    }

    #[test]
    fn diff_then_apply_gives_the_new_value() {
        roundtrip(json!({"a": 1}), json!({"a": 2}));
        roundtrip(json!({"a": 1, "b": 2}), json!({"a": 1}));
        roundtrip(json!({"a": 1}), json!({"a": 1, "c": {"d": [1, 2]}}));
        roundtrip(json!({"f": [{"x": 1}, {"x": 2}]}), json!({"f": [{"x": 1}, {"x": 5}]}));
        roundtrip(json!({"f": [1, 2, 3]}), json!({"f": [1, 2]}));
        roundtrip(json!({"f": null}), json!({"f": {"g": 1}}));
        roundtrip(json!([1]), json!("whole"));
    }

    #[test]
    fn diffs_are_small() {
        let old = json!({"formations": [{"units": [{"x": 1, "y": 1}, {"x": 2, "y": 2}]}], "name": "n"});
        let mut new = old.clone();
        new["formations"][0]["units"][1]["x"] = json!(9);
        assert_eq!(
            diff(&old, &new),
            vec![PatchOp::Set { path: vec!["formations".into(), 0.into(), "units".into(), 1.into(), "x".into()], value: json!(9) }]
        );
        assert!(diff(&old, &old).is_empty());
        // A length change sends the whole array.
        let mut grown = old.clone();
        grown["formations"][0]["units"].as_array_mut().unwrap().push(json!({"x": 3, "y": 3}));
        let ops = diff(&old, &grown);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path(), &["formations".into(), Seg::Index(0), "units".into()]);
    }

    #[test]
    fn concurrent_edits_merge_and_last_write_wins() {
        let base = json!({"name": "a", "units": [{"x": 0, "y": 0}, {"x": 0, "y": 0}]});
        let mut alice = base.clone();
        alice["units"][0]["x"] = json!(1);
        alice["name"] = json!("alice");
        let mut bob = base.clone();
        bob["units"][1]["y"] = json!(2);
        bob["name"] = json!("bob");
        let mut server = base.clone();
        apply(&mut server, &diff(&base, &alice));
        apply(&mut server, &diff(&base, &bob));
        assert_eq!(server, json!({"name": "bob", "units": [{"x": 1, "y": 0}, {"x": 0, "y": 2}]}));
        // Resending a batch changes nothing more.
        let again = server.clone();
        apply(&mut server, &diff(&base, &bob));
        assert_eq!(server, again);
    }

    #[test]
    fn missing_paths_are_skipped() {
        let mut v = json!({"units": [{"x": 0}]});
        let ops = vec![
            PatchOp::Set { path: vec!["units".into(), 3.into(), "x".into()], value: json!(1) },
            PatchOp::Set { path: vec!["units".into(), 0.into(), "x".into()], value: json!(2) },
            PatchOp::Remove { path: vec!["nope".into()] },
        ];
        assert_eq!(apply(&mut v, &ops), 1);
        assert_eq!(v, json!({"units": [{"x": 2}]}));
    }

    #[test]
    fn ops_serialize_compactly() {
        let op = PatchOp::Set { path: vec!["units".into(), 2.into()], value: json!(1) };
        let s = serde_json::to_string(&op).unwrap();
        assert_eq!(s, r#"{"op":"set","path":["units",2],"value":1}"#);
        assert_eq!(serde_json::from_str::<PatchOp>(&s).unwrap(), op);
    }

    #[test]
    fn image_helpers() {
        let mut v = json!({"stageBackground": {"file": {"imageFileData": "AAAA"}}});
        assert!(touches_image(&PatchOp::Set { path: vec!["stageBackground".into()], value: json!(null) }));
        assert!(touches_image(&PatchOp::Set { path: vec![], value: json!(null) }));
        assert!(!touches_image(&PatchOp::Set { path: vec!["name".into()], value: json!(null) }));
        assert!(!touches_image(&PatchOp::Set { path: vec!["stageBackground".into(), "alpha".into()], value: json!(1) }));
        assert_eq!(take_image(&mut v).as_deref(), Some("AAAA"));
        assert_eq!(v["stageBackground"]["file"]["imageFileData"], json!(null));
        assert_eq!(take_image(&mut json!({"stageBackground": null})), None);
    }
}
