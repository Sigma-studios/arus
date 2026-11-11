//! Eightcount web client: view and edit ArrangeUs choreographies in the browser.
//!
//! The geometry / editing logic (`geom`, `view`, `edit`, `undo`, `util`) and
//! the stage painter (`paint`) are plain Rust + egui and are unit-tested
//! natively. Everything that touches the browser lives in `web` and the UI in
//! `ui`, both compiled for `wasm32` only.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

mod edit;
mod geom;
mod paint;
mod theme;
mod undo;
mod util;
mod view;

#[cfg(target_arch = "wasm32")]
mod ui;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("arrus-web is a browser app. Build it with `trunk build --release` in crates/arrus-web/.");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().and_then(|w| w.document()).expect("no document");
        let canvas = document
            .get_element_by_id("arus_canvas")
            .expect("no #arus_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#arus_canvas is not a canvas");
        let options = eframe::WebOptions::default();
        let result = eframe::WebRunner::new()
            .start(canvas, options, Box::new(|cc| Ok(Box::new(ui::App::new(cc)))))
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_text_content(Some(&format!("Eightcount failed to start: {e:?}"))),
            }
        }
    });
}
