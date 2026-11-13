//! Git subprocess operations in the invocation directory.

use anyhow::{Context, Result, anyhow};
use std::io::{self, ErrorKind, Read};
use std::process::{Command, Stdio};
use std::str;

const MAX_DIFF_CHARS: usize = 3072;
const DIFF_TRUNCATION_MARKER: &str = "\n... (truncated)";

/// Whether Git diff may use configured helpers that can write repository state.
pub(crate) enum DiffMode {
    /// Honor the normal Git diff configuration.
    Standard,
    /// Disable textconv and external diff helpers.
    ReadOnly,
}

/// How the generated message is reviewed before committing.
pub(crate) enum CommitReview {
    /// Open Git's editor with the generated message.
    Editor,
    /// Commit the generated message without opening the editor.
    AcceptGenerated,
}

/// Stages changes under the invocation directory with `git add .`.
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

/// Reads the staged diff, retaining at most 3072 characters plus a truncation marker.
///
/// Drains and validates all output as UTF-8, and rejects empty or whitespace-only diffs.
pub(crate) fn get_staged_changes(mode: DiffMode) -> Result<String> {
    let mut command = Command::new("git");
    command.args(["diff", "--cached", "-b"]);
    if matches!(mode, DiffMode::ReadOnly) {
        // Textconv caching writes Git refs; custom diff helpers can mutate files.
        command.args(["--no-textconv", "--no-ext-diff"]);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to run `git diff --cached -b`")?;
    let stdout = child.stdout.take().expect("Git stdout was piped");
    let diff = read_bounded_diff(stdout);

    // Closing stdout and stopping the child on a read error prevents a blocked
    // writer. Always wait, including when stopping an already-exited child fails.
    let stop_result = if diff
        .as_ref()
        .is_err_and(|error| error.downcast_ref::<io::Error>().is_some())
    {
        child.kill()
    } else {
        Ok(())
    };
    let status = child.wait().context("failed to wait for `git diff`")?;

    if !status.success() {
        let message = format!("git diff failed with status: {status}");
        return match diff {
            Ok(_) => Err(anyhow!(message)),
            Err(error) => Err(error.context(message)),
        };
    }
    if let Err(stop_error) = stop_result {
        return Err(diff
            .expect_err("only a failed read stops Git")
            .context(format!("failed to stop `git diff`: {stop_error}")));
    }

    diff
}

fn read_bounded_diff(mut reader: impl Read) -> Result<String> {
    let mut diff = String::with_capacity(MAX_DIFF_CHARS * 4 + DIFF_TRUNCATION_MARKER.len());
    let mut buffer = [0_u8; 8192];
    let mut pending = 0;
    let mut retained_chars = 0;
    let mut truncated = false;
    let mut has_non_whitespace = false;
    let mut utf8_error = None;

    loop {
        let count = match reader.read(&mut buffer[pending..]) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("failed to read `git diff` output"),
        };
        if count == 0 {
            if let Some(error) = utf8_error {
                return Err(error).context("git output was not valid UTF-8");
            }
            // An incomplete final code point is invalid, even in discarded output.
            str::from_utf8(&buffer[..pending]).context("git output was not valid UTF-8")?;
            break;
        }

        if utf8_error.is_some() {
            continue;
        }
        let filled = pending + count;
        let valid = match str::from_utf8(&buffer[..filled]) {
            Ok(valid) => valid,
            Err(error) if error.error_len().is_none() => {
                str::from_utf8(&buffer[..error.valid_up_to()])
                    .context("git output was not valid UTF-8")?
            }
            Err(error) => {
                // Preserve Git's exit status by draining even invalid output.
                utf8_error = Some(error);
                pending = 0;
                continue;
            }
        };
        let valid_len = valid.len();

        if !has_non_whitespace {
            has_non_whitespace = !valid.trim().is_empty();
        }
        if !truncated {
            for (index, _) in valid.char_indices() {
                if retained_chars == MAX_DIFF_CHARS {
                    diff.push_str(&valid[..index]);
                    truncated = true;
                    break;
                }
                retained_chars += 1;
            }
            if !truncated {
                diff.push_str(valid);
            }
        }

        // Only an incomplete UTF-8 code point (at most three bytes) is carried.
        // Discarded tails still need UTF-8 validation, but no character counting.
        buffer.copy_within(valid_len..filled, 0);
        pending = filled - valid_len;
    }

    if !has_non_whitespace {
        return Err(anyhow!("no staged changes found"));
    }
    if truncated {
        diff.push_str(DIFF_TRUNCATION_MARKER);
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

/// Creates a commit with the generated message, optionally opening Git's editor.
pub(crate) fn create_commit(message: &str, review: CommitReview) -> Result<()> {
    let mut commit_cmd = Command::new("git");
    commit_cmd.arg("commit");
    if matches!(review, CommitReview::Editor) {
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

/// Pushes the current branch, setting its upstream to `origin` if none exists.
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

#[cfg(test)]
mod tests {
    use super::{DIFF_TRUNCATION_MARKER, MAX_DIFF_CHARS};
    use std::io::{self, Cursor, Read};

    struct Chunked<R> {
        inner: R,
        chunk_size: usize,
    }

    impl<R: Read> Read for Chunked<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.chunk_size);
            self.inner.read(&mut buffer[..count])
        }
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read failure"))
        }
    }

    #[test]
    fn bounded_diff_keeps_exact_character_cap() {
        let input = "é".repeat(MAX_DIFF_CHARS);
        assert_eq!(
            super::read_bounded_diff(input.as_bytes()).expect("valid diff at the character cap"),
            input
        );
    }

    #[test]
    fn bounded_diff_truncates_one_character_over_cap() {
        let input = "🦀".repeat(MAX_DIFF_CHARS + 1);
        let expected = format!("{}{DIFF_TRUNCATION_MARKER}", "🦀".repeat(MAX_DIFF_CHARS));
        assert_eq!(
            super::read_bounded_diff(input.as_bytes()).expect("valid diff above the character cap"),
            expected
        );
    }

    #[test]
    fn bounded_diff_preserves_split_multibyte_characters() {
        let prefix = "a".repeat(MAX_DIFF_CHARS - 1);
        let input = format!("{prefix}🦀é");
        let expected = format!("{prefix}🦀{DIFF_TRUNCATION_MARKER}");
        for chunk_size in 1..=4 {
            let reader = Chunked {
                inner: Cursor::new(input.as_bytes()),
                chunk_size,
            };
            assert_eq!(
                super::read_bounded_diff(reader).expect("valid UTF-8 split across reads"),
                expected
            );
        }
    }

    #[test]
    fn bounded_diff_drains_large_discarded_tail() {
        let mut reader = io::repeat(b'x').take(16 * 1024 * 1024);
        let expected = format!("{}{DIFF_TRUNCATION_MARKER}", "x".repeat(MAX_DIFF_CHARS));
        assert_eq!(
            super::read_bounded_diff(&mut reader).expect("valid diff with a large discarded tail"),
            expected
        );
        assert_eq!(reader.limit(), 0);
    }

    #[test]
    fn bounded_diff_reports_read_failure_after_character_cap() {
        let reader = io::repeat(b'x')
            .take((MAX_DIFF_CHARS + 1) as u64)
            .chain(FailingReader);
        let error =
            super::read_bounded_diff(reader).expect_err("injected read failure must propagate");
        assert_eq!(error.to_string(), "failed to read `git diff` output");
        assert_eq!(
            error
                .downcast_ref::<io::Error>()
                .expect("read failure retains its I/O error")
                .to_string(),
            "injected read failure"
        );
    }

    #[test]
    fn bounded_diff_rejects_invalid_utf8_in_retained_and_discarded_output() {
        for prefix_len in [0, MAX_DIFF_CHARS + 1] {
            for invalid in [&[0xff][..], &[0xc0, 0xaf][..], &[0xf0, 0x9f][..]] {
                let reader = Chunked {
                    inner: io::repeat(b'x')
                        .take(prefix_len as u64)
                        .chain(Cursor::new(invalid)),
                    chunk_size: 1,
                };
                let error =
                    super::read_bounded_diff(reader).expect_err("invalid UTF-8 must be rejected");
                assert_eq!(error.to_string(), "git output was not valid UTF-8");
            }
        }
    }

    #[test]
    fn bounded_diff_drains_output_after_invalid_utf8() {
        let length = (MAX_DIFF_CHARS + 1 + 1 + 32768) as u64;
        let mut reader = io::repeat(b'x')
            .take((MAX_DIFF_CHARS + 1) as u64)
            .chain(Cursor::new([0xff]))
            .chain(io::repeat(b'x').take(32768))
            .take(length);
        let error =
            super::read_bounded_diff(&mut reader).expect_err("invalid UTF-8 must be rejected");
        assert_eq!(error.to_string(), "git output was not valid UTF-8");
        assert_eq!(reader.limit(), 0);
    }

    #[test]
    fn bounded_diff_rejects_empty_and_whitespace_only_output() {
        for input in [String::new(), " \n\t\u{2003}".repeat(MAX_DIFF_CHARS)] {
            let error = super::read_bounded_diff(input.as_bytes())
                .expect_err("empty or whitespace-only diff must be rejected");
            assert_eq!(error.to_string(), "no staged changes found");
        }
    }

    #[test]
    fn bounded_diff_accepts_non_whitespace_only_in_discarded_tail() {
        let prefix = " ".repeat(MAX_DIFF_CHARS);
        let input = format!("{prefix}x");
        assert_eq!(
            super::read_bounded_diff(input.as_bytes())
                .expect("non-whitespace in discarded output makes the diff nonempty"),
            format!("{prefix}{DIFF_TRUNCATION_MARKER}")
        );
    }
}
