//! tracker-top in the browser.
//!
//! The same database, import, optimisation and export code as the desktop
//! program, compiled to WebAssembly. The page sends the same API requests it
//! sends to the desktop program; `Engine::request` answers them from an
//! in-memory database, which the page stores in the browser (IndexedDB) after
//! every change, using `Engine::save`.

use top_db::{Store, api};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);
}

/// Answer to one request.
#[wasm_bindgen]
pub struct Reply {
    status: u16,
    content_type: String,
    body: Vec<u8>,
    filename: Option<String>,
    changed: bool,
}

#[wasm_bindgen]
impl Reply {
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> u16 {
        self.status
    }

    #[wasm_bindgen(getter)]
    pub fn content_type(&self) -> String {
        self.content_type.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn filename(&self) -> Option<String> {
        self.filename.clone()
    }

    /// The database changed: store it.
    #[wasm_bindgen(getter)]
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The body, once (moved out).
    pub fn take_body(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.body)
    }
}

#[wasm_bindgen]
pub struct Engine {
    store: Store,
}

fn js(e: String) -> JsError {
    JsError::new(&e)
}

#[wasm_bindgen]
impl Engine {
    /// Open the stored database, or start an empty one.
    #[wasm_bindgen(constructor)]
    pub fn new(saved: Option<Vec<u8>>) -> Result<Engine, JsError> {
        std::panic::set_hook(Box::new(|info| error(&format!("tracker-top: {info}"))));
        let mut store = match saved {
            Some(bytes) if !bytes.is_empty() => Store::from_bytes(&bytes).map_err(js)?,
            _ => Store::open_in_memory().map_err(js)?,
        };
        store.set_label("this browser");
        Ok(Engine { store })
    }

    /// Bring results up to date after an update of the calculation.
    /// True when anything was recomputed (then save).
    pub fn refresh(&mut self) -> Result<bool, JsError> {
        let run = self
            .store
            .optimize(false, "program update", 1)
            .map_err(js)?;
        Ok(run.run_id.is_some())
    }

    /// One API request, as the page would send it to the desktop program.
    /// `watch` is the watch-folder status (JSON) shown on the page.
    pub fn request(
        &mut self,
        method: &str,
        path: &str,
        query: &str,
        body: &[u8],
        watch: &str,
    ) -> Reply {
        let host = api::Host {
            threads: 1,
            mode: "browser",
            watch: serde_json::from_str(watch).unwrap_or_default(),
            check_folder: false,
        };
        let r = api::handle(
            &mut self.store,
            &host,
            &api::Request {
                method,
                path,
                query,
                body,
            },
        );
        Reply {
            changed: api::is_write(method) && r.status < 400,
            status: r.status,
            content_type: r.content_type,
            body: r.body,
            filename: r.filename,
        }
    }

    /// The database file, to store or download.
    pub fn save(&mut self) -> Result<Vec<u8>, JsError> {
        self.store.to_bytes().map_err(js)
    }

    /// SHA-256 of a file, to recognise files already imported.
    pub fn sha256(bytes: &[u8]) -> String {
        top_db::sha256_hex(bytes)
    }
}
