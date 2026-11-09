//! The choreography editor: state, playback clock, music, keyboard,
//! toolbar, formation list and dialogs. The stage, inspector and timeline
//! live in their own modules.

use std::collections::BTreeSet;
use std::rc::Rc;

use arrus_api::{ChoreographyDoc, ChoreographyId, ChoreographySummary};
use arrus_core::timeline::{self, Easing, Phase};
use arrus_core::{Choreography, Comment, StageBackground, StageBackgroundFile};
use egui::{Align, Event, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText, ScrollArea, Sense, Ui, vec2};

use super::widgets::{self, Choice};
use super::{App, Net, Screen, inspector, live, net, stage, timeline as timeline_ui};
use crate::edit::{self, Selection};
use crate::geom::{self, P};
use crate::undo::{UNDO_LIMIT, UndoStack};
use crate::view::View;
use crate::web::{self, Audio};
use crate::{theme, util};

/// An undo step. The (possibly large) custom background image is shared, not copied.
#[derive(Clone)]
pub struct Snap {
    doc: Choreography,
    bg: Option<Rc<String>>,
    /// How many of other people's changes it already has (see `live::Live::remote_log`).
    seq: usize,
}

pub enum EdModal {
    RemoveDancers(BTreeSet<usize>),
    RemoveMusic,
    /// The name being typed.
    Rename(String),
}

pub struct Editor {
    pub summary: ChoreographySummary,
    /// Working copy, without the custom background image (kept in `bg_data`).
    pub doc: Choreography,
    /// Base64 of the custom background image.
    pub bg_data: Option<Rc<String>>,
    pub bg_tex: Option<(usize, egui::TextureHandle)>,
    bg_loading: Option<usize>,
    bg_failed: Option<usize>,
    undo: UndoStack<Snap>,
    /// Changes not stored by the server yet (saving is automatic).
    pub dirty: bool,
    /// Live co-editing: autosave, other people's changes, presence.
    pub live: live::Live,
    pub t: f64,
    /// Formation picked from the list or with previous/next: it stays current
    /// while the playhead sits at its hold start (see `edit::phase`), and the
    /// playhead follows it when edits move that start.
    pinned: Option<usize>,
    pub playing: bool,
    last_tick: Option<f64>,
    pub audio: Option<Audio>,
    pub view: Option<View>,
    pub need_fit: bool,
    /// The user zoomed or panned (so window resizes keep their view).
    pub view_moved: bool,
    pub last_stage_size: egui::Vec2,
    pub sel: Selection,
    pub drag: Option<stage::Drag>,
    pub easing: Easing,
    pub can_edit: bool,
    /// The server says this account may change this choreography.
    pub may_write: bool,
    pub compact: bool,
    pub scrubbing: bool,
    pub space_down: bool,
    pub space_pan_used: bool,
    pub modal: Option<EdModal>,
    pub music_busy: bool,
    pub show_formations: bool,
    /// Phone layout: the inspector sheet at the bottom is open.
    pub show_inspector: bool,
    /// "Next formation (outlines)": off whenever a choreography is opened and
    /// only for this screen (the file's own setting is kept as it is).
    pub show_next: bool,
    /// "Paths to next formation": the same, for where each dancer goes next.
    pub show_next_paths: bool,
    /// Touch: one-finger drags on empty space draw a selection box (instead
    /// of moving the view).
    pub multi_select: bool,
    /// A touch screen: taps add / remove dancers from the selection.
    pub touch: bool,
    /// A drag is changing `doc` directly, frame by frame.
    pub gesture: bool,
    /// egui time of this frame.
    pub now: f64,
}

pub fn open(app: &mut App, id: ChoreographyId) {
    app.opening = Some(id);
    app.screen = Screen::Editor;
    app.editor = None;
    app.net.spawn(async move {
        let r = net::get::<ChoreographyDoc>(&format!("/api/choreographies/{id}")).await;
        move |app: &mut App| {
            if app.opening != Some(id) {
                return;
            }
            app.opening = None;
            match r {
                Ok(doc) => {
                    let compact = app.compact;
                    app.editor = Some(Box::new(Editor::new(doc, compact)));
                    app.screen = Screen::Editor;
                    web::set_hash_choreography(Some(id));
                }
                Err(e) => {
                    app.screen = Screen::Library;
                    app.fail(e);
                }
            }
        }
    });
}

fn music_url(s: &ChoreographySummary) -> String {
    let v = s.music.as_ref().map_or(0, |m| m.bytes) ^ (js_sys::Date::now() as u64);
    format!("/api/choreographies/{}/music?v={v}", s.id)
}

impl Editor {
    pub fn new(d: ChoreographyDoc, compact: bool) -> Self {
        let mut doc = d.data;
        let bg_data = doc
            .stage_background
            .as_mut()
            .and_then(|b| b.file.as_mut())
            .and_then(|f| f.image_file_data.take())
            .filter(|s| !s.trim().is_empty())
            .map(Rc::new);
        if doc.formations.is_empty() {
            doc.formations.push(arrus_core::Formation::new());
        }
        let audio = d.summary.music.as_ref().and_then(|_| Audio::new(&music_url(&d.summary)));
        let live = live::Live::new(&doc, d.summary.revision, bg_data.clone());
        let may_write = d.can_write;
        Self {
            summary: d.summary,
            doc,
            bg_data,
            bg_tex: None,
            bg_loading: None,
            bg_failed: None,
            undo: UndoStack::new(UNDO_LIMIT),
            dirty: false,
            live,
            t: 0.0,
            pinned: None,
            playing: false,
            last_tick: None,
            audio,
            view: None,
            need_fit: true,
            view_moved: false,
            last_stage_size: egui::Vec2::ZERO,
            sel: Selection::None,
            drag: None,
            easing: Easing::Linear,
            can_edit: false,
            may_write,
            compact,
            scrubbing: false,
            space_down: false,
            space_pan_used: false,
            modal: None,
            music_busy: false,
            show_formations: !compact,
            show_inspector: false,
            show_next: false,
            show_next_paths: false,
            multi_select: false,
            touch: false,
            gesture: false,
            now: 0.0,
        }
    }

    // ---- editing ---------------------------------------------------------

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.live.note_change(self.now);
    }

    fn snap_now(&self) -> Snap {
        Snap { doc: self.doc.clone(), bg: self.bg_data.clone(), seq: self.live.remote_log.len() }
    }

    /// After the document changed under us (someone else's edit, a reload).
    pub(super) fn after_outside_change(&mut self) {
        self.sanitize();
        self.t = self.t.min(self.timeline_len());
    }

    /// Forget undo history (it no longer lines up with the document).
    pub(super) fn clear_history(&mut self) {
        self.undo = UndoStack::new(UNDO_LIMIT);
    }

    /// Apply an edit with an undo step (merged with the previous one when the
    /// same `key` was edited a moment ago).
    pub fn edit(&mut self, key: Option<&str>, f: impl FnOnce(&mut Choreography)) {
        let (doc, bg, seq) = (&self.doc, &self.bg_data, self.live.remote_log.len());
        self.undo.record(|| Snap { doc: doc.clone(), bg: bg.clone(), seq }, key, self.now);
        f(&mut self.doc);
        self.mark_dirty();
        self.sanitize();
    }

    /// Start a drag gesture that mutates `doc` directly over several frames
    /// (every frame of it counts as a change to save; see `live::sync`).
    pub fn begin_gesture(&mut self) {
        let (doc, bg, seq) = (&self.doc, &self.bg_data, self.live.remote_log.len());
        self.undo.record(|| Snap { doc: doc.clone(), bg: bg.clone(), seq }, None, self.now);
        self.mark_dirty();
        self.gesture = true;
    }

    pub fn end_gesture(&mut self) {
        self.undo.break_merge();
        if std::mem::take(&mut self.gesture) {
            // Save where the drag ended.
            self.mark_dirty();
        }
    }

    /// Change settings: an undoable edit for writers, a local view change for viewers.
    pub fn set_settings(&mut self, s: arrus_core::Settings, key: Option<&str>) {
        if self.can_edit {
            self.edit(key, |d| d.settings = s);
        } else {
            self.doc.settings = s;
        }
    }

    pub fn set_background(&mut self, bg: Option<StageBackground>, data: Option<Rc<String>>) {
        let (doc, old, seq) = (&self.doc, &self.bg_data, self.live.remote_log.len());
        self.undo.record(|| Snap { doc: doc.clone(), bg: old.clone(), seq }, None, self.now);
        self.doc.stage_background = bg;
        self.bg_data = data;
        self.mark_dirty();
    }

    /// Use a custom image (base64) as the stage background.
    pub fn set_custom_background(&mut self, b64: String) {
        let old = self.doc.stage_background.clone();
        let bg = StageBackground {
            file: Some(StageBackgroundFile {
                last_selection_timestamp: Some(web::now_secs()),
                ..Default::default()
            }),
            aspect_ratio: old.as_ref().map_or_else(|| "scaleToFit".into(), |b| b.aspect_ratio.clone()),
            alpha: old.as_ref().map_or(1.0, |b| b.alpha),
            is_backstage_separated: old.as_ref().is_some_and(|b| b.is_backstage_separated),
            extra: old.map(|b| b.extra).unwrap_or_default(),
        };
        self.set_background(Some(bg), Some(Rc::new(b64)));
    }

    pub(super) fn sanitize(&mut self) {
        let f = self.cur();
        let n = edit::props(&self.doc, f).len();
        self.sel.sanitize(self.doc.dancer_count(), n);
    }

    /// Go back to an undo snapshot, keeping other people's changes made since.
    fn restore(&mut self, s: Snap) {
        self.doc = self.replay_remote(s.doc, s.seq);
        self.bg_data = s.bg;
        self.mark_dirty();
        self.after_outside_change();
    }

    pub fn undo(&mut self) {
        if let Ok(s) = self.undo.undo(self.snap_now()) {
            self.restore(s);
        }
    }

    pub fn redo(&mut self) {
        if let Ok(s) = self.undo.redo(self.snap_now()) {
            self.restore(s);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Formation edits apply to (the one held or being entered).
    pub fn cur(&self) -> usize {
        edit::formation_at(&self.doc, self.t, self.pinned)
    }

    /// Where the playhead is (with the formation picked explicitly, if any).
    pub fn phase(&self) -> Option<Phase> {
        edit::phase(&self.doc, self.t, self.pinned, self.easing)
    }

    pub fn delete_selection(&mut self) {
        match self.sel.clone() {
            Selection::Dancers(s) if !s.is_empty() => self.modal = Some(EdModal::RemoveDancers(s)),
            Selection::Prop(i) => {
                let f = self.cur();
                self.edit(None, |d| {
                    edit::remove_prop(d, f, i);
                });
                self.sel = Selection::None;
            }
            _ => {}
        }
    }

    pub fn add_dancer(&mut self) {
        let f = self.cur();
        let mut idx = 0;
        self.edit(None, |d| idx = edit::add_dancer(d, f));
        self.sel = Selection::one(idx);
    }

    pub fn add_prop(&mut self, kind: arrus_core::PropType) {
        let f = self.cur();
        let mut idx = None;
        self.edit(None, |d| idx = edit::add_prop(d, f, kind));
        if let Some(i) = idx {
            self.sel = Selection::Prop(i);
        }
    }

    /// Nudge the selection by a screen direction (arrow keys).
    fn nudge(&mut self, screen_dir: (f64, f64), step: f64) {
        // Same orientation as the stage (see `stage::show`).
        let inv = !self.doc.settings.is_scene_directions_inverted;
        let s = if inv { -step } else { step };
        let d = P::new(screen_dir.0 * s, screen_dir.1 * s);
        let f = self.cur();
        let bounds = geom::drawable_bounds(&self.doc);
        match self.sel.clone() {
            Selection::Dancers(set) if !set.is_empty() => {
                self.edit(Some("nudge"), |doc| edit::nudge_dancers(doc, f, &set, d, bounds));
            }
            Selection::Prop(i) => self.edit(Some("nudge"), |doc| {
                if let Some(p) = edit::prop_mut(doc, f, i) {
                    p.position = bounds.clamp(geom::add(p.position, d));
                }
            }),
            _ => {}
        }
    }

    // ---- playback --------------------------------------------------------

    pub fn total(&self) -> f64 {
        timeline::total_duration(&self.doc)
    }

    /// Choreography length, extended to the music's length when longer.
    pub fn timeline_len(&self) -> f64 {
        let total = self.total();
        match self.audio.as_ref().and_then(Audio::duration) {
            Some(d) => total.max(d),
            None => total,
        }
    }

    fn tick(&mut self) {
        let now = self.now;
        if self.playing {
            let len = self.timeline_len();
            match &self.audio {
                Some(a) => {
                    self.t = a.time();
                    if a.paused() {
                        self.playing = false;
                    }
                }
                None => self.t += (now - self.last_tick.unwrap_or(now)).clamp(0.0, 0.25),
            }
            if self.t >= len {
                self.t = len;
                self.pause();
            }
        }
        self.last_tick = Some(now);
    }

    pub fn play(&mut self) {
        let len = self.timeline_len();
        if len <= 0.0 {
            return;
        }
        if self.t >= len - 0.01 {
            self.t = 0.0;
        }
        self.drag = None;
        self.pinned = None;
        if let Some(a) = &self.audio {
            a.seek(self.t);
            a.play();
        }
        self.playing = true;
        self.last_tick = Some(self.now);
    }

    pub fn pause(&mut self) {
        if let Some(a) = &self.audio {
            a.pause();
            self.t = a.time().min(self.timeline_len());
        }
        self.playing = false;
    }

    pub fn toggle_play(&mut self) {
        if self.playing { self.pause() } else { self.play() }
    }

    pub fn seek(&mut self, t: f64) {
        self.pinned = None;
        self.t = t.clamp(0.0, self.timeline_len().max(0.0));
        if let Some(a) = &self.audio {
            a.seek(self.t);
        }
    }

    pub fn go_to_formation(&mut self, i: usize) {
        self.seek(timeline::formation_time(&self.doc, i));
        self.pinned = Some(i);
    }

    /// Keep the playhead on the picked formation when edits (a longer
    /// transition into it, someone else's change) move its hold start.
    fn follow_pinned(&mut self) {
        match self.pinned {
            Some(p) if p >= self.doc.formations.len() => self.pinned = None,
            Some(p) if timeline::formation_time(&self.doc, p) != self.t => self.go_to_formation(p),
            _ => {}
        }
    }

    pub fn prev_formation(&mut self) {
        let spans = timeline::spans(&self.doc);
        let f = self.cur();
        let Some(s) = spans.get(f) else { return };
        if self.t > s.hold_start + 0.25 || f == 0 {
            self.go_to_formation(f);
        } else {
            self.go_to_formation(f - 1);
        }
    }

    pub fn next_formation(&mut self) {
        let f = self.cur();
        if f + 1 < self.doc.formations.len() {
            self.go_to_formation(f + 1);
        } else {
            let end = self.total();
            self.seek(end);
        }
    }

    pub(super) fn reload_audio(&mut self) {
        self.pause();
        self.audio = self.summary.music.as_ref().and_then(|_| Audio::new(&music_url(&self.summary)));
    }

    // ---- background texture ----------------------------------------------

    fn update_background(&mut self, net: &Net) {
        let Some(data) = &self.bg_data else {
            self.bg_tex = None;
            return;
        };
        let key = Rc::as_ptr(data) as usize;
        if self.bg_tex.as_ref().map(|t| t.0) == Some(key) || self.bg_loading == Some(key) || self.bg_failed == Some(key) {
            return;
        }
        self.bg_loading = Some(key);
        let url = format!("data:{};base64,{}", web::sniff_image_mime(data), data.trim());
        let net = net.clone();
        let id = self.summary.id;
        web::decode_image(&url, move |r| {
            net.post(move |app: &mut App| {
                let ctx = app.net.ctx().clone();
                let Some(ed) = app.editor.as_mut().filter(|e| e.summary.id == id) else { return };
                if ed.bg_loading == Some(key) {
                    ed.bg_loading = None;
                }
                match r {
                    Ok(img) => {
                        let tex = ctx.load_texture("stage_background", img, egui::TextureOptions::LINEAR);
                        ed.bg_tex = Some((key, tex));
                    }
                    Err(e) => {
                        ed.bg_failed = Some(key);
                        app.toast_error(e);
                    }
                }
            });
        });
    }
}

// ---- server actions -------------------------------------------------------

fn editor_for(app: &mut App, id: ChoreographyId) -> Option<&mut Editor> {
    app.editor.as_deref_mut().filter(|e| e.summary.id == id)
}

fn upload_music(ed: &mut Editor, app: &mut App) {
    let id = ed.summary.id;
    let net = app.net.clone();
    web::pick_files("audio/*,.mp3,.m4a,.aac,.wav,.ogg,.flac", false, move |files| {
        let Some(file) = files.into_iter().next() else { return };
        net.post(move |app: &mut App| {
            if file.size() > arrus_api::MAX_MUSIC_BYTES as f64 {
                app.toast_error(format!(
                    "That file is too big (limit {}).",
                    util::fmt_bytes(arrus_api::MAX_MUSIC_BYTES)
                ));
                return;
            }
            let mut ct = file.type_();
            if !ct.starts_with("audio/") {
                ct = match util::extension(&file.name()).as_str() {
                    "mp3" => "audio/mpeg",
                    "m4a" | "aac" => "audio/mp4",
                    "wav" => "audio/wav",
                    "ogg" => "audio/ogg",
                    "flac" => "audio/flac",
                    _ => "audio/mpeg",
                }
                .into();
            }
            let Some(ed) = editor_for(app, id) else { return };
            ed.music_busy = true;
            app.net.spawn(async move {
                let r = net::upload::<ChoreographySummary>("PUT", &format!("/api/choreographies/{id}/music"), file, ct).await;
                move |app: &mut App| on_music(app, id, r, "Music uploaded")
            });
        });
    });
}

fn remove_music(ed: &mut Editor, app: &mut App) {
    let id = ed.summary.id;
    ed.music_busy = true;
    app.net.spawn(async move {
        let r = net::delete_json::<ChoreographySummary>(&format!("/api/choreographies/{id}/music")).await;
        move |app: &mut App| on_music(app, id, r, "Music removed")
    });
}

fn on_music(app: &mut App, id: ChoreographyId, r: net::ApiResult<ChoreographySummary>, done: &str) {
    let Some(ed) = editor_for(app, id) else { return };
    ed.music_busy = false;
    match r {
        Ok(s) => {
            // Music does not change the document revision we are editing against.
            let rev = ed.summary.revision;
            ed.summary = s;
            ed.summary.revision = rev;
            ed.reload_audio();
            app.toast(done);
            app.refresh_storage();
        }
        Err(e) => app.fail(e),
    }
}

pub fn pick_background(ed: &Editor, app: &App) {
    let id = ed.summary.id;
    let net = app.net.clone();
    web::pick_files("image/png,image/jpeg", false, move |files| {
        let Some(file) = files.into_iter().next() else { return };
        if file.size() > 8.0 * 1024.0 * 1024.0 {
            net.post(|app: &mut App| app.toast_error("Please choose an image under 8 MB."));
            return;
        }
        web::read_data_url(&file, move |r| {
            net.post(move |app: &mut App| match r {
                Ok(url) => {
                    let b64 = url.split_once(',').map(|(_, b)| b.to_owned()).unwrap_or_default();
                    if let Some(ed) = editor_for(app, id) {
                        ed.set_custom_background(b64);
                    }
                }
                Err(e) => app.toast_error(e),
            });
        });
    });
}

// ---- frame ----------------------------------------------------------------

pub fn show(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    ed.now = ui.input(|i| i.time);
    ed.can_edit = ed.may_write;
    ed.touch = app.touch;
    if ed.compact != app.compact {
        ed.compact = app.compact;
        ed.need_fit = true;
    }
    ed.tick();
    live::sync(ed, app);
    if ed.playing {
        ui.ctx().request_repaint();
    }
    keyboard(ed, app, ui);
    if !ed.playing && !ed.scrubbing {
        ed.follow_pinned();
    }
    if ed.can_edit && !ed.playing && !ed.scrubbing
        && let Some(to) = edit::entered_formation(&ed.doc, ed.t, ed.pinned)
    {
        ed.go_to_formation(to);
    }
    ed.update_background(&app.net);

    toolbar(ed, app, ui);
    if ed.compact {
        tool_row(ed, app, ui);
    }
    match timeline_ui::show(ed, ui) {
        Some(timeline_ui::MusicAction::Upload) => upload_music(ed, app),
        Some(timeline_ui::MusicAction::Remove) => ed.modal = Some(EdModal::RemoveMusic),
        None => {}
    }
    formations_panel(ed, ui);
    if !ed.compact {
        inspector::show(ed, app, ui);
    } else if ed.can_edit {
        let mut open = ed.show_inspector;
        inspector::show_sheet(ed, app, ui, &mut open);
        ed.show_inspector = open;
    }
    egui::CentralPanel::default().frame(Frame::new().fill(theme::BG)).show(ui, |ui| stage::show(ed, ui));
    modals(ed, app, ui.ctx());
}

fn keyboard(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let save_key = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    if ctx.input_mut(|i| i.consume_shortcut(&save_key)) && ed.can_edit {
        // Saving is automatic; send what is pending right away.
        ed.collect();
        app.toast("Changes are saved automatically");
    }
    if ctx.egui_wants_keyboard_input() || ed.modal.is_some() {
        return;
    }
    if ed.can_edit {
        let redo1 = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo2 = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        if ctx.input_mut(|i| i.consume_shortcut(&redo1) || i.consume_shortcut(&redo2)) {
            ed.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
            ed.undo();
        }
    }
    // Space: play/pause on release, unless it was held to pan.
    let (pressed, released) = ctx.input_mut(|i| {
        let (mut p, mut r) = (false, false);
        i.events.retain(|e| match e {
            Event::Key { key: Key::Space, pressed, repeat, .. } => {
                if *pressed && !*repeat {
                    p = true;
                }
                if !*pressed {
                    r = true;
                }
                false
            }
            _ => true,
        });
        (p, r)
    });
    if pressed {
        ed.space_down = true;
        ed.space_pan_used = false;
    }
    if released && ed.space_down {
        ed.space_down = false;
        if !ed.space_pan_used {
            ed.toggle_play();
        }
    }

    let has_sel = !matches!(ed.sel, Selection::None);
    let arrows = [(Key::ArrowLeft, (-1.0, 0.0)), (Key::ArrowRight, (1.0, 0.0)), (Key::ArrowUp, (0.0, -1.0)), (Key::ArrowDown, (0.0, 1.0))];
    for (key, dir) in arrows {
        if ed.can_edit && has_sel && !ed.playing {
            if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, key)) {
                ed.nudge(dir, 1.0);
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                ed.nudge(dir, geom::SNAP_STEP);
            }
        } else if dir.1 == 0.0 && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            if dir.0 < 0.0 { ed.prev_formation() } else { ed.next_formation() }
        }
    }
    if ed.can_edit
        && !ed.playing
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace))
    {
        ed.delete_selection();
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        ed.sel = Selection::None;
        ed.drag = None;
    }
}

fn toolbar(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    egui::Panel::top("editor_bar")
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(12, 7)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let back = if ed.compact { "←" } else { "← Library" };
                if ui.button(back).on_hover_text("Back to the library").clicked() {
                    if ed.dirty {
                        app.leave_to = Some(Screen::Library);
                    } else {
                        app.close_editor();
                    }
                }
                if ed.compact {
                    // Phone: back, name, and undo / redo / save. The tools
                    // live in the row at the bottom (see `tool_row`).
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !ed.can_edit {
                            widgets::badge(ui, "View only", theme::TEXT_WEAK);
                        }
                        if ed.can_edit {
                            if ui.add_enabled(ed.can_redo(), egui::Button::new("↷")).clicked() {
                                ed.redo();
                            }
                            if ui.add_enabled(ed.can_undo(), egui::Button::new("↶")).clicked() {
                                ed.undo();
                            }
                        }
                        widgets::presence(ui, &ed.live.others(), 3);
                        save_status(ui, ed, true);
                        name_label(ui, ed, true);
                    });
                    return;
                }
                ui.add_space(6.0);
                name_label(ui, ed, false);
                if !ed.can_edit {
                    widgets::badge(ui, "View only", theme::TEXT_WEAK)
                        .on_hover_text("You can view this choreography; ask your admin for write access to edit.");
                } else {
                    save_status(ui, ed, false);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ed.can_edit {
                        if ui.add_enabled(ed.can_redo(), egui::Button::new("↷ Redo")).on_hover_text("Shift+Ctrl/Cmd+Z").clicked() {
                            ed.redo();
                        }
                        if ui
                            .add_enabled(ed.can_undo(), egui::Button::new("↶ Undo"))
                            .on_hover_text("Ctrl/Cmd+Z · undoes your own changes only")
                            .clicked()
                        {
                            ed.undo();
                        }
                        ui.add_space(6.0);
                    }
                    // Who else has it open, left of Undo (like a shared document).
                    widgets::presence(ui, &ed.live.others(), 6);
                    if ui.button("Export PDF").on_hover_text("One page per formation, to share with people who don't use arus").clicked() {
                        export_pdf(ed, app);
                    }
                });
            });
        });
}

/// The choreography's name in the top bar; writers click it to rename.
fn name_label(ui: &mut Ui, ed: &mut Editor, truncate: bool) {
    let mut label = egui::Label::new(RichText::new(&ed.doc.name).strong().size(16.0));
    if truncate {
        label = label.truncate();
    }
    if !ed.can_edit {
        ui.add(label);
        return;
    }
    let r = ui.add(label.sense(Sense::click())).on_hover_cursor(egui::CursorIcon::Text).on_hover_text("Rename");
    if r.clicked() {
        ed.modal = Some(EdModal::Rename(ed.doc.name.clone()));
    }
}

/// Download the PDF (of what the server has: wait for pending changes first).
fn export_pdf(ed: &mut Editor, app: &mut App) {
    if ed.live.saving() {
        ed.collect();
        app.toast("Saving your last change first, try again in a moment.");
        return;
    }
    web::download(&format!("/api/choreographies/{}/pdf", ed.summary.id));
}

/// "Saving…" / "All changes saved" / "Offline". Compact: a coloured dot only.
fn save_status(ui: &mut Ui, ed: &Editor, compact: bool) {
    if !ed.can_edit {
        return;
    }
    let (text, color, tip) = if ed.live.offline {
        ("Offline – changes will be saved when back online", theme::WARN, "Not connected; your changes are kept and sent automatically.")
    } else if ed.live.saving() {
        ("Saving…", theme::TEXT_WEAK, "Your changes are being saved.")
    } else {
        ("All changes saved", theme::TEXT_FAINT, "Every change is saved automatically.")
    };
    if compact {
        let (r, resp) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
        let dot = if ed.live.offline { theme::WARN } else if ed.live.saving() { theme::TEXT_WEAK } else { theme::OK };
        ui.painter().circle_filled(r.center(), 4.0, dot);
        resp.on_hover_text(text);
        return;
    }
    let tip = format!("{tip}\nRevision {} · last change by {}", ed.summary.revision, ed.summary.updated_by);
    ui.label(RichText::new(text).color(color)).on_hover_text(tip);
}

/// Phone layout: a resizable sheet above the timeline (formations or details).
pub fn sheet(id: &str, ui: &Ui) -> egui::Panel {
    let h = ui.ctx().content_rect().height();
    egui::Panel::bottom(id.to_owned())
        .resizable(true)
        .default_size(h * 0.4)
        .size_range(140.0..=(h * 0.7).max(160.0))
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::same(12)))
}

/// Phone layout: every tool in one row at the bottom of the screen, in its
/// own panel (not over the stage) so swiping it scrolls the row instead of
/// moving the stage.
fn tool_row(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    egui::Panel::bottom("tools")
        .frame(Frame::new().fill(theme::SURFACE).inner_margin(Margin::symmetric(8, 8)))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 44.0;
            ui.spacing_mut().button_padding = vec2(14.0, 9.0);
            ScrollArea::horizontal().id_salt("tools_scroll").show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::selectable(ed.show_formations, "Formations")).clicked() {
                        ed.show_formations = !ed.show_formations;
                        // One sheet at a time: a phone has no room for both.
                        ed.show_inspector &= !ed.show_formations;
                    }
                    if ed.can_edit {
                        let n = match &ed.sel {
                            Selection::Dancers(s) => s.len(),
                            Selection::Prop(_) => 1,
                            Selection::None => 0,
                        };
                        let label = if n > 0 { format!("Details ({n})") } else { "Details".to_owned() };
                        if ui.add(egui::Button::selectable(ed.show_inspector, label)).clicked() {
                            ed.show_inspector = !ed.show_inspector;
                            ed.show_formations &= !ed.show_inspector;
                        }
                        ui.separator();
                        if ui.button("+ Dancer").clicked() {
                            ed.pause();
                            ed.add_dancer();
                        }
                        ui.menu_button("+ Prop", |ui| {
                            for &k in arrus_core::PropType::ALL {
                                if ui.button(geom::prop_label(k)).clicked() {
                                    ed.pause();
                                    ed.add_prop(k);
                                    ui.close();
                                }
                            }
                        });
                        ui.toggle_value(&mut ed.multi_select, "Box select")
                            .on_hover_text("Drag on empty space to select several dancers with a box");
                    }
                    ui.separator();
                    if ui.button("Fit").clicked() {
                        ed.need_fit = true;
                    }
                    let mut inv = ed.doc.settings.is_scene_directions_inverted;
                    if ui.toggle_value(&mut inv, "Audience view").changed() {
                        let mut s = ed.doc.settings.clone();
                        s.is_scene_directions_inverted = inv;
                        ed.set_settings(s, None);
                    }
                    if ui.button("PDF").clicked() {
                        export_pdf(ed, app);
                    }
                    ui.separator();
                    widgets::fullscreen_button(ui);
                });
            });
        });
}

fn formations_panel(ed: &mut Editor, ui: &mut Ui) {
    if ed.compact {
        let mut open = ed.show_formations;
        sheet("formations_sheet", ui).show_collapsible(ui, &mut open, |ui| formations_ui(ed, ui));
        ed.show_formations = open;
    } else {
        egui::Panel::left("formations")
            .resizable(true)
            .default_size(250.0)
            .size_range(200.0..=420.0)
            .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::same(12)))
            .show(ui, |ui| formations_ui(ed, ui));
    }
}

fn formations_ui(ed: &mut Editor, ui: &mut Ui) {
    let cur = ed.cur();
    let n = ed.doc.formations.len();
    ui.horizontal(|ui| {
        widgets::section(ui, "Formations");
        ui.label(RichText::new(n.to_string()).small().color(theme::TEXT_FAINT));
    });
    let row_h = if ed.compact { 44.0 } else { 32.0 };
    let list_h = (ui.available_height() * if ed.can_edit { 0.45 } else { 0.6 }).max(120.0);
    let mut go = None;
    // Who else is on which formation (a dot in their colour).
    let others: Vec<(usize, egui::Color32, String)> = ed
        .live
        .others()
        .iter()
        .filter_map(|v| Some((v.formation?, theme::person_color(&v.username), v.username.clone())))
        .collect();
    ScrollArea::vertical().id_salt("formation_list").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
        for i in 0..n {
            let f = &ed.doc.formations[i];
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            let p = ui.painter();
            let selected = i == cur;
            let fill = if selected {
                theme::ACCENT_DIM
            } else if resp.hovered() {
                theme::HOVER
            } else {
                theme::SURFACE
            };
            p.rect_filled(rect.shrink2(vec2(0.0, 1.5)), 6.0, fill);
            let num_c = rect.left_center() + vec2(18.0, 0.0);
            p.circle_filled(num_c, 11.0, if selected { theme::ACCENT } else { theme::RAISED });
            p.text(num_c, egui::Align2::CENTER_CENTER, (i + 1).to_string(), egui::FontId::proportional(12.0), theme::TEXT);
            let name = f.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("");
            let dur = util::fmt_secs(f.duration);
            let right = p.text(
                rect.right_center() - vec2(10.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                &dur,
                egui::FontId::proportional(12.5),
                theme::TEXT_WEAK,
            );
            if !name.is_empty() {
                let clip = egui::Rect::from_min_max(rect.min + vec2(36.0, 0.0), egui::pos2(right.left() - 6.0, rect.max.y));
                ui.painter().with_clip_rect(clip).text(
                    clip.left_center(),
                    egui::Align2::LEFT_CENTER,
                    name,
                    egui::FontId::proportional(14.0),
                    theme::TEXT,
                );
            }
            if !f.comments.is_empty() {
                p.circle_filled(rect.right_top() + vec2(-6.0, 7.0), 3.0, theme::SELECT);
            }
            let here: Vec<&(usize, egui::Color32, String)> = others.iter().filter(|o| o.0 == i).collect();
            for (k, (_, color, _)) in here.iter().enumerate() {
                let c = egui::pos2(right.left() - 10.0 - 12.0 * k as f32, rect.center().y);
                p.circle_filled(c, 5.0, *color);
                p.circle_stroke(c, 5.0, egui::Stroke::new(1.0, theme::PANEL));
            }
            let resp = if here.is_empty() {
                resp
            } else {
                let names: Vec<&str> = here.iter().map(|o| o.2.as_str()).collect();
                resp.on_hover_text(format!("Here: {}", names.join(", ")))
            };
            let resp = if f.comments.is_empty() {
                resp
            } else {
                resp.on_hover_text(f.comments.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n"))
            };
            if resp.clicked() {
                go = Some(i);
            }
        }
    });
    if let Some(i) = go {
        ed.pause();
        ed.go_to_formation(i);
    }

    if ed.can_edit {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ Add").on_hover_text("New formation after this one, with the same positions").clicked() {
                let mut at = 0;
                ed.edit(None, |d| at = edit::add_formation_after(d, cur));
                ed.go_to_formation(at);
            }
            if ui.button("Duplicate").clicked() {
                let mut at = None;
                ed.edit(None, |d| at = edit::duplicate_formation(d, cur));
                if let Some(at) = at {
                    ed.go_to_formation(at);
                }
            }
            if ui.add_enabled(n > 1, egui::Button::new("Delete")).on_disabled_hover_text("A choreography keeps at least one formation").clicked() {
                let mut next = None;
                ed.edit(None, |d| next = edit::delete_formation(d, cur));
                if let Some(next) = next {
                    ed.go_to_formation(next);
                }
            }
            if ui.add_enabled(cur > 0, egui::Button::new("↑")).on_hover_text("Move earlier").clicked() {
                let mut to = None;
                ed.edit(None, |d| to = edit::move_formation(d, cur, true));
                if let Some(to) = to {
                    ed.go_to_formation(to);
                }
            }
            if ui.add_enabled(cur + 1 < n, egui::Button::new("↓")).on_hover_text("Move later").clicked() {
                let mut to = None;
                ed.edit(None, |d| to = edit::move_formation(d, cur, false));
                if let Some(to) = to {
                    ed.go_to_formation(to);
                }
            }
        });
    }

    ui.add_space(8.0);
    ui.separator();
    let cur = ed.cur();
    let Some(f) = ed.doc.formations.get(cur).cloned() else { return };
    ScrollArea::vertical().id_salt("formation_details").auto_shrink([false, true]).show(ui, |ui| {
        widgets::section(ui, &format!("Formation {}", cur + 1));
        if ed.can_edit {
            let mut name = f.name.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut name).hint_text("Name (optional)").desired_width(f32::INFINITY)).changed() {
                ed.edit(Some(&format!("fname{cur}")), |d| {
                    d.formations[cur].name = (!name.trim().is_empty()).then_some(name);
                });
            }
            ui.add_space(4.0);
            egui::Grid::new("formation_timing").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Hold");
                let mut dur = f.duration;
                if ui.add(egui::DragValue::new(&mut dur).speed(0.1).range(0.0..=3600.0).max_decimals(2).suffix(" s")).changed() {
                    ed.edit(Some(&format!("fdur{cur}")), |d| d.formations[cur].duration = dur);
                }
                ui.end_row();
                ui.label("Transition in");
                let mut tr = f.transition_duration;
                let r = ui.add_enabled(
                    cur > 0,
                    egui::DragValue::new(&mut tr).speed(0.1).range(0.0..=600.0).max_decimals(2).suffix(" s"),
                );
                if r.on_disabled_hover_text("The first formation has no transition").changed() {
                    ed.edit(Some(&format!("ftr{cur}")), |d| d.formations[cur].transition_duration = tr);
                }
                ui.end_row();
            });
            widgets::section(ui, "Comments");
            let mut remove = None;
            for (k, c) in f.comments.iter().enumerate() {
                // Right-to-left: the button first, then the box takes exactly
                // what is left. A fixed "width - 30" overflowed by a few
                // pixels, and the panel grew to fit it every frame, so it
                // could no longer be resized.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.small_button("✕").on_hover_text("Delete comment").clicked() {
                        remove = Some(k);
                    }
                    let mut text = c.text.clone();
                    let w = ui.available_width();
                    if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).desired_width(w).align(egui::Align2::LEFT_TOP)).changed() {
                        ed.edit(Some(&format!("fcom{cur}-{k}")), |d| {
                            if let Some(c) = d.formations[cur].comments.get_mut(k) {
                                c.text = text;
                            }
                        });
                    }
                });
            }
            if let Some(k) = remove {
                ed.edit(None, |d| {
                    d.formations[cur].comments.remove(k);
                });
            }
            if ui.button("+ Add comment").clicked() {
                ed.edit(None, |d| d.formations[cur].comments.push(Comment { text: String::new(), extra: Default::default() }));
            }
        } else {
            if let Some(n) = f.name.as_deref().filter(|s| !s.trim().is_empty()) {
                ui.label(RichText::new(n).strong());
            }
            ui.label(
                RichText::new(format!(
                    "Hold {}{}",
                    util::fmt_secs(f.duration),
                    if cur > 0 { format!(" · transition {}", util::fmt_secs(f.transition_duration)) } else { String::new() }
                ))
                .color(theme::TEXT_WEAK),
            );
            for c in f.comments.iter().filter(|c| !c.text.trim().is_empty()) {
                ui.add_space(4.0);
                widgets::card().inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(&c.text);
                });
            }
        }
    });
}

fn modals(ed: &mut Editor, app: &mut App, ctx: &egui::Context) {
    if let Some(EdModal::Rename(name)) = &mut ed.modal {
        if let Some(r) = widgets::rename_dialog(ctx, "rename_editor", name, false) {
            ed.modal = None;
            if let Some(new) = r {
                // An ordinary edit: undoable, saved and shown to the others live.
                ed.edit(None, |d| d.name = new);
            }
        }
        return;
    }
    let Some(m) = ed.modal.take() else { return };
    let keep = match &m {
        // Handled above.
        EdModal::Rename(_) => false,
        EdModal::RemoveDancers(set) => {
            let n = set.len();
            let title = if n == 1 { "Remove this dancer?".to_owned() } else { format!("Remove {n} dancers?") };
            match widgets::dialog(ctx, "rm_dancers", &title, "Remove", true, true, |ui| {
                ui.label("They are removed from every formation of this choreography (you can undo).");
            }) {
                Some(Choice::Confirm) => {
                    ed.edit(None, |d| edit::remove_dancers(d, set));
                    ed.sel = Selection::None;
                    false
                }
                Some(Choice::Cancel) => false,
                None => true,
            }
        }
        EdModal::RemoveMusic => {
            match widgets::dialog(ctx, "rm_music", "Remove the music?", "Remove", true, true, |ui| {
                ui.label("The track is deleted from the server for everyone in the team.");
            }) {
                Some(Choice::Confirm) => {
                    remove_music(ed, app);
                    false
                }
                Some(Choice::Cancel) => false,
                None => true,
            }
        }
    };
    if keep && ed.modal.is_none() {
        ed.modal = Some(m);
    }
}
