#[path = "../src/shallow.rs"]
mod shallow;

use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("git command failed");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(path: &Path) {
    run_git(path, &["init"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
    std::fs::write(path.join("README.md"), "hello").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init"]);
    run_git(path, &["branch", "-m", "main"]);
}

#[test]
fn detects_non_shallow_repo() {
    let temp = tempfile::tempdir().unwrap();
    init_repo(temp.path());
    let result = shallow::is_shallow_repo(temp.path()).unwrap();
    assert!(!result);
}

#[test]
fn detects_shallow_repo_by_file() {
    let temp = tempfile::tempdir().unwrap();
    init_repo(temp.path());

    let repo = git2::Repository::discover(temp.path()).unwrap();
    let shallow_path = repo.path().join("shallow");
    std::fs::write(&shallow_path, "0000000000000000000000000000000000000000\n").unwrap();

    let result = shallow::is_shallow_repo(temp.path()).unwrap();
    assert!(result);
}

#[test]
fn reads_shallow_boundary_oids() {
    let temp = tempfile::tempdir().unwrap();
    init_repo(temp.path());
    let root = run_git(temp.path(), &["rev-list", "--max-parents=0", "HEAD"]);

    let repo = git2::Repository::discover(temp.path()).unwrap();
    let shallow_path = repo.path().join("shallow");
    std::fs::write(&shallow_path, format!("{}\n", root)).unwrap();

    let oids = shallow::shallow_boundary_oids(temp.path()).unwrap();
    assert_eq!(oids.len(), 1);
    assert_eq!(oids[0].to_string(), root);
}
