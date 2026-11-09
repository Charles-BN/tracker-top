//! Web files compiled into the executable, so it runs offline as a single file.

pub struct Asset {
    pub path: &'static str,
    pub mime: &'static str,
    pub body: &'static [u8],
}

macro_rules! asset {
    ($path:literal, $mime:literal) => {
        Asset {
            path: $path,
            mime: $mime,
            body: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/", $path)),
        }
    };
}

pub const ASSETS: &[Asset] = &[
    asset!("index.html", "text/html; charset=utf-8"),
    asset!("style.css", "text/css; charset=utf-8"),
    asset!("app.js", "text/javascript; charset=utf-8"),
    asset!("view3d.js", "text/javascript; charset=utf-8"),
    asset!("profile.js", "text/javascript; charset=utf-8"),
    asset!("sync.js", "text/javascript; charset=utf-8"),
    asset!("map2d.js", "text/javascript; charset=utf-8"),
    asset!("config.js", "text/javascript; charset=utf-8"),
    asset!(
        "vendor/three.module.min.js",
        "text/javascript; charset=utf-8"
    ),
    asset!("vendor/OrbitControls.js", "text/javascript; charset=utf-8"),
    asset!("vendor/delaunator.min.js", "text/javascript; charset=utf-8"),
    asset!("vendor/LICENSES.txt", "text/plain; charset=utf-8"),
];

pub fn find(path: &str) -> Option<&'static Asset> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    ASSETS.iter().find(|a| a.path == path)
}
