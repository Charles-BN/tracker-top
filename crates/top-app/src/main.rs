//! `tracker-top`: one executable that serves the master pile database and the
//! ToP optimiser as a local web page.

mod api;
mod assets;
mod watch;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use top_db::Store;

const HELP: &str = "\
tracker-top - master pile database and top-of-pile optimiser (web interface)

USAGE:
    tracker-top [OPTIONS]

Opens the interface in your browser. Everything is stored in one database file.

OPTIONS:
        --db <file>            Database file, created if missing   [default: tracker-top.db]
        --port <n>             Port of the web interface           [default: 8770]
        --host <addr>          Address to listen on. 0.0.0.0 shares it on the local
                               network (no password: trusted networks only)
                                                                   [default: 127.0.0.1]
        --watch <folder>       Import every new survey file that appears in this folder
                               (for example a Trimble Connect Sync folder). Saved in
                               the database; can also be set in Settings.
        --watch-interval <s>   Seconds between two scans of the folder  [default: 30]
        --threads <n>          Optimiser threads                   [default: all cores]
        --no-browser           Do not open the browser
    -h, --help                 Show this help
";

struct Args {
    db: PathBuf,
    port: u16,
    host: String,
    watch: Option<String>,
    watch_interval: u64,
    threads: usize,
    open_browser: bool,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut a = Args {
        db: PathBuf::from("tracker-top.db"),
        port: 8770,
        host: "127.0.0.1".into(),
        watch: None,
        watch_interval: 30,
        threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        open_browser: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "-h" | "--help" => return Ok(None),
            "--db" => a.db = PathBuf::from(value("--db")?),
            "--port" => {
                a.port = value("--port")?
                    .parse()
                    .map_err(|_| "--port must be a number")?
            }
            "--host" => a.host = value("--host")?,
            "--watch" => a.watch = Some(value("--watch")?),
            "--watch-interval" => {
                a.watch_interval = value("--watch-interval")?
                    .parse()
                    .map_err(|_| "--watch-interval must be a number")?
            }
            "--threads" => {
                a.threads = value("--threads")?
                    .parse()
                    .map_err(|_| "--threads must be a number")?
            }
            "--no-browser" => a.open_browser = false,
            other => return Err(format!("unknown option \"{other}\" (try --help)")),
        }
    }
    Ok(Some(a))
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => {
            print!("{HELP}");
            return std::process::ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let mut store = match Store::open(&args.db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot open database {}: {e}", args.db.display());
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Some(folder) = &args.watch
        && let Err(e) = store.set_watch_folder(Some(folder))
    {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    let db_path = std::fs::canonicalize(&args.db).unwrap_or(args.db.clone());
    println!("Database: {}", db_path.display());
    // Bring results up to date after a program update that changed the
    // calculation (nothing to do otherwise).
    match store.optimize(false, "program update", args.threads.max(1)) {
        Ok(run) if run.run_id.is_some() => println!("Results updated: {}", run.message),
        Ok(_) => {}
        Err(e) => eprintln!("warning: could not update results: {e}"),
    }

    let state = api::AppState {
        store: Arc::new(Mutex::new(store)),
        threads: args.threads.max(1),
        watch: Arc::new(Mutex::new(watch::WatchStatus::default())),
    };
    tokio::spawn(watch::run(state.clone(), args.watch_interval.max(5)));

    // Use the first free port from the requested one.
    let mut listener = None;
    for port in args.port..args.port.saturating_add(20) {
        let addr: SocketAddr = match format!("{}:{port}", args.host).parse() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("error: invalid --host \"{}\"", args.host);
                return std::process::ExitCode::FAILURE;
            }
        };
        if let Ok(l) = tokio::net::TcpListener::bind(addr).await {
            listener = Some(l);
            break;
        }
    }
    let Some(listener) = listener else {
        eprintln!("error: no free port from {}", args.port);
        return std::process::ExitCode::FAILURE;
    };
    let addr = listener.local_addr().expect("bound address");
    let shown_host = if args.host == "0.0.0.0" {
        "localhost".to_string()
    } else {
        args.host.clone()
    };
    let url = format!("http://{shown_host}:{}/", addr.port());
    println!("tracker-top is running at {url}");
    println!("Keep this window open while you use it; press Ctrl+C to stop.");
    if args.open_browser {
        let _ = open::that_detached(&url);
    }

    let app = api::router(state);
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        println!("Stopping.");
    };
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
