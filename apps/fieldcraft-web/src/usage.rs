//! Anonymous usage in the browser (docs/privacy.md): the same events and the same consent as the
//! desktop app, the choice kept in the browser's local storage with the other preferences. Queued
//! events are posted with `fetch` every half minute and when the page is closed or left (`keepalive`, so
//! a closing page still sends them); a failed post is dropped silently.
//!
//! The project key is a build setting, `FIELDCRAFT_POSTHOG_KEY`: a build without one asks nothing
//! and sends nothing.

use fieldcraft_engine::usage;
use fieldcraft_ui_egui::files::PrefsStore;
use fieldcraft_ui_egui::privacy::UsageHost;
use wasm_bindgen::JsCast as _;

const KEY: Option<&str> = option_env!("FIELDCRAFT_POSTHOG_KEY");
/// Where the preferences are kept in local storage.
const PREFS_KEY: &str = "fieldcraft-prefs";
/// How often queued events are sent (seconds).
pub const INTERVAL: f64 = 30.0;

fn key() -> Option<&'static str> {
    KEY.map(str::trim).filter(|k| !k.is_empty())
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The preferences in local storage (navigation, privacy).
pub fn prefs_store() -> PrefsStore {
    PrefsStore {
        load: Box::new(|| local_storage()?.get_item(PREFS_KEY).ok().flatten().map(String::into_bytes)),
        save: Box::new(|bytes| {
            let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
            local_storage().ok_or("browser storage is turned off")?.set_item(PREFS_KEY, text).map_err(crate::web::js_err)
        }),
    }
}

/// What the window needs to ask and record, when the build has a key.
pub fn host() -> Option<UsageHost> {
    // The browser's clock for command durations (the plain wasm target has none).
    usage::set_clock(js_sys::Date::now);
    key().is_some().then(|| UsageHost {
        platform: "web",
        os: "web",
        arch: "wasm32",
        new_id: Box::new(|| web_sys::window()?.crypto().ok().map(|c| c.random_uuid())),
    })
}

/// Post what is queued.
pub fn send() {
    let (Some(key), Some(window)) = (key(), web_sys::window()) else { return };
    let url = format!("{}/batch/", usage::DEFAULT_HOST);
    while let Some(batch) = usage::take_batch() {
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_body(&wasm_bindgen::JsValue::from_str(&batch.body(key)));
        // The Fetch standard's `keepalive` (a closing page still sends it); web-sys has no setter.
        let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &true.into());
        let headers = web_sys::Headers::new().ok();
        if let Some(h) = &headers {
            let _ = h.set("Content-Type", "application/json");
            init.set_headers(h);
        }
        let promise = window.fetch_with_str_and_init(&url, &init);
        wasm_bindgen_futures::spawn_local(async move {
            // Offline or refused: the batch is dropped.
            if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise).await {
                log::debug!("usage: not sent: {}", crate::web::js_err(e));
            }
        });
    }
}

/// When the page is closed or left: the session's length, then send.
pub fn send_when_hidden(window: &web_sys::Window) {
    let on_hide = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
        usage::closing();
        send();
    });
    if let Err(e) = window.add_event_listener_with_callback("pagehide", on_hide.as_ref().unchecked_ref()) {
        log::warn!("usage: {}", crate::web::js_err(e));
    }
    on_hide.forget();
}
