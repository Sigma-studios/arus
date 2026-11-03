//! Typed model of the JSON inside ArrangeUs files (app v2.9.4, schema version 8).
//!
//! Field order matches what the app writes, so an unmodified choreography
//! re-serializes byte for byte. Every struct keeps keys it does not know in
//! `extra`, so saving never drops data written by a newer app version.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// Keys this model does not know, kept as-is.
pub type Extra = Map<String, Value>;

/// Schema version written by ArrangeUs 2.9.4.
pub const SCHEMA_VERSION: i64 = 8;

/// Accepts `10` or `10.0` (the app types these as `num`).
fn de_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let v = Value::deserialize(d)?;
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(|| serde::de::Error::custom("bad number")),
        Value::Null => Ok(0),
        _ => Err(serde::de::Error::custom("expected a number")),
    }
}

fn de_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v = Value::deserialize(d)?;
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => Ok(n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))),
        _ => Err(serde::de::Error::custom("expected a number")),
    }
}

fn default_version() -> i64 {
    SCHEMA_VERSION
}

/// One choreography: the root of a `.arrus` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choreography {
    pub name: String,
    pub grid_size: SceneSize,
    #[serde(default)]
    pub formations: Vec<Formation>,
    #[serde(default, deserialize_with = "de_i64")]
    pub last_edit_timestamp: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub creation_timestamp: i64,
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_version", deserialize_with = "de_i64")]
    pub version: i64,
    #[serde(default)]
    pub description_text: Option<String>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub music_data: Option<MusicData>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub is_archived: Option<bool>,
    #[serde(default)]
    pub is_deleted: Option<bool>,
    #[serde(default)]
    pub stage_background: Option<StageBackground>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Choreography {
    /// A new, empty choreography on a stage of the app's default size, with one
    /// formation. `id` should be a fresh UUID; `now` is Unix seconds.
    pub fn new(name: impl Into<String>, id: impl Into<String>, now: i64) -> Self {
        Self {
            name: name.into(),
            grid_size: SceneSize::default(),
            formations: vec![Formation::new()],
            last_edit_timestamp: now,
            creation_timestamp: now,
            id: id.into(),
            version: SCHEMA_VERSION,
            description_text: None,
            settings: Settings::default(),
            music_data: None,
            is_active: Some(true),
            is_archived: Some(false),
            is_deleted: Some(false),
            stage_background: None,
            extra: Extra::new(),
        }
    }

    /// Number of dancers: the largest unit count of any formation.
    pub fn dancer_count(&self) -> usize {
        self.formations.iter().map(|f| f.units.len()).max().unwrap_or(0)
    }

    /// Add a dancer to every formation (dancers are matched by index across
    /// formations), at `position` in each. Returns its index.
    pub fn add_dancer(&mut self, unit: Unit) -> usize {
        let index = self.dancer_count();
        for f in &mut self.formations {
            while f.units.len() < index {
                f.units.push(unit.clone());
            }
            f.units.push(unit.clone());
        }
        index
    }

    /// Remove dancer `index` from every formation.
    pub fn remove_dancer(&mut self, index: usize) {
        for f in &mut self.formations {
            if index < f.units.len() {
                f.units.remove(index);
            }
        }
    }
}

/// The stage grid. Note the app's naming: `rows` is the extent along `x`
/// (across the stage) and `columns` the extent along `y` (front to back).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSize {
    #[serde(deserialize_with = "de_i64")]
    pub rows: i64,
    #[serde(deserialize_with = "de_i64")]
    pub columns: i64,
    #[serde(default)]
    pub backstage: BackstageSize,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for SceneSize {
    fn default() -> Self {
        Self { rows: 14, columns: 8, backstage: BackstageSize::default(), extra: Extra::new() }
    }
}

impl SceneSize {
    /// Stage width in cells (along `x`).
    pub fn width(&self) -> f64 {
        self.rows as f64
    }
    /// Stage depth in cells (along `y`, `0` = audience edge).
    pub fn depth(&self) -> f64 {
        self.columns as f64
    }
}

/// Extra off-stage cells: `sides` on both `x` ends, `front` before `y = 0`,
/// `back` beyond `y = depth`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackstageSize {
    #[serde(default, deserialize_with = "de_i64")]
    pub sides: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub back: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub front: i64,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for BackstageSize {
    fn default() -> Self {
        Self { sides: 2, back: 0, front: 0, extra: Extra::new() }
    }
}

/// One formation: where every dancer stands, held for `duration` seconds after
/// moving there over `transition_duration` seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Formation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub transition_duration: f64,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub units: Vec<Unit>,
    #[serde(default)]
    pub props: Option<Vec<Prop>>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub color: Option<i64>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Formation {
    pub fn new() -> Self {
        Self {
            name: None,
            duration: 10.0,
            transition_duration: 3.0,
            comments: Vec::new(),
            units: Vec::new(),
            props: Some(Vec::new()),
            color: Some(DEFAULT_COLOR),
            extra: Extra::new(),
        }
    }
}

impl Default for Formation {
    fn default() -> Self {
        Self::new()
    }
}

/// The app's default dancer/formation colour.
pub const DEFAULT_COLOR: i64 = 0xFFE9_2E67;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    #[serde(default)]
    pub text: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One dancer placed in one formation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unit {
    #[serde(default)]
    pub dancer: Dancer,
    pub position: Position,
    #[serde(default = "default_color", deserialize_with = "de_i64")]
    pub color: i64,
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

fn default_color() -> i64 {
    DEFAULT_COLOR
}

impl Unit {
    pub fn at(x: f64, y: f64) -> Self {
        Self {
            dancer: Dancer::default(),
            position: Position::new(x, y),
            color: DEFAULT_COLOR,
            shape: Some(DancerShape::Circle.as_str().to_owned()),
            short_name: None,
            extra: Extra::new(),
        }
    }

    pub fn shape(&self) -> DancerShape {
        self.shape.as_deref().map(DancerShape::parse).unwrap_or(DancerShape::Circle)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Dancer {
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Grid coordinates: `x` across the stage (grows towards stage right, the
/// performers' right), `y` from the audience edge (`0`) to the back.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn lerp(self, to: Position, t: f64) -> Position {
        Position::new(self.x + (to.x - self.x) * t, self.y + (to.y - self.y) * t)
    }
}

/// A stage object (a premium feature in the app; semantics partly guessed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prop {
    pub prop_type: String,
    /// Extent along `x`, in cells.
    #[serde(default)]
    pub width: f64,
    /// Extent along `y`, in cells.
    #[serde(default)]
    pub length: f64,
    /// Vertical height (3D view only).
    #[serde(default)]
    pub height: f64,
    pub position: Position,
    /// Radians (Flutter's convention; unconfirmed).
    #[serde(default)]
    pub rotation_angle: f64,
    #[serde(default = "default_color", deserialize_with = "de_i64")]
    pub color: i64,
    /// Opacity, 0 to 1.
    #[serde(default = "one")]
    pub alpha: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

fn one() -> f64 {
    1.0
}

impl Prop {
    pub fn kind(&self) -> PropType {
        PropType::parse(&self.prop_type)
    }
}

/// Per-choreography view options, with the app's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub is_grid_snapping_enabled: bool,
    pub is_dancer_names_shown: bool,
    pub is_dancer_indexes_shown: bool,
    pub is_ghost_dancers_shown: bool,
    pub is_additional_indicators_shown: bool,
    pub is_grid_shown: bool,
    pub is_scene_directions_shown: bool,
    pub is_scene_directions_inverted: bool,
    pub is_future_dancers_shown: bool,
    pub is_transition_paths_shown: bool,
    pub dancer_size_multiplier: f64,
    pub dancer_label_size_multiplier: f64,
    pub dancer_fill_style: String,
    pub is_backstage_shown: bool,
    pub is_distance_marks_shown: bool,
    pub is_subdivisions_shown: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            is_grid_snapping_enabled: true,
            is_dancer_names_shown: true,
            is_dancer_indexes_shown: false,
            is_ghost_dancers_shown: false,
            is_additional_indicators_shown: true,
            is_grid_shown: true,
            is_scene_directions_shown: true,
            is_scene_directions_inverted: false,
            is_future_dancers_shown: false,
            is_transition_paths_shown: false,
            dancer_size_multiplier: 0.7,
            dancer_label_size_multiplier: 1.0,
            dancer_fill_style: DancerFillStyle::Indexed.as_str().to_owned(),
            is_backstage_shown: false,
            is_distance_marks_shown: false,
            is_subdivisions_shown: false,
            extra: Extra::new(),
        }
    }
}

impl Settings {
    pub fn fill_style(&self) -> DancerFillStyle {
        DancerFillStyle::parse(&self.dancer_fill_style)
    }
}

/// Points at an audio file on the phone; the audio itself is never in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicData {
    #[serde(default)]
    pub music_type: String,
    #[serde(default)]
    pub music_address: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBackground {
    #[serde(default)]
    pub file: Option<StageBackgroundFile>,
    #[serde(default = "default_aspect")]
    pub aspect_ratio: String,
    #[serde(default = "one")]
    pub alpha: f64,
    #[serde(default)]
    pub is_backstage_separated: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

fn default_aspect() -> String {
    "scaleToFit".into()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBackgroundFile {
    /// A built-in background (see [`DefaultStageBackground`]).
    #[serde(default)]
    pub standard_background_value: Option<String>,
    /// Path of a custom image on the phone.
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub last_selection_timestamp: Option<i64>,
    /// The custom image itself, base64, filled in by the app on export.
    #[serde(default)]
    pub image_file_data: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A full backup: the root of a `.aubkp` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    #[serde(default, deserialize_with = "de_i64")]
    pub version: i64,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub choreographies: Vec<Choreography>,
    #[serde(default)]
    pub crews: Vec<Value>,
    #[serde(default)]
    pub presets: Vec<Value>,
    #[serde(default)]
    pub backgrounds: Vec<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }
            /// Unknown values map to the first variant.
            pub fn parse(s: &str) -> Self {
                match s { $($s => $name::$variant,)+ _ => Self::ALL[0] }
            }
        }
    };
}

string_enum!(DancerShape {
    Circle = "circle",
    Square = "square",
    RotatedSquare = "rotatedSquare",
    Triangle = "triangle",
    UpsideDownTriangle = "upsideDownTriangle",
    LeftTriangle = "leftTriangle",
    RightTriangle = "rightTriangle",
    TopLeftTriangle = "topLeftTriangle",
    TopRightTriangle = "topRightTriangle",
    BottomLeftTriangle = "bottomLeftTriangle",
    BottomRightTriangle = "bottomRightTriangle",
});

string_enum!(DancerFillStyle {
    Indexed = "indexed",
    Solid = "solid",
    ShortNamed = "shortNamed",
});

string_enum!(PropType {
    Rectangle = "rectangle",
    Ellipse = "ellipse",
    Triangle = "triangle",
});

string_enum!(StageAspectRatio {
    ScaleToFit = "scaleToFit",
    ScaleToFill = "scaleToFill",
    StretchToFill = "stretchToFill",
});

string_enum!(DefaultStageBackground {
    Unknown = "unknown",
    BasketballField = "basketballField",
    FootballField = "footballField",
    NflField = "nflField",
    NflFieldSmall = "nflFieldSmall",
    SevenCheerleadingMats = "sevenCheerleadingMats",
    EightCheerleadingMats = "eightCheerleadingMats",
    NineCheerleadingMats = "nineCheerleadingMats",
    TenCheerleadingMats = "tenCheerleadingMats",
});

/// An ARGB colour as stored in the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argb {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Argb {
    pub fn from_i64(v: i64) -> Self {
        let v = v as u32;
        Self { a: (v >> 24) as u8, r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
    }
    pub fn to_i64(self) -> i64 {
        ((self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32) as i64
    }
}
