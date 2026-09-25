// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use commands::AppState;
use kitsu::workspaces::List;
use tauri::{Emitter, Manager};

/// Look for changes that should refresh the window. Deliberately dumb and
/// bounded: every 250 ms, for each repository the window has opened,
/// compare the state database's `data_version` and a `stat` fingerprint of
/// `.kitsu/`, HEAD and the index with the last look. If anything moved,
/// emit one tiny event naming that repository; the window pulls what it
/// needs. There is no queue that can grow, only a dirty flag per repository.
fn watch(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        type Seen = (Option<kitsu::store::Store>, Option<(i64, u64)>);
        let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let open = app.state::<AppState>().opened();
            seen.retain(|id, _| open.iter().any(|(o, _)| o == id));
            for (id, ws) in open {
                let (store, last) = seen
                    .entry(id.clone())
                    .or_insert_with(|| (ws.open_store().ok(), None));
                let db = store
                    .as_ref()
                    .and_then(|s| s.data_version().ok())
                    .unwrap_or(0);
                let now = (db, kitsu::workspaces::fingerprint(&ws));
                if *last != Some(now) {
                    if last.is_some() {
                        let _ = app.emit("kitsu://changed", serde_json::json!({ "repo": id }));
                    }
                    *last = Some(now);
                }
            }
        }
    });
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

    // `kitsu-app ~/code/project` opens that project; otherwise the one
    // around the current directory, if any. Either way it joins the list.
    let launch_dir = args
        .get(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok());

    tauri::Builder::default()
        .manage(AppState::new(List::new(List::default_path()), launch_dir))
        .setup(|app| {
            watch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::launch_repo,
            commands::list_workspaces,
            commands::add_workspace,
            commands::remove_workspace,
            commands::rename_workspace,
            commands::move_workspace,
            commands::workspace_overview,
            commands::git_view,
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
