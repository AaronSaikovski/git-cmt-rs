mod cli;
mod commit;
mod git;
mod llm;

use crate::git::{CommitReview, DiffMode};

use anyhow::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args = match cli::parse_args(env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    if let Some(message) = cli::early_exit_message(&args) {
        println!("{message}");
        return Ok(());
    }

    if !args.staged && !args.dry_run {
        match git::stage_all_changes() {
            Ok(_) => eprintln!("Staged all changes with `git add .`"),
            Err(e) => {
                eprintln!("Failed to stage changes: {e}");
                std::process::exit(1);
            }
        };
    }

    let diff_mode = if args.dry_run {
        DiffMode::ReadOnly
    } else {
        DiffMode::Standard
    };
    let changes = match git::get_staged_changes(diff_mode) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to get staged changes: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("Staged diff found; generating message for changes...");

    let mut commit = match llm::generate_message(&changes).await {
        Ok(c) => c,
        Err(e) => {
            let mut message = e.to_string();
            cli::sanitize_terminal(&mut message);
            eprintln!("Failed to generate commit message: {message}");
            std::process::exit(1);
        }
    };

    // Sanitize once before diagnostics, dry-run output, editor review, or Git output.
    cli::sanitize_terminal(&mut commit.r#type);
    cli::sanitize_terminal(&mut commit.scope);
    cli::sanitize_terminal(&mut commit.message);

    eprintln!(
        "Parsed commit: type='{}', scope='{}', message='{}'",
        commit.r#type, commit.scope, commit.message
    );

    let line = commit::build_commit_line(&commit);

    if args.dry_run {
        println!("{line}");
        return Ok(());
    }

    let review = if args.auto {
        CommitReview::AcceptGenerated
    } else {
        CommitReview::Editor
    };
    git::create_commit(&line, review)?;

    eprintln!("Commit created successfully.");

    // Ask for confirmation before pushing (skipped with --auto)
    let should_push = if args.auto {
        eprintln!("Auto mode: pushing without confirmation.");
        true
    } else {
        match cli::confirm_push() {
            Ok(confirmed) => confirmed,
            Err(e) => {
                eprintln!("Error during push confirmation: {e}");
                std::process::exit(1);
            }
        }
    };

    if !should_push {
        eprintln!("Push cancelled. Commit saved locally.");
        return Ok(());
    }

    git::push()?;

    eprintln!("Changes pushed successfully!");

    Ok(())
}
