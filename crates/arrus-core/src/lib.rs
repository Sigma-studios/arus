//! ArrangeUs choreography files (`.arrus`, `.aubkp`): the encrypted container,
//! a typed model of their JSON, and playback timing.
//!
//! See `docs/arrus-format.md` for the format and schema.

pub mod format;
pub mod layout;
pub mod model;
pub mod pdf;
pub mod timeline;

pub use format::{FormatKey, Imported};
pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an ArrangeUs file (or a different key)")]
    NotArrus,
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the format key must be 32 bytes, got {0}")]
    KeyLength(usize),
    #[error("no format key: set ARUS_FORMAT_KEY or create .arrus-key")]
    NoKey,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{Easing, Phase};

    const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples");

    fn key() -> Option<FormatKey> {
        FormatKey::from_env_or_file().ok()
    }

    fn sample_json(name: &str) -> String {
        std::fs::read_to_string(format!("{SAMPLES}/{name}")).unwrap()
    }

    #[test]
    fn parses_and_reserializes_exactly() {
        for name in ["Test.json", "Test-v2.json"] {
            let original: serde_json::Value = serde_json::from_str(&sample_json(name)).unwrap();
            let c: Choreography = serde_json::from_value(original.clone()).unwrap();
            let back = serde_json::to_value(&c).unwrap();
            assert_eq!(original, back, "{name}");
            assert_eq!(c.formations[0].units.len(), 12);
        }
    }

    #[test]
    fn files_roundtrip_byte_for_byte() {
        let Some(key) = key() else {
            eprintln!("skipped: no format key");
            return;
        };
        for name in ["Test.arrus", "Test-v2.arrus"] {
            let blob = std::fs::read(format!("{SAMPLES}/{name}")).unwrap();
            let c = match format::read(&key, &blob).unwrap() {
                Imported::Choreography(c) => c,
                Imported::Backup(_) => panic!("expected a choreography"),
            };
            let iv: [u8; 16] = blob[..16].try_into().unwrap();
            assert_eq!(format::write_choreography(&key, &c, iv).unwrap(), blob, "{name}");
        }
    }

    #[test]
    fn unknown_fields_survive() {
        let mut v: serde_json::Value = serde_json::from_str(&sample_json("Test.json")).unwrap();
        v["futureField"] = serde_json::json!({"a": 1});
        v["formations"][0]["units"][0]["glow"] = serde_json::json!(true);
        let c: Choreography = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&c).unwrap(), v);
    }

    #[test]
    fn timeline() {
        let mut c: Choreography = serde_json::from_str(&sample_json("Test.json")).unwrap();
        let mut f2 = c.formations[0].clone();
        f2.units[0].position = Position::new(5.0, 1.0);
        f2.duration = 4.0;
        f2.transition_duration = 2.0;
        c.formations.push(f2);
        // f0 held 0..10, transition 10..12, f1 held 12..16
        assert_eq!(timeline::total_duration(&c), 16.0);
        assert_eq!(timeline::phase_at(&c, 5.0, Easing::Linear), Some(Phase::Hold { formation: 0 }));
        assert_eq!(
            timeline::phase_at(&c, 11.0, Easing::Linear),
            Some(Phase::Transition { from: 0, to: 1, progress: 0.5 })
        );
        let d = timeline::dancers_at(&c, 11.0, Easing::Linear);
        assert_eq!(d[0].position, Position::new(3.0, 1.0));
        assert_eq!(timeline::phase_at(&c, 99.0, Easing::Linear), Some(Phase::Hold { formation: 1 }));
    }

    #[test]
    fn add_and_remove_dancers_keep_formations_aligned() {
        let mut c = Choreography::new("x", "id", 0);
        c.formations.push(Formation::new());
        let i = c.add_dancer(Unit::at(1.0, 1.0));
        assert_eq!(i, 0);
        c.add_dancer(Unit::at(2.0, 1.0));
        assert!(c.formations.iter().all(|f| f.units.len() == 2));
        c.remove_dancer(0);
        assert!(c.formations.iter().all(|f| f.units.len() == 1 && f.units[0].position.x == 2.0));
    }

    #[test]
    fn colors() {
        let c = Argb::from_i64(4293471847);
        assert_eq!((c.a, c.r, c.g, c.b), (0xFF, 0xE9, 0x2E, 0x67));
        assert_eq!(c.to_i64(), 4293471847);
    }
}
