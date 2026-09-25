// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use commands::AppState;
use tauri::{Emitter, Manager};

/// Look for changes that should refresh the window. Deliberately dumb and
/// bounded: every 250 ms, compare the state database's `data_version` and
/// the mtimes of files under `.kitsu/` and git's HEAD with the last look.
/// If anything moved, emit one tiny event; the window pulls what it needs.
/// There is no queue that can grow, only a dirty flag.
fn watch(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<(i64, u128)> = None;
        let mut store: Option<(PathBuf, kitsu::store::Store)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let state = app.state::<AppState>();
            let Some(ws) = state.ws.lock().ok().and_then(|g| g.clone()) else {
                continue;
            };
            if store.as_ref().map(|(p, _)| p != &ws.root).unwrap_or(true) {
                store = ws.open_store().ok().map(|s| (ws.root.clone(), s));
                last = None;
            }
            let db = store
                .as_ref()
                .and_then(|(_, s)| s.data_version().ok())
                .unwrap_or(0);
            let fs = fingerprint(&ws);
            if last != Some((db, fs)) {
                if last.is_some() {
                    let _ = app.emit("kitsu://changed", ());
                }
                last = Some((db, fs));
            }
        }
    });
}

fn fingerprint(ws: &kitsu::workspace::Workspace) -> u128 {
    fn mtime(p: &std::path::Path) -> u128 {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
    let mut acc = 0u128;
    let base = ws.root.join(kitsu::intent::DIR);
    acc = acc.wrapping_add(mtime(&base.join("kitsu.toml")));
    for kind in kitsu::intent::Kind::ALL {
        let dir = base.join(kind.dir());
        acc = acc.wrapping_mul(31).wrapping_add(mtime(&dir));
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                acc = acc.wrapping_add(mtime(&e.path()));
            }
        }
    }
    let git = ws.root.join(".git");
    acc.wrapping_mul(31)
        .wrapping_add(mtime(&git.join("HEAD")))
        .wrapping_add(mtime(&git.join("index")))
}

fn main() {
    // `kitsu-app --kitsu-cli <args>` is the kitsu CLI. The app launches its
    // agent runs this way, so a run started from the window is the exact
    // same code path as `kitsu run` in a terminal.
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "--kitsu-cli") {
        args.remove(1);
        std::process::exit(match kitsu::cli::main_with(args) {
            code if code == std::process::ExitCode::SUCCESS => 0,
            _ => 1,
        });
    }

    tauri::Builder::default()
        .manage(AppState {
            ws: Mutex::new(None),
            instance: Mutex::new(None),
        })
        .setup(|app| {
            watch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_repo,
            commands::init_repo,
            commands::trust_repo,
            commands::overview,
            commands::mark_seen,
            commands::task_detail,
            commands::run_detail,
            commands::evidence_log,
            commands::review,
            commands::file_diff,
            commands::start_run,
            commands::stop_run,
            commands::answer_ask,
            commands::answer_question,
            commands::accept_run,
            commands::discard_run,
            commands::new_entity,
            commands::plan,
            commands::update_task,
            commands::rules,
            commands::run_checks,
            commands::read_file,
            commands::write_file,
            commands::list_files,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("kitsu: failed to start the window: {e}");
            std::process::exit(1);
        });
}
