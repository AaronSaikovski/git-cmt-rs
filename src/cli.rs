//! Command-line options and terminal interaction.

use anyhow::{Context, Result, anyhow};
use std::io::{self, Write};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
git-cmt-rs - AI-powered Conventional Commit message generator

Usage: git-cmt-rs [OPTIONS]

Options:
  -a, --auto     Accept the generated message without opening the editor and
                 push without prompting for confirmation
  -v, --version  Print the version and exit
  -h, --help     Print this help message";

// ---------- CLI ----------
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    /// Skip the editor review and the push confirmation prompt.
    pub(crate) auto: bool,
    /// Print usage and exit.
    help: bool,
    /// Print the version and exit.
    version: bool,
}

pub(crate) fn parse_args<I, S>(args: I) -> Result<Args>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = Args::default();

    for arg in args {
        match arg.as_ref() {
            "-a" | "--auto" => parsed.auto = true,
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

pub(crate) fn confirm_push() -> Result<bool> {
    loop {
        eprint!("Push commit to remote? (y/n): ");
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read user input")?;

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                eprintln!("Please answer 'y' or 'n'");
                continue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- parse_args ----------

    #[test]
    fn args_default_to_interactive() {
        let args = parse_args(Vec::<String>::new()).unwrap();
        assert_eq!(args, Args::default());
        assert!(!args.auto);
        assert!(!args.help);
        assert!(!args.version);
    }

    #[test]
    fn args_short_auto_flag() {
        assert!(parse_args(["-a"]).unwrap().auto);
    }

    #[test]
    fn args_long_auto_flag() {
        assert!(parse_args(["--auto"]).unwrap().auto);
    }

    #[test]
    fn args_help_flags() {
        assert!(parse_args(["-h"]).unwrap().help);
        assert!(parse_args(["--help"]).unwrap().help);
    }

    #[test]
    fn args_version_flags() {
        assert!(parse_args(["-v"]).unwrap().version);
        assert!(parse_args(["--version"]).unwrap().version);
    }

    #[test]
    fn args_each_flag_sets_only_its_own_field() {
        let auto = parse_args(["-a"]).unwrap();
        assert_eq!(
            auto,
            Args {
                auto: true,
                help: false,
                version: false
            }
        );

        let help = parse_args(["-h"]).unwrap();
        assert_eq!(
            help,
            Args {
                auto: false,
                help: true,
                version: false
            }
        );

        let version = parse_args(["-v"]).unwrap();
        assert_eq!(
            version,
            Args {
                auto: false,
                help: false,
                version: true
            }
        );
    }

    #[test]
    fn args_repeated_and_combined_flags() {
        let args = parse_args(["-a", "--auto", "--help", "-v"]).unwrap();
        assert!(args.auto);
        assert!(args.help);
        assert!(args.version);
    }

    #[test]
    fn args_auto_and_version_together() {
        let args = parse_args(["-a", "-v"]).unwrap();
        assert!(args.auto);
        assert!(args.version);
        assert!(!args.help);
    }

    #[test]
    fn args_order_does_not_matter() {
        assert_eq!(
            parse_args(["-a", "-v", "-h"]).unwrap(),
            parse_args(["-h", "-v", "-a"]).unwrap()
        );
    }

    #[test]
    fn version_is_non_empty_and_matches_cargo_manifest() {
        assert!(!VERSION.is_empty());
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn args_unknown_flag_errors_with_usage() {
        let err = parse_args(["--nope"]).unwrap_err().to_string();
        assert!(err.contains("unknown argument '--nope'"));
        assert!(err.contains("Usage: git-cmt-rs"));
    }

    #[test]
    fn args_unknown_flag_after_valid_flags_still_errors() {
        let err = parse_args(["-a", "-v", "-x"]).unwrap_err().to_string();
        assert!(err.contains("unknown argument '-x'"));
    }

    #[test]
    fn args_are_case_sensitive() {
        assert!(parse_args(["-A"]).is_err());
        assert!(parse_args(["--AUTO"]).is_err());
        assert!(parse_args(["-V"]).is_err());
    }

    #[test]
    fn args_reject_empty_and_bare_dash_arguments() {
        assert!(parse_args([""]).is_err());
        assert!(parse_args(["-"]).is_err());
        assert!(parse_args(["--"]).is_err());
    }

    #[test]
    fn args_reject_clustered_short_flags() {
        // `-av` is not expanded into `-a -v`; it is rejected rather than
        // silently ignored.
        assert!(parse_args(["-av"]).is_err());
    }

    #[test]
    fn args_reject_inline_flag_values() {
        assert!(parse_args(["--auto=true"]).is_err());
    }

    // ---------- early_exit_message ----------

    #[test]
    fn early_exit_message_none_for_normal_run() {
        assert!(early_exit_message(&Args::default()).is_none());
        assert!(early_exit_message(&parse_args(["-a"]).unwrap()).is_none());
    }

    #[test]
    fn early_exit_message_prints_usage_for_help() {
        let out = early_exit_message(&parse_args(["-h"]).unwrap()).unwrap();
        assert_eq!(out, USAGE);
    }

    #[test]
    fn early_exit_message_prints_version_for_version_flag() {
        let out = early_exit_message(&parse_args(["-v"]).unwrap()).unwrap();
        assert_eq!(out, format!("git-cmt-rs {VERSION}"));
        assert!(out.contains(VERSION));
    }

    #[test]
    fn early_exit_message_help_wins_over_version() {
        let out = early_exit_message(&parse_args(["-v", "-h"]).unwrap()).unwrap();
        assert_eq!(out, USAGE);
    }

    #[test]
    fn early_exit_message_short_circuits_auto_run() {
        // `-a -v` must not start committing; the version output takes over.
        let out = early_exit_message(&parse_args(["-a", "-v"]).unwrap()).unwrap();
        assert_eq!(out, format!("git-cmt-rs {VERSION}"));
    }

    #[test]
    fn args_positional_value_is_rejected() {
        let err = parse_args(["main"]).unwrap_err().to_string();
        assert!(err.contains("unknown argument 'main'"));
    }

    #[test]
    fn usage_documents_all_flags() {
        assert!(USAGE.contains("-a, --auto"));
        assert!(USAGE.contains("-v, --version"));
        assert!(USAGE.contains("-h, --help"));
    }
}
