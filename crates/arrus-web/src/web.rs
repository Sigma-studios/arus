//! Browser glue (wasm only): fetch, file pickers, downloads, audio, image
//! decoding, navigation, `beforeunload`, media queries, clock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use arrus_api::CSRF_HEADER;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    BeforeUnloadEvent, CanvasRenderingContext2d, File, FileReader, Headers, HtmlAnchorElement, HtmlAudioElement,
    HtmlCanvasElement, HtmlImageElement, HtmlInputElement, Request, RequestCredentials, RequestInit, Response,
};

fn window() -> web_sys::Window {
    web_sys::window().expect("no window")
}

fn document() -> web_sys::Document {
    window().document().expect("no document")
}

pub fn js_err(e: JsValue) -> String {
    e.as_string()
        .or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())))
        .unwrap_or_else(|| format!("{e:?}"))
}

// ---------------------------------------------------------------------------
// HTTP

pub enum Body {
    Empty,
    Json(String),
    /// A browser file/blob sent as-is (never copied into wasm memory).
    Blob(web_sys::Blob, String),
}

pub struct Resp {
    pub status: u16,
    pub text: String,
}

/// One same-origin request (the session cookie goes along automatically).
/// Mutating methods carry the CSRF header.
pub async fn fetch(method: &str, url: &str, body: Body, headers: &[(&str, String)]) -> Result<Resp, String> {
    let init = RequestInit::new();
    init.set_method(method);
    init.set_credentials(RequestCredentials::SameOrigin);
    let h = Headers::new().map_err(js_err)?;
    if method != "GET" {
        h.set(CSRF_HEADER, "1").map_err(js_err)?;
    }
    match body {
        Body::Empty => {}
        Body::Json(s) => {
            h.set("Content-Type", "application/json").map_err(js_err)?;
            init.set_body(&JsValue::from_str(&s));
        }
        Body::Blob(b, content_type) => {
            if !content_type.is_empty() {
                h.set("Content-Type", &content_type).map_err(js_err)?;
            }
            init.set_body(&b);
        }
    }
    for (k, v) in headers {
        h.set(k, v).map_err(js_err)?;
    }
    init.set_headers(&h);
    let req = Request::new_with_str_and_init(url, &init).map_err(js_err)?;
    let resp: Response = JsFuture::from(window().fetch_with_request(&req))
        .await
        .map_err(|e| format!("network error: {}", js_err(e)))?
        .dyn_into()
        .map_err(js_err)?;
    let text = JsFuture::from(resp.text().map_err(js_err)?).await.map_err(js_err)?.as_string().unwrap_or_default();
    Ok(Resp { status: resp.status(), text })
}

// ---------------------------------------------------------------------------
// Navigation, unload guard, environment

pub fn navigate(url: &str) {
    let _ = window().location().set_href(url);
}

/// The choreography named in the URL fragment (`#c=42`), if any.
pub fn hash_choreography() -> Option<i64> {
    let h = window().location().hash().ok()?;
    h.strip_prefix("#c=")?.parse().ok()
}

/// The screen named in the URL fragment (`#admin`, `#accounts`, `#team`), if any.
pub fn hash_screen() -> Option<String> {
    let h = window().location().hash().ok()?;
    let name = h.strip_prefix('#')?;
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase())).then(|| name.to_owned())
}

/// The team named in the URL fragment (`#team=7`, after an invite link).
pub fn hash_team() -> Option<i64> {
    let h = window().location().hash().ok()?;
    h.strip_prefix("#team=")?.parse().ok()
}

/// Reflect a screen in the URL fragment (`None`: the library).
pub fn set_hash_screen(name: Option<&str>) {
    match name {
        Some(n) => set_hash(format!("#{n}")),
        None if hash_screen().is_some() || hash_team().is_some() => set_hash(String::new()),
        None => {}
    }
}

/// The page's origin (`https://host`), for share links.
pub fn origin() -> String {
    window().location().origin().unwrap_or_default()
}

/// Reflect the open choreography in the URL fragment (so reload / links work).
pub fn set_hash_choreography(id: Option<i64>) {
    set_hash(id.map(|id| format!("#c={id}")).unwrap_or_default());
}

fn set_hash(h: String) {
    if let Ok(history) = window().history() {
        let url = if h.is_empty() { window().location().pathname().unwrap_or_else(|_| "/".into()) } else { h };
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&url));
    }
}

thread_local! {
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    static UNLOAD_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static PICKER: RefCell<Option<HtmlInputElement>> = const { RefCell::new(None) };
}

/// Tell the `beforeunload` guard whether there are unsaved changes.
pub fn set_unsaved(dirty: bool) {
    DIRTY.with(|d| d.set(dirty));
    if dirty && !UNLOAD_INSTALLED.with(|i| i.replace(true)) {
        let handler = Closure::<dyn Fn(BeforeUnloadEvent)>::new(|e: BeforeUnloadEvent| {
            if DIRTY.with(Cell::get) {
                e.prevent_default();
                e.set_return_value("You have unsaved changes.");
            }
        });
        window().set_onbeforeunload(Some(handler.as_ref().unchecked_ref()));
        handler.forget();
    }
}

/// Phones and tablets: a coarse primary pointer.
pub fn coarse_pointer() -> bool {
    window().match_media("(pointer: coarse)").ok().flatten().is_some_and(|m| m.matches())
}

/// Call the first of `names` that `target` has as a method; swallow the
/// promise rejection a refused fullscreen request returns.
fn call_first(target: &JsValue, names: &[&str]) {
    for name in names {
        let Ok(f) = js_sys::Reflect::get(target, &JsValue::from_str(name)) else { continue };
        let Some(f) = f.dyn_ref::<js_sys::Function>() else { continue };
        if let Ok(r) = f.call0(target)
            && let Ok(p) = r.dyn_into::<js_sys::Promise>()
        {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(p).await;
            });
        }
        return;
    }
}

fn doc_flag(names: &[&str]) -> bool {
    let d: JsValue = document().into();
    names.iter().any(|n| js_sys::Reflect::get(&d, &JsValue::from_str(n)).is_ok_and(|v| v.is_truthy()))
}

/// Whether the page may go fullscreen (Android, desktop; not iPhone Safari).
pub fn fullscreen_supported() -> bool {
    doc_flag(&["fullscreenEnabled", "webkitFullscreenEnabled"])
}

pub fn is_fullscreen() -> bool {
    doc_flag(&["fullscreenElement", "webkitFullscreenElement"])
}

/// Enter or leave fullscreen. Must run from a tap/click.
pub fn toggle_fullscreen() {
    if is_fullscreen() {
        call_first(&document().into(), &["exitFullscreen", "webkitExitFullscreen"]);
    } else if let Some(el) = document().document_element() {
        call_first(&el.into(), &["requestFullscreen", "webkitRequestFullscreen"]);
    }
}

/// Running inside the arus Windows app (Edge WebView2 exposes `chrome.webview`).
pub fn in_windows_app() -> bool {
    js_sys::Reflect::get(&window().into(), &JsValue::from_str("chrome"))
        .ok()
        .filter(|c| c.is_object())
        .and_then(|c| js_sys::Reflect::get(&c, &JsValue::from_str("webview")).ok())
        .is_some_and(|w| w.is_object())
}

/// Opened from the home screen as an app (no browser bars to hide).
pub fn standalone() -> bool {
    let media = window().match_media("(display-mode: standalone)").ok().flatten().is_some_and(|m| m.matches());
    let ios = js_sys::Reflect::get(&window().navigator().into(), &JsValue::from_str("standalone")).is_ok_and(|v| v.is_truthy());
    media || ios
}

pub fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Local date and time of a Unix timestamp, e.g. "03/11/2026, 14:05".
pub fn local_datetime(unix: i64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(unix as f64 * 1000.0));
    let opts = js_sys::Object::new();
    for (k, v) in [("year", "numeric"), ("month", "short"), ("day", "numeric"), ("hour", "2-digit"), ("minute", "2-digit")] {
        let _ = js_sys::Reflect::set(&opts, &k.into(), &v.into());
    }
    String::from(d.to_locale_string("default", &opts))
}

/// A random v4 UUID (the server replaces it anyway).
/// A server-sent event stream. The browser reconnects by itself after a
/// network error; closed when dropped.
pub struct EventStream {
    es: web_sys::EventSource,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

impl EventStream {
    /// `on_message` gets each event's data; `on_error` whether the browser
    /// gave up (it does after an HTTP error, e.g. when logged out).
    pub fn open(url: &str, mut on_message: impl FnMut(String) + 'static, mut on_error: impl FnMut(bool) + 'static) -> Option<Self> {
        let es = web_sys::EventSource::new(url).ok()?;
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Some(s) = e.data().as_string() {
                on_message(s);
            }
        });
        let es2 = es.clone();
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            on_error(es2.ready_state() == web_sys::EventSource::CLOSED);
        });
        es.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        es.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Some(Self { es, _on_message: on_message, _on_error: on_error })
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        self.es.set_onmessage(None);
        self.es.set_onerror(None);
        self.es.close();
    }
}

/// A WebSocket to this site (`path` from the root). Closed when dropped.
pub struct Socket {
    ws: web_sys::WebSocket,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Socket {
    /// `on_open` runs once connected, `on_message` with each text message,
    /// `on_close` once when it ends or fails (whether it had opened).
    pub fn open(
        path: &str,
        mut on_open: impl FnMut() + 'static,
        mut on_message: impl FnMut(String) + 'static,
        mut on_close: impl FnMut(bool) + 'static,
    ) -> Option<Self> {
        let base = origin();
        let url = match base.strip_prefix("https://") {
            Some(rest) => format!("wss://{rest}{path}"),
            None => format!("ws://{}{path}", base.strip_prefix("http://").unwrap_or(&base)),
        };
        let ws = web_sys::WebSocket::new(&url).ok()?;
        let opened = std::rc::Rc::new(std::cell::Cell::new(false));
        let o2 = opened.clone();
        let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            o2.set(true);
            on_open();
        });
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Some(s) = e.data().as_string() {
                on_message(s);
            }
        });
        let on_close = Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_: web_sys::CloseEvent| {
            on_close(opened.get());
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        Some(Self { ws, _on_open: on_open, _on_message: on_message, _on_close: on_close })
    }

    /// Whether it was handed to the browser (false when not open).
    pub fn send(&self, text: &str) -> bool {
        self.ws.ready_state() == web_sys::WebSocket::OPEN && self.ws.send_with_str(text).is_ok()
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}

pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    for x in &mut b {
        *x = (js_sys::Math::random() * 256.0) as u8;
    }
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

// ---------------------------------------------------------------------------
// Files

/// Open the browser's file picker. `on_pick` runs only if files were chosen.
pub fn pick_files(accept: &str, multiple: bool, on_pick: impl FnOnce(Vec<File>) + 'static) {
    let doc = document();
    // Drop a picker left over from a cancelled dialog.
    if let Some(old) = PICKER.with(|p| p.borrow_mut().take()) {
        old.remove();
    }
    let Ok(input) = doc.create_element("input").and_then(|e| e.dyn_into::<HtmlInputElement>().map_err(Into::into))
    else {
        return;
    };
    input.set_type("file");
    input.set_accept(accept);
    input.set_multiple(multiple);
    let _ = input.style().set_property("display", "none");
    if let Some(body) = doc.body() {
        let _ = body.append_child(&input);
    }
    let input2 = input.clone();
    let on_change = Closure::once_into_js(move || {
        let mut files = Vec::new();
        if let Some(list) = input2.files() {
            for i in 0..list.length() {
                if let Some(f) = list.get(i) {
                    files.push(f);
                }
            }
        }
        input2.remove();
        PICKER.with(|p| p.borrow_mut().take());
        if !files.is_empty() {
            on_pick(files);
        }
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    PICKER.with(|p| *p.borrow_mut() = Some(input.clone()));
    input.click();
}

/// Download a same-origin URL (served with `Content-Disposition: attachment`)
/// without leaving the page.
pub fn download(url: &str) {
    let doc = document();
    if let Ok(a) = doc.create_element("a").and_then(|e| e.dyn_into::<HtmlAnchorElement>().map_err(Into::into)) {
        a.set_href(url);
        a.set_download("");
        let _ = a.style().set_property("display", "none");
        if let Some(body) = doc.body() {
            let _ = body.append_child(&a);
            a.click();
            a.remove();
        }
    }
}

/// Read a file as a `data:` URL.
pub fn read_data_url(file: &File, on_done: impl FnOnce(Result<String, String>) + 'static) {
    let Ok(reader) = FileReader::new() else {
        on_done(Err("FileReader unavailable".into()));
        return;
    };
    let done = Rc::new(RefCell::new(Some(on_done)));
    let (d1, d2) = (done.clone(), done);
    let r1 = reader.clone();
    let onload = Closure::once_into_js(move || {
        if let Some(f) = d1.borrow_mut().take() {
            f(r1.result().ok().and_then(|v| v.as_string()).ok_or_else(|| "could not read the file".to_owned()));
        }
    });
    let onerror = Closure::once_into_js(move || {
        if let Some(f) = d2.borrow_mut().take() {
            f(Err("could not read the file".into()));
        }
    });
    reader.set_onload(Some(onload.unchecked_ref()));
    reader.set_onerror(Some(onerror.unchecked_ref()));
    let _ = reader.read_as_data_url(file);
}

// ---------------------------------------------------------------------------
// Images

/// MIME type of base64 image data, from its first bytes.
pub fn sniff_image_mime(b64: &str) -> &'static str {
    if b64.starts_with("/9j/") {
        "image/jpeg"
    } else if b64.starts_with("R0lG") {
        "image/gif"
    } else if b64.starts_with("UklG") {
        "image/webp"
    } else {
        "image/png"
    }
}

/// Largest background texture side (keeps GPU memory in check on phones).
const MAX_BG_SIDE: f64 = 2048.0;

/// Decode an image URL (typically a `data:` URL) with the browser's own
/// decoders into RGBA pixels.
pub fn decode_image(url: &str, on_done: impl FnOnce(Result<egui::ColorImage, String>) + 'static) {
    let Ok(img) = HtmlImageElement::new() else {
        on_done(Err("images unavailable".into()));
        return;
    };
    let done = Rc::new(RefCell::new(Some(on_done)));
    let (d1, d2) = (done.clone(), done);
    let img2 = img.clone();
    let onload = Closure::once_into_js(move || {
        let r = rasterize(&img2);
        if let Some(f) = d1.borrow_mut().take() {
            f(r);
        }
    });
    let onerror = Closure::once_into_js(move || {
        if let Some(f) = d2.borrow_mut().take() {
            f(Err("the background image could not be decoded".into()));
        }
    });
    img.set_onload(Some(onload.unchecked_ref()));
    img.set_onerror(Some(onerror.unchecked_ref()));
    img.set_src(url);
}

fn rasterize(img: &HtmlImageElement) -> Result<egui::ColorImage, String> {
    let (w0, h0) = (img.natural_width() as f64, img.natural_height() as f64);
    if w0 < 1.0 || h0 < 1.0 {
        return Err("empty image".into());
    }
    let k = (MAX_BG_SIDE / w0.max(h0)).min(1.0);
    let (w, h) = ((w0 * k).round().max(1.0) as u32, (h0 * k).round().max(1.0) as u32);
    let canvas: HtmlCanvasElement =
        document().create_element("canvas").map_err(js_err)?.dyn_into().map_err(|_| "no canvas".to_owned())?;
    canvas.set_width(w);
    canvas.set_height(h);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(js_err)?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|_| "no 2d context".to_owned())?;
    ctx.draw_image_with_html_image_element_and_dw_and_dh(img, 0.0, 0.0, w as f64, h as f64).map_err(js_err)?;
    let data = ctx.get_image_data(0.0, 0.0, w as f64, h as f64).map_err(js_err)?;
    Ok(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &data.data().0))
}

// ---------------------------------------------------------------------------
// Audio

/// An `<audio>` element used as the playback clock.
pub struct Audio {
    el: HtmlAudioElement,
}

impl Audio {
    pub fn new(url: &str) -> Option<Self> {
        let el = HtmlAudioElement::new_with_src(url).ok()?;
        el.set_preload("auto");
        Some(Self { el })
    }

    pub fn time(&self) -> f64 {
        self.el.current_time()
    }

    /// Track length once known.
    pub fn duration(&self) -> Option<f64> {
        let d = self.el.duration();
        (d.is_finite() && d > 0.0).then_some(d)
    }

    pub fn paused(&self) -> bool {
        self.el.paused()
    }

    pub fn play(&self) {
        if let Ok(p) = self.el.play() {
            // Swallow the rejection if the browser blocks playback.
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(p).await;
            });
        }
    }

    pub fn pause(&self) {
        let _ = self.el.pause();
    }

    pub fn seek(&self, t: f64) {
        self.el.set_current_time(t.max(0.0));
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.el.pause();
        self.el.set_src("");
    }
}
