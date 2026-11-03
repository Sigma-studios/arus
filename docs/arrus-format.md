# ArrangeUs `.arrus` / `.aubkp` file format

Parse (and write) choreography files exported by the **ArrangeUs** Android app
(`com.AndriyStefanchuk.ArrangeUs.main`, v2.9.4), so you can build a visualiser.

## File format

```
file   = IV (16 bytes)  ||  ciphertext
cipher = AES-256-CBC, PKCS7 padding
key    = a fixed 32-byte ASCII secret compiled into the app (not published here;
         tools read it from ARUS_FORMAT_KEY or a .arrus-key file)
plain  = UTF-8 JSON
```

Two file types use the same encryption:

| extension | contents |
|---|---|
| `.arrus` | one **Choreography** (shared from the app) |
| `.aubkp` | a full **ChoreographiesBackup** (all choreographies, crews, presets, backgrounds) |

The key is a fixed secret compiled into the app (`utilities/data_encryptor.dart`),
so there is no per-file password.

## Full JSON schema

Recovered from the app's own `fromJson`/`toJson` code (json_serializable + hand-written
parsers), not just from a sample, so this is every key the app reads or writes.
`?` = nullable / may be absent. "default" = value the app uses when the key is missing.
Numbers typed `num` may arrive as int (`10`) or float (`10.0`); treat them as floats.

### Choreography (root of `.arrus`)

| key | type | notes |
|---|---|---|
| `name` | string | |
| `gridSize` | SceneSize | stage grid |
| `formations` | Formation[] | ordered timeline |
| `lastEditTimestamp` | num | Unix seconds |
| `creationTimestamp` | num | Unix seconds |
| `id` | string | UUID |
| `version` | num | schema version (8 in v2.9.4) |
| `descriptionText` | string? | |
| `settings` | Settings | view options |
| `musicData` | MusicData? | see "Portability" below |
| `isActive` / `isArchived` / `isDeleted` | bool? | lifecycle flags |
| `stageBackground` | StageBackground? | |

### SceneSize (`gridSize`)

| key | type | notes |
|---|---|---|
| `rows` | int | stage depth in cells |
| `columns` | int | stage width in cells |
| `backstage` | `{sides:int, back:int, front:int}` | extra off-stage cells on each edge |

### Formation

| key | type | notes |
|---|---|---|
| `name` | string? | optional formation label |
| `duration` | num | seconds the formation is held |
| `transitionDuration` | num | seconds to move into it from the previous one |
| `comments` | `{text:string}[]` | notes attached to the formation |
| `units` | Unit[] | dancers' positions |
| `props` | Prop[]? | stage objects |
| `color` | int? | ARGB 32-bit |

### Unit (one dancer in one formation)

| key | type | notes |
|---|---|---|
| `dancer` | `{name:string}` | identity; same dancer across formations |
| `position` | `{x:num, y:num}` | grid cells, fractional allowed (e.g. `3.5`) |
| `color` | num | ARGB 32-bit, e.g. `4293471847` = `0xFFE92E67` (A=FF, R=E9, G=2E, B=67) |
| `shape` | string? | default `"circle"`; see DancerShape |
| `shortName` | string? | short label (used when fill style is `shortNamed`) |

**DancerShape**: `circle`, `square`, `rotatedSquare`, `triangle`, `upsideDownTriangle`,
`leftTriangle`, `rightTriangle`, `topLeftTriangle`, `topRightTriangle`,
`bottomLeftTriangle`, `bottomRightTriangle`.

### Prop

| key | type | notes |
|---|---|---|
| `propType` | string | `rectangle` \| `ellipse` \| `triangle` |
| `width` | num | |
| `length` | num | |
| `height` | num | (the app has a 3D view, so likely vertical height) |
| `position` | `{x, y}` | grid cells |
| `rotationAngle` | num | units not yet confirmed (degrees vs radians) |
| `color` | int | ARGB |
| `alpha` | num | opacity |

### Settings

All bool unless noted; defaults in brackets.
`isGridSnappingEnabled` [true], `isDancerNamesShown` [true], `isDancerIndexesShown` [false],
`isGhostDancersShown` [false], `isAdditionalIndicatorsShown` [true], `isGridShown` [true],
`isSceneDirectionsShown` [true], `isSceneDirectionsInverted` [false],
`isFutureDancersShown` [false], `isTransitionPathsShown` [false],
`dancerSizeMultiplier` num [0.7], `dancerLabelSizeMultiplier` num [1.0],
`dancerFillStyle` string [`indexed`] (`solid` \| `indexed` \| `shortNamed`),
`isBackstageShown` [false], `isDistanceMarksShown` [false], `isSubdivisionsShown` [false].

### MusicData

| key | type | notes |
|---|---|---|
| `musicType` | string | `sandbox` (copied into app storage) \| `local` (path on device) |
| `musicAddress` | string | file name / path **on the phone** |

### StageBackground

| key | type | notes |
|---|---|---|
| `file` | StageBackgroundFile? | |
| `aspectRatio` | string | `scaleToFit` \| `scaleToFill` \| `stretchToFill` |
| `alpha` | num | opacity |
| `isBackstageSeparated` | bool | |

### StageBackgroundFile

| key | type | notes |
|---|---|---|
| `standardBackgroundValue` | string? | built-in: `basketballField`, `footballField`, `nflField`, `nflFieldSmall`, `sevenCheerleadingMats`, `eightCheerleadingMats`, `nineCheerleadingMats`, `tenCheerleadingMats`, `unknown` |
| `filePath` | string? | custom image path on device |
| `imageFileData` | string? | **base64 image bytes**, filled in on export |
| `lastSelectionTimestamp` | int? | |

### ChoreographiesBackup (root of `.aubkp`)

| key | type |
|---|---|
| `version` | num |
| `deviceName` | string |
| `choreographies` | Choreography[] |
| `crews` | `{name:string, dancers:Unit[]}[]` |
| `presets` | `{id, name, typeString, version, creationTimestamp, gridSize:SceneSize, formation:Formation}[]` |
| `backgrounds` | StageBackgroundFile[] |

## Portability

- **Custom stage backgrounds are embedded**: on export the app base64-encodes the image
  into `stageBackground.file.imageFileData`. Built-in backgrounds are only a name
  (`standardBackgroundValue`), so the visualiser has to supply its own artwork for those.
- **Music is not embedded**: `musicData` only points at a file on the phone. The visualiser
  has to let the user pick the audio track separately (matching by `musicAddress` file name).

## Animation model

Timeline = formations in order. For formation *i*: dancers move from formation *i−1*
positions to formation *i* positions over `transitionDuration`, then hold for `duration`.
Match dancers across formations by `dancer.name` (fall back to unit index when names are
blank, as in the sample).

## Not yet confirmed (needs a richer sample)

Structure is complete; these are semantics to check against a real export:
- **`y = 0` is the back (backstage) edge**, `y` grows towards the audience (front). Earlier notes read `Test-v2.arrus` the other way round (dancers 1–3 "near the audience" at `y` 0.5–1.0); the editor now follows this orientation
- ✅ **`x` spans `rows` (0–14) and `y` spans `columns` (0–8)**: the app's naming is swapped versus intuition. Dancers 8–12 at `x = 13` are on stage at the performers' right (audience's left), so **`x` grows towards stage right**. The side backstage (`sides: 2`) lies outside the stage range (x < 0 or x > rows); `back` extends `y` below 0 and `front` beyond `columns` (inferred, not yet observed)
- `settings.isSceneDirectionsInverted` probably mirrors this orientation in the app's view
- props are a premium feature, so no sample: best guesses are `width` along x, `length` along y, `height` vertical (3D only), `rotationAngle` in radians (Flutter convention), `alpha` 0–1
- prop `rotationAngle` units, `alpha` range, and `width`/`length`/`height` axes
- whether units outside the grid (backstage) use negative or > `columns`/`rows` coordinates
