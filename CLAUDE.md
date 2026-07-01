# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

git-cmt-rs is an AI-powered Git commit message generator that stages changes, sends diffs to OpenAI, and creates Conventional Commit messages. It uses `git commit -e` for editor review and optionally pushes after user confirmation.

## Build & Development Commands

```bash
# Build
cargo build --release

# Run (requires OPENAI_API_KEY env var)
cargo run

# Lint (CI enforces deny warnings)
cargo clippy -- -D warnings

# Format check
cargo fmt --check

# Format
cargo fmt

# Security audit
cargo audit

# Test
cargo test
```

Tests live in `#[cfg(test)] mod tests` at the bottom of `src/main.rs` and
cover the pure functions (`build_response_format`, `build_commit_line`,
`Commit` deserialization, `parse_commit` tolerant parsing, and
`ResponseFormat` wire-format serialization).
Network, git, and stdin paths are deliberately untested — the CLI is a
one-shot orchestrator over real subprocesses.

## Architecture

The entire application lives in **src/main.rs** as a single-file design.

### Flow

`stage_all_changes()` → `get_staged_changes()` → `generate_message()` → `parse_commit()` → `build_commit_line()` → `git commit -e` → `confirm_push()` → `git push`

### Key Components

- **Domain types**: `Commit` struct with `r#type` (Conventional Commit types enum), `scope` (optional), `message`
- **Git operations** (sync): `stage_all_changes()` runs `git add .`; `get_staged_changes()` runs `git diff --cached -b` and truncates to 3072 chars
- **OpenAI integration** (async via reqwest): `generate_message()` sends the diff with a configurable `response_format` (defaults to `json_object`); temperature=0.0; the `Authorization` header is omitted when `OPENAI_API_KEY` is empty/unset so local backends work
- **Tolerant parsing**: `parse_commit()` parses raw model output, falling back to `extract_json_object()` (a string/escape-aware balanced-brace scan) so fenced or prose-wrapped JSON from local models still parses
- **User interaction**: `confirm_push()` reads stdin for y/n; commit uses `-e` flag for editor review

### Environment Variables

- `OPENAI_API_KEY` (required for hosted OpenAI; optional for Ollama/MLX/most local proxies)
- `OPENAI_MODEL` (default: `gpt-4.1-mini`)
- `OPENAI_BASE_URL` (default: `https://api.openai.com/v1`)
- `OPENAI_RESPONSE_FORMAT` (`json_object` default, `json_schema` for strict hosted-OpenAI outputs, or `none`)

## CI/CD

- **rust-ci.yml**: Multi-platform (Ubuntu/macOS/Windows) — runs fmt check, clippy, release build, cargo-audit
- **release.yml**: Tag-triggered (v*) — builds all platforms, creates tarballs/zips with SHA256 checksums, publishes to GitHub Releases

## Rust Edition

The project uses Rust edition **2024**. This affects keyword reservations and language features.


## Workflow Orchestration

### 1. Plan Node Default

- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan immediately – don't keep pushing
- Use plan mode for verification steps, not just building
- Write detailed specs upfront to reduce ambiguity

### 2. Subagent Strategy

- Use subagents liberally to keep main context window clean
- Offload research, exploration, and parallel analysis to subagents
- For complex problems, throw more compute at it via subagents
- One task per subagent for focused execution

### 3. Self-Improvement Loop

- After ANY correction from the user: update `tasks/lessons.md` with the pattern
- Write rules for yourself that prevent the same mistake
- Ruthlessly iterate on these lessons until mistake rate drops
- Review lessons at session start for relevant project

### 4. Verification Before Done

- Never mark a task complete without proving it works
- Diff behavior between main and your changes when relevant
- Ask yourself: "Would a staff engineer approve this?"
- Run tests, check logs, demonstrate correctness

### 5. Demand Elegance (Balanced)

- For non-trivial changes: pause and ask "is there a more elegant way?"
- If a fix feels hacky: "Knowing everything I know now, implement the elegant solution"
- Skip this for simple, obvious fixes – don't over-engineer
- Challenge your own work before presenting it

### 6. Autonomous Bug Fixing

- When given a bug report: just fix it. Don't ask for hand-holding
- Point at logs, errors, failing tests – then resolve them
- Zero context switching required from the user
- Go fix failing CI tests without being told how

## Task Management

1. **Plan First**: Write plan to `tasks/todo.md` with checkable items
2. **Verify Plan**: Check in before starting implementation
3. **Track Progress**: Mark items complete as you go
4. **Explain Changes**: High-level summary at each step
5. **Document Results**: Add review section to `tasks/todo.md`
6. **Capture Lessons**: Update `tasks/lessons.md` after corrections

## Core Principles

- **Simplicity First**: Make every change as simple as possible. Impact minimal code.
- **No Laziness**: Find root causes. No temporary fixes. Senior developer standards.
- **Minimal Impact**: Changes should only touch what's necessary. Avoid introducing bugs.
