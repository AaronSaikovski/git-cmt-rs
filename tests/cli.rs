//! Binary-level CLI contracts: informational early exits and dry-run safety.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_git-cmt-rs");

/// A scratch directory outside any git repository.
///
/// `std::env::temp_dir()` resolves to a system temp location that is not under
/// version control; a per-test subdirectory keeps concurrent tests isolated.
fn non_git_dir(test_name: &str) -> PathBuf {
    let dir = env::temp_dir()
        .join("git-cmt-rs-cli-tests")
        .join(process::id().to_string())
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

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
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
        let (code, stdout, stderr) = run(&dir, &["--dry-run", "--staged", "--auto", flag]);

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
    let (code, stdout, stderr) = run(&dir, &["--dry-run", "--staged", "-a", "-v"]);

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

#[test]
fn dry_run_does_not_run_diff_helpers_or_write_textconv_cache() {
    let dir = non_git_dir("dry-run-textconv");
    let config = dir.join("empty-git-config");
    fs::write(&config, "").expect("test fixture should be writable");
    let command = |program: &str| {
        let mut cmd = Command::new(program);
        cmd.current_dir(&dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &config)
            .env_remove("GIT_EXTERNAL_DIFF");
        cmd
    };
    let git = |args: &[&str]| {
        let output = command("git")
            .args(args)
            .output()
            .expect("test subprocess should launch and exit");
        assert!(output.status.success(), "{:?}", output.stderr);
        output.stdout
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "CLI Test"]);
    git(&["config", "user.email", "cli@example.invalid"]);
    git(&["config", "diff.preview.textconv", "git hash-object"]);
    git(&["config", "diff.preview.cachetextconv", "true"]);
    fs::write(dir.join(".gitattributes"), "*.txt diff=preview\n")
        .expect("test fixture should be writable");
    fs::write(dir.join("selected.txt"), "staged content\n")
        .expect("test fixture should be writable");
    git(&["add", ".gitattributes", "selected.txt"]);
    let index = fs::read(dir.join(".git/index")).expect("test index should be readable");

    for external_diff in [None, Some("git")] {
        let mut cmd = command(BIN);
        cmd.args(["--dry-run", "--staged", "--auto"])
            // Stop after reading the diff, before any HTTP request.
            .env("OPENAI_RESPONSE_FORMAT", "invalid-test-format");
        if let Some(helper) = external_diff {
            cmd.env("GIT_EXTERNAL_DIFF", helper);
        }
        let output = cmd
            .output()
            .expect("test subprocess should launch and exit");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains("OPENAI_RESPONSE_FORMAT must be one of:"),
            "{stderr}"
        );
        assert!(output.stdout.is_empty());
        assert!(
            git(&["for-each-ref", "refs/notes/textconv/"]).is_empty(),
            "dry-run wrote a textconv cache ref"
        );
        assert_eq!(
            fs::read(dir.join(".git/index")).expect("test index should be readable"),
            index
        );
        assert_eq!(
            fs::read_to_string(dir.join("selected.txt"))
                .expect("staged fixture should be readable"),
            "staged content\n"
        );
    }
    fs::remove_dir_all(&dir).expect("test repository should be removable");
}
