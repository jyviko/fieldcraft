//! A new deployment of the web app: every half hour the page's own address is fetched past the browser's cache, and when it loads another build
//! of the app than the one running (Trunk names each build's files by their content hash), the
//! status bar offers "Reload to update" (docs/updates.md). Reloading asks first when the model
//! has unsaved changes, as closing the page does.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;

/// How often the page looks for a new deployment (seconds).
pub const INTERVAL: f64 = 30.0 * 60.0;

/// Watches for a new deployment.
pub struct Watch {
    /// The build this page runs.
    running: Option<String>,
    /// A fetch found another build.
    found: Rc<Cell<bool>>,
    /// App time of the last look.
    pub looked_at: f64,
}

impl Watch {
    pub fn start() -> Watch {
        let html = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()).map(|e| e.outer_html()).unwrap_or_default();
        let running = crate::build_in_page(&html).map(str::to_string);
        if running.is_none() {
            log::info!("deploy: no hashed build in the page (a development server?); not watching");
        }
        Watch { running, found: Rc::default(), looked_at: 0.0 }
    }

    /// Whether a newer deployment was found.
    pub fn found(&self) -> bool {
        self.found.get()
    }

    /// Fetch the page again and compare its build with the running one.
    pub fn look(&self) {
        let (Some(running), Some(window)) = (self.running.clone(), web_sys::window()) else { return };
        if self.found.get() {
            return;
        }
        let url = window.location().pathname().unwrap_or_else(|_| "./".into());
        let init = web_sys::RequestInit::new();
        init.set_cache(web_sys::RequestCache::NoStore);
        let promise = window.fetch_with_str_and_init(&url, &init);
        let found = self.found.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let text = async {
                let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?.dyn_into().ok()?;
                if !response.ok() {
                    return None;
                }
                wasm_bindgen_futures::JsFuture::from(response.text().ok()?).await.ok()?.as_string()
            };
            // Offline or refused: look again next time.
            if let Some(html) = text.await
                && let Some(deployed) = crate::build_in_page(&html)
                && deployed != running
            {
                log::info!("deploy: {deployed} is deployed (running {running})");
                found.set(true);
            }
        });
    }
}

/// Reload the page (the new deployment loads).
pub fn reload() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}
