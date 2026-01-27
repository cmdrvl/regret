use assert_cmd::Command;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

const UPDATE_ENV: &str = "REGRET_UPDATE_SNAPSHOTS";

pub fn run_regret(repo_path: &Path, args: &[&str]) -> String {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("regret"));
    let output = command
        .current_dir(repo_path)
        .args(args)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("NO_COLOR", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run regret");

    assert!(
        output.status.success(),
        "regret failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    normalize_output(&stdout, repo_path)
}

pub fn assert_snapshot(snapshot_name: &str, output: &str) {
    let snapshot_path = snapshot_path(snapshot_name);
    if should_update_snapshots() {
        fs::create_dir_all(snapshot_path.parent().expect("snapshot parent"))
            .expect("create snapshot dir");
        fs::write(&snapshot_path, output).expect("write snapshot");
        return;
    }

    let expected = fs::read_to_string(&snapshot_path).expect("read snapshot");
    assert_eq!(output, expected, "snapshot mismatch: {snapshot_name}");
}

fn snapshot_path(snapshot_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join(format!("{snapshot_name}.txt"))
}

fn should_update_snapshots() -> bool {
    matches!(std::env::var(UPDATE_ENV).as_deref(), Ok("1") | Ok("true"))
}

pub fn normalize_output(raw: &str, repo_path: &Path) -> String {
    let mut normalized = raw.replace("\r\n", "\n");
    normalized = normalized.replace('\\', "/");

    let repo_string = repo_path.to_string_lossy().replace('\\', "/");
    let cache_string = repo_path
        .join(".regret")
        .to_string_lossy()
        .replace('\\', "/");

    if !repo_string.is_empty() {
        normalized = normalized.replace(&repo_string, "<REPO>");
    }
    if !cache_string.is_empty() {
        normalized = normalized.replace(&cache_string, "<CACHE>");
    }

    let repo_id_re = Regex::new(r#""repo_id"\s*:\s*"[a-f0-9]{8,}""#).expect("repo_id regex");
    normalized = repo_id_re
        .replace_all(&normalized, "\"repo_id\":\"<REPO_ID>\"")
        .into_owned();

    let version_re =
        Regex::new(r#""(tool_version|regret_version)"\s*:\s*"[^"]+""#).expect("version regex");
    normalized = version_re
        .replace_all(&normalized, |caps: &regex::Captures<'_>| {
            format!("\"{}\":\"<VERSION>\"", &caps[1])
        })
        .into_owned();

    normalized
}
