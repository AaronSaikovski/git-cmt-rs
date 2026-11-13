//! Command-line options and terminal interaction.

use anyhow::{Context, Result, anyhow};
use std::io::{self, BufRead, Write};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
git-cmt-rs - AI-powered Conventional Commit message generator

Usage: git-cmt-rs [OPTIONS]

Options:
  -a, --auto     Accept the generated message without opening the editor and
                 push without prompting for confirmation
      --dry-run  Print a message for staged changes without staging, committing,
                 or pushing (overrides --auto; still calls the model)
      --staged   Use only staged changes; do not run git add
  -v, --version  Print the version and exit
  -h, --help     Print this help message";

// ---------- CLI ----------
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    /// Skip the editor review and the push confirmation prompt.
    pub(crate) auto: bool,
    /// Generate from the existing index without staging, committing, or pushing.
    pub(crate) dry_run: bool,
    /// Preserve the existing index instead of staging all changes.
    pub(crate) staged: bool,
    /// Print usage and exit.
    help: bool,
    /// Print the version and exit.
    version: bool,
}

/// Parse this invocation's flags, rejecting unknown or positional arguments.
pub(crate) fn parse_args<I, S>(args: I) -> Result<Args>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = Args::default();

    for arg in args {
        match arg.as_ref() {
            "-a" | "--auto" => parsed.auto = true,
            "--dry-run" => parsed.dry_run = true,
            "--staged" => parsed.staged = true,
            "-v" | "--version" => parsed.version = true,
            "-h" | "--help" => parsed.help = true,
            other => return Err(anyhow!("unknown argument '{other}'\n\n{USAGE}")),
        }
    }

    Ok(parsed)
}

/// Text to print before exiting for the informational flags, if either was
/// requested. `--help` wins when both are passed.
pub(crate) fn early_exit_message(args: &Args) -> Option<String> {
    if args.help {
        Some(USAGE.to_string())
    } else if args.version {
        Some(format!("git-cmt-rs {VERSION}"))
    } else {
        None
    }
}

/// Read an explicit push decision; EOF cancels without pushing.
pub(crate) fn confirm_push() -> Result<bool> {
    confirm_push_with(&mut io::stdin().lock(), &mut io::stderr().lock())
}

fn confirm_push_with(input: &mut impl BufRead, output: &mut impl Write) -> Result<bool> {
    let mut answer = String::new();
    loop {
        write!(output, "Push commit to remote? (y/n): ")?;
        output.flush()?;
        answer.clear();
        if input
            .read_line(&mut answer)
            .context("failed to read user input")?
            == 0
        {
            return Ok(false);
        }

        let answer = answer.trim();
        if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            return Ok(true);
        }
        if answer.eq_ignore_ascii_case("n") || answer.eq_ignore_ascii_case("no") {
            return Ok(false);
        }
        writeln!(output, "Please answer 'y' or 'n'")?;
    }
}

#[must_use]
fn unsafe_terminal_char(ch: char) -> bool {
    ch.is_control()
        || matches!(
            ch,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

/// Escape terminal controls and directional overrides; leave ordinary Unicode intact.
pub(crate) fn sanitize_terminal(text: &mut String) {
    let Some((start, _)) = text
        .char_indices()
        .find(|&(_, ch)| unsafe_terminal_char(ch))
    else {
        return;
    };
    let mut safe = String::with_capacity(text.len());
    safe.push_str(&text[..start]);
    for ch in text[start..].chars() {
        if unsafe_terminal_char(ch) {
            safe.extend(ch.escape_default());
        } else {
            safe.push(ch);
        }
    }
    *text = safe;
}

#[cfg(test)]
mod tests {
    use super::{Args, USAGE, VERSION};

    #[test]
    fn push_confirmation_cancels_on_eof() {
        for input in ["", "invalid\n", "\n"] {
            assert!(
                !super::confirm_push_with(&mut input.as_bytes(), &mut Vec::new())
                    .expect("in-memory confirmation I/O should succeed")
            );
        }
    }

    #[test]
    fn push_confirmation_accepts_explicit_answers_after_invalid_input() {
        for (input, expected) in [("invalid\n YES \n", true), ("n\n", false), ("No", false)] {
            assert_eq!(
                super::confirm_push_with(&mut input.as_bytes(), &mut Vec::new())
                    .expect("in-memory confirmation I/O should succeed"),
                expected
            );
        }
    }

    #[test]
    fn terminal_sanitization_neutralizes_controls_and_directional_overrides() {
        let mut text = "fix: café \u{1b}[2J\u{9b}31m\r\n\t\u{7}\u{202e}hidden".to_string();
        super::sanitize_terminal(&mut text);
        assert_eq!(
            text,
            "fix: café \\u{1b}[2J\\u{9b}31m\\r\\n\\t\\u{7}\\u{202e}hidden"
        );
    }

    #[test]
    fn terminal_sanitization_preserves_unicode_and_is_idempotent() {
        let mut text = "fix: 修正 café 👩\u{200d}💻".to_string();
        super::sanitize_terminal(&mut text);
        assert_eq!(text, "fix: 修正 café 👩\u{200d}💻");
        let mut escaped = "fix: \\u{1b}[2J".to_string();
        super::sanitize_terminal(&mut escaped);
        assert_eq!(escaped, "fix: \\u{1b}[2J");
    }

    // ---------- parse_args ----------

    #[test]
    fn args_default_to_interactive() {
        let args =
            super::parse_args(Vec::<String>::new()).expect("valid CLI arguments should parse");
        assert_eq!(args, Args::default());
        assert!(!args.auto);
        assert!(!args.help);
        assert!(!args.version);
    }

    #[test]
    fn args_short_auto_flag() {
        assert!(
            super::parse_args(["-a"])
                .expect("short auto flag should parse")
                .auto
        );
    }

    #[test]
    fn args_long_auto_flag() {
        assert!(
            super::parse_args(["--auto"])
                .expect("long auto flag should parse")
                .auto
        );
    }

    #[test]
    fn args_help_flags() {
        assert!(
            super::parse_args(["-h"])
                .expect("short help flag should parse")
                .help
        );
        assert!(
            super::parse_args(["--help"])
                .expect("long help flag should parse")
                .help
        );
    }

    #[test]
    fn args_version_flags() {
        assert!(
            super::parse_args(["-v"])
                .expect("short version flag should parse")
                .version
        );
        assert!(
            super::parse_args(["--version"])
                .expect("long version flag should parse")
                .version
        );
    }

    #[test]
    fn args_each_flag_sets_only_its_own_field() {
        let auto = super::parse_args(["-a"]).expect("valid CLI arguments should parse");
        assert_eq!(
            auto,
            Args {
                auto: true,
                ..Args::default()
            }
        );

        let help = super::parse_args(["-h"]).expect("valid CLI arguments should parse");
        assert_eq!(
            help,
            Args {
                help: true,
                ..Args::default()
            }
        );

        let version = super::parse_args(["-v"]).expect("valid CLI arguments should parse");
        assert_eq!(
            version,
            Args {
                version: true,
                ..Args::default()
            }
        );
    }

    #[test]
    fn args_repeated_and_combined_flags() {
        let args = super::parse_args(["-a", "--auto", "--dry-run", "--staged", "--help", "-v"])
            .expect("valid CLI arguments should parse");
        assert!(args.auto);
        assert!(args.dry_run);
        assert!(args.staged);
        assert!(args.help);
        assert!(args.version);
    }

    #[test]
    fn args_auto_and_version_together() {
        let args = super::parse_args(["-a", "-v"]).expect("valid CLI arguments should parse");
        assert!(args.auto);
        assert!(args.version);
        assert!(!args.help);
    }

    #[test]
    fn args_order_does_not_matter() {
        assert_eq!(
            super::parse_args(["-a", "-v", "-h"]).expect("combined flags should parse"),
            super::parse_args(["-h", "-v", "-a"]).expect("reordered flags should parse")
        );
    }

    #[test]
    fn version_is_non_empty_and_matches_cargo_manifest() {
        assert!(!VERSION.is_empty());
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn args_unknown_flag_errors_with_usage() {
        let err = super::parse_args(["--nope"])
            .expect_err("unsupported CLI arguments should be rejected")
            .to_string();
        assert!(err.contains("unknown argument '--nope'"));
        assert!(err.contains("Usage: git-cmt-rs"));
    }

    #[test]
    fn args_unknown_flag_after_valid_flags_still_errors() {
        let err = super::parse_args(["-a", "-v", "-x"])
            .expect_err("unsupported CLI arguments should be rejected")
            .to_string();
        assert!(err.contains("unknown argument '-x'"));
    }

    #[test]
    fn args_are_case_sensitive() {
        assert!(super::parse_args(["-A"]).is_err());
        assert!(super::parse_args(["--AUTO"]).is_err());
        assert!(super::parse_args(["-V"]).is_err());
    }

    #[test]
    fn args_reject_empty_and_bare_dash_arguments() {
        assert!(super::parse_args([""]).is_err());
        assert!(super::parse_args(["-"]).is_err());
        assert!(super::parse_args(["--"]).is_err());
    }

    #[test]
    fn args_reject_clustered_short_flags() {
        // `-av` is not expanded into `-a -v`; it is rejected rather than
        // silently ignored.
        assert!(super::parse_args(["-av"]).is_err());
    }

    #[test]
    fn args_reject_inline_flag_values() {
        assert!(super::parse_args(["--auto=true"]).is_err());
    }

    // ---------- early_exit_message ----------

    #[test]
    fn early_exit_message_none_for_normal_run() {
        assert!(super::early_exit_message(&Args::default()).is_none());
        assert!(
            super::early_exit_message(&super::parse_args(["-a"]).expect("auto flag should parse"))
                .is_none()
        );
    }

    #[test]
    fn early_exit_message_prints_usage_for_help() {
        let out =
            super::early_exit_message(&super::parse_args(["-h"]).expect("help flag should parse"))
                .expect("help flag should produce usage");
        assert_eq!(out, USAGE);
    }

    #[test]
    fn early_exit_message_prints_version_for_version_flag() {
        let out = super::early_exit_message(
            &super::parse_args(["-v"]).expect("version flag should parse"),
        )
        .expect("version flag should produce version text");
        assert_eq!(out, format!("git-cmt-rs {VERSION}"));
        assert!(out.contains(VERSION));
    }

    #[test]
    fn early_exit_message_help_wins_over_version() {
        let out = super::early_exit_message(
            &super::parse_args(["-v", "-h"]).expect("combined flags should parse"),
        )
        .expect("help should take precedence over version");
        assert_eq!(out, USAGE);
    }

    #[test]
    fn early_exit_message_short_circuits_auto_run() {
        // `-a -v` must not start committing; the version output takes over.
        let out = super::early_exit_message(
            &super::parse_args(["-a", "-v"]).expect("combined flags should parse"),
        )
        .expect("version should short-circuit auto mode");
        assert_eq!(out, format!("git-cmt-rs {VERSION}"));
    }

    #[test]
    fn args_positional_value_is_rejected() {
        let err = super::parse_args(["main"])
            .expect_err("unsupported CLI arguments should be rejected")
            .to_string();
        assert!(err.contains("unknown argument 'main'"));
    }

    #[test]
    fn usage_documents_all_flags() {
        assert!(USAGE.contains("-a, --auto"));
        assert!(USAGE.contains("-v, --version"));
        assert!(USAGE.contains("-h, --help"));
    }
}
