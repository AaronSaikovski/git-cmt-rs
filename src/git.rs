//! Git subprocess operations in the invocation directory.

use anyhow::{Context, Result, anyhow};
use std::process::{Command, Stdio};

const MAX_DIFF_CHARS: usize = 3072;

pub(crate) fn stage_all_changes() -> Result<()> {
    let status = Command::new("git")
        .args(["add", "."])
        .status()
        .context("failed to run `git add .`")?;

    if !status.success() {
        return Err(anyhow!("git add failed with status: {}", status));
    }

    Ok(())
}

pub(crate) fn get_staged_changes() -> Result<String> {
    let output = Command::new("git")
        .args(["diff", "--cached", "-b"])
        .output()
        .context("failed to run `git diff --cached -b`")?;

    if !output.status.success() {
        return Err(anyhow!("git diff failed with status: {}", output.status));
    }

    let mut diff = String::from_utf8(output.stdout).context("git output was not valid UTF-8")?;

    if diff.trim().is_empty() {
        return Err(anyhow!("no staged changes found"));
    }

    if diff.chars().count() > MAX_DIFF_CHARS {
        let truncated: String = diff.chars().take(MAX_DIFF_CHARS).collect();
        diff = format!("{truncated}\n... (truncated)");
    }

    Ok(diff)
}

fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .context("failed to run `git symbolic-ref`")?;
    if !output.status.success() {
        return Err(anyhow!(
            "could not determine current branch (detached HEAD?)"
        ));
    }
    Ok(String::from_utf8(output.stdout)
        .context("git output was not valid UTF-8")?
        .trim()
        .to_string())
}

fn has_upstream() -> bool {
    Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) fn create_commit(message: &str, edit: bool) -> Result<()> {
    // Run: git commit [-e] -m "<line>" (-e opens the editor unless --auto)
    let mut commit_cmd = Command::new("git");
    commit_cmd.arg("commit");
    if edit {
        commit_cmd.arg("-e");
    }
    let status = commit_cmd
        .args(["-m", message])
        .status()
        .context("failed to run `git commit`")?;

    if !status.success() {
        return Err(anyhow!("git commit failed with status: {status}"));
    }

    Ok(())
}

pub(crate) fn push() -> Result<()> {
    let mut push_cmd = Command::new("git");
    push_cmd.arg("push");
    if !has_upstream() {
        let branch = current_branch()?;
        eprintln!("No upstream set; pushing with `--set-upstream origin {branch}`");
        push_cmd.args(["--set-upstream", "origin", &branch]);
    }
    let status = push_cmd.status().context("failed to run `git push`")?;

    if !status.success() {
        return Err(anyhow!("git push failed with status: {status}"));
    }

    Ok(())
}
