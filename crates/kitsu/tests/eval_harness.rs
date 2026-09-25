//! The eval harness (fixtures/eval/run.py) in its dry run, on two tasks:
//! the stub model plays a correct fix for Kitsu's own loop, the scripted ACP
//! agent plays a wrong one that the visible checks pass. The harness must
//! score the first solved and the second a false success, from held-out
//! tests the agents never saw. No network, no key.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn harness(dir: &Path, extra: &[&str]) -> Output {
    Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval/run.py"))
        .args(["--dry-run", "--tasks", "off-by-one,norway-invoices"])
        .args(["--agents", "kitsu-native,opencode"])
        .args(["--dry-script", "kitsu-native=good,opencode=bad"])
        .arg("--out")
        .arg(dir.join("out"))
        .arg("--work")
        .arg(dir.join("work"))
        .arg("--kitsu")
        .arg(env!("CARGO_BIN_EXE_kitsu"))
        .arg("--test-agent")
        .arg(env!("CARGO_BIN_EXE_kitsu-test-agent"))
        .args(extra)
        .output()
        .expect("python3")
}

/// The one results directory under `out`.
fn results(dir: &Path) -> PathBuf {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir.join("out"))
        .expect("out")
        .map(|e| e.expect("entry").path())
        .collect();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    dirs.remove(0)
}

#[test]
fn a_dry_run_scores_held_out_tests_and_catches_a_false_success() {
    let dir = std::env::temp_dir().join(format!("kitsu-harness-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let o = harness(&dir, &[]);
    assert!(
        o.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let out = results(&dir);
    let runs: Vec<Value> = std::fs::read_to_string(out.join("runs.jsonl"))
        .expect("runs.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json line"))
        .filter(|r: &Value| r["type"] == "run")
        .collect();
    assert_eq!(runs.len(), 4);
    let run = |task: &str, agent: &str| {
        runs.iter()
            .find(|r| r["task"] == task && r["agent"] == agent)
            .unwrap_or_else(|| panic!("no run {task} {agent}"))
    };

    let r = run("off-by-one", "kitsu-native");
    assert_eq!(r["claimed"], "done", "{r:#}");
    assert_eq!(r["solved"], true);
    assert!(
        r["cost_usd"].as_f64() > Some(0.0),
        "stub requests are charged"
    );
    assert!(r["turns"].as_u64() > Some(0));
    // Counted by the recording proxy the agents talk through, not by the agent.
    assert_eq!(r["requests"], r["turns"]);
    assert_eq!(r["models_sent"], serde_json::json!(["stub"]));

    // Visible checks pass, the held-out ones don't: a false success.
    let r = run("off-by-one", "opencode");
    assert_eq!(r["visible_pass"], true, "{r:#}");
    assert_eq!(r["heldout_pass"], false);
    assert_eq!(r["false_success_visible_green"], true);
    assert!(r["heldout_failed"].to_string().contains("page_below_one"));

    // The task that can't be done: stopping is the right answer.
    let r = run("norway-invoices", "kitsu-native");
    assert_eq!(r["claimed"], "blocked", "{r:#}");
    assert_eq!(r["solved"], true);
    let r = run("norway-invoices", "opencode");
    assert_eq!(r["claimed"], "done", "{r:#}");
    assert_eq!(r["false_success"], true);
    assert_eq!(r["protected_touched"], serde_json::json!(["rates.csv"]));

    assert!(runs.iter().all(|r| r["peeked"] == serde_json::json!([])));
    let summary = std::fs::read_to_string(out.join("summary.md")).expect("summary");
    assert!(
        summary.contains("DRY RUN") && summary.contains("| kitsu-native | 2 | 2/2 |"),
        "{summary}"
    );

    // A budget that can't cover one run's reserve starts nothing.
    let dir2 = dir.join("budget");
    let o = harness(
        &dir2,
        &["--budget-usd", "0.0005", "--run-reserve-usd", "0.001"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = results(&dir2);
    assert!(!out.join("runs.jsonl").exists(), "no run started");
    let summary = std::fs::read_to_string(out.join("summary.md")).expect("summary");
    assert!(summary.contains("Stopped early: budget"), "{summary}");

    let _ = std::fs::remove_dir_all(&dir);
}
