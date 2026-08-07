//! End-to-end checks that the informational flags are handled before any git
//! work happens.
//!
//! These run the real binary from a directory that is not a git repository, so
//! any stray `git add` / `git diff` call would surface as a non-zero exit and
//! a "not a git repository" error.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_git-cmt-rs");

/// A scratch directory outside any git repository.
///
/// `std::env::temp_dir()` resolves to a system temp location that is not under
/// version control; a per-test subdirectory keeps concurrent tests isolated.
fn non_git_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("git-cmt-rs-cli-tests")
        .join(test_name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create scratch dir");

    // Guard the premise of every test below: this really is not a repo.
    let inside = Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .current_dir(&dir)
        .output()
        .expect("failed to run git");
    assert!(
        !inside.status.success(),
        "scratch dir unexpectedly inside a git repository: {}",
        dir.display()
    );

    dir
}

fn run(dir: &PathBuf, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run git-cmt-rs");

    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn version_flag_runs_no_git_commands() {
    let dir = non_git_dir("version");

    for flag in ["-v", "--version"] {
        let (code, stdout, stderr) = run(&dir, &[flag]);

        assert_eq!(code, 0, "{flag} exited {code}; stderr: {stderr}");
        assert_eq!(
            stdout.trim(),
            format!("git-cmt-rs {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(
            stderr.is_empty(),
            "{flag} wrote to stderr, so it did more than print the version: {stderr}"
        );
        assert!(
            !stderr.contains("not a git repository") && !stderr.contains("Staged all changes"),
            "{flag} touched git: {stderr}"
        );
    }
}

#[test]
fn help_flag_runs_no_git_commands() {
    let dir = non_git_dir("help");

    for flag in ["-h", "--help"] {
        let (code, stdout, stderr) = run(&dir, &[flag]);

        assert_eq!(code, 0, "{flag} exited {code}; stderr: {stderr}");
        assert!(
            stdout.contains("Usage: git-cmt-rs"),
            "{flag} stdout: {stdout}"
        );
        assert!(stderr.is_empty(), "{flag} wrote to stderr: {stderr}");
    }
}

#[test]
fn version_flag_wins_over_auto_and_never_commits() {
    let dir = non_git_dir("auto-version");
    let (code, stdout, stderr) = run(&dir, &["-a", "-v"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        format!("git-cmt-rs {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        !stderr.contains("Staged all changes"),
        "-a -v started the commit flow: {stderr}"
    );
}

#[test]
fn unknown_flag_exits_two_before_touching_git() {
    let dir = non_git_dir("unknown");
    let (code, _stdout, stderr) = run(&dir, &["--bogus"]);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("unknown argument '--bogus'"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("not a git repository"),
        "argument errors must precede git calls: {stderr}"
    );
}
