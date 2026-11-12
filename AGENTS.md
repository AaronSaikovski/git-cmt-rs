# Repository Guidelines

## Project Overview

`git-cmt-rs` generates Conventional Commit messages from staged Git changes using an OpenAI-compatible Chat Completions API. It stages changes, offers editor review, creates a commit, and optionally pushes.

## Architecture & Data Flow

The binary uses small, cohesive modules: `cli` owns arguments and terminal interaction, `commit` owns parsing and formatting, `llm` owns HTTP integration, and `git` owns subprocess operations. `src/main.rs` orchestrates the workflow; `llm` depends on `commit`, while the other modules are independent.

Flow: `parse_args` → help/version early exit → `stage_all_changes` → `get_staged_changes` → `generate_message` → `parse_commit` → `build_commit_line` → `git commit` → push confirmation → `git push`.

- `git add .` runs in the invocation directory: it stages that subtree, including untracked files, and can overwrite partial staging. Failures do not roll back staging or commits.
- `git diff --cached -b` supplies at most 3,072 Unicode characters plus a truncation marker. The diff is sent to the configured API; the default is hosted OpenAI.
- Normal commits use `git commit -e -m`; `--auto` omits editor review and pushes without confirmation. Without an upstream, pushing uses `--set-upstream origin <branch>`.
- Tokio drives the async entry point and Reqwest HTTP call; Git subprocesses and stdin remain synchronous. State is local to one invocation, with no persistent application state or dependency-injection framework.

## Key Directories

- `src/`: production modules with unit tests colocated in their owning modules.
- `tests/`: binary-level CLI integration tests.
- `scripts/`: optional Bash launcher for local backends.
- `.github/workflows/`: cross-platform CI and tag-triggered release packaging.

## Development Commands

```bash
rustup update stable
cargo build --release --locked
cargo test --all --locked --no-default-features
cargo test parse_commit                  # focused parser regressions
cargo test --test cli                    # CLI early-exit contracts
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo audit --deny warnings             # requires cargo-audit
cargo run -- --help                      # non-mutating smoke check
cargo run                               # stages, generates, commits, may push
```

Use help/version for smoke checks. Run the actual commit flow only in an intended working repository with explicit authorization for its Git and network effects.

## Code Conventions & Common Patterns

- Follow rustfmt, `snake_case` functions/fields, `PascalCase` types, and uppercase constants. Keep pure parsing/formatting helpers separate from Git, HTTP, and terminal I/O.
- Keep module interfaces `pub(crate)` and implementation helpers private. Use explicit module-qualified calls in orchestration; avoid service traits or shared mutable state for this one-shot workflow.
- Use `anyhow::Result`, `.context(...)`, and `anyhow!` for contextual errors. Diagnostics and prompts use stderr; help/version use stdout. Invalid arguments exit 2.
- Serde handles request/response DTOs. `Commit.r#type` maps to JSON `type`; commit fields are default-empty strings, not a validated type enum. `ResponseFormat` uses a `type` tag and snake-case variants.
- Preserve tolerant parsing: direct JSON → generic JSON coercion → balanced embedded JSON extraction. Supported variants include fenced/prose-wrapped output, decorated keys, arrays of candidate objects, and alternating key/value arrays. The scanner respects strings and escapes.
- Conventional Commit types and the 50-character message limit are prompt/schema constraints, not local validation. `build_commit_line` trims fields and omits empty scope.
- Preserve the pure-helper seams for tests; environment reads and concrete subprocess/HTTP calls belong at I/O boundaries.

## Important Files

- `src/main.rs`: module declarations and CLI workflow orchestration.
- `src/cli.rs`: arguments, `USAGE`, help/version output, and push confirmation. `VERSION` comes from `CARGO_PKG_VERSION`.
- `src/commit.rs`: `Commit`, tolerant JSON parsing, and Conventional Commit formatting.
- `src/llm.rs`: backend environment configuration, request schema, HTTP serialization, and message generation.
- `src/git.rs`: staging, staged-diff truncation, commit creation, and upstream-aware pushing.
- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`: package/dependencies, resolved versions, and development toolchain. Update the lockfile through Cargo, intentionally.
- `tests/cli.rs`: informational flags and argument-error process contracts.
- `scripts/git-cmt`: backend defaults and executable override. Preserve `exec "${GIT_CMT_RS_BIN:-git-cmt-rs}" "$@"` so flags and exit status survive the wrapper.
- `README.md`: installation, backend configuration, CLI usage, and troubleshooting.
- `.github/workflows/rust-ci.yml`: Ubuntu/macOS/Windows format, lint, build, and tests; Linux security audit.
- `.github/workflows/release.yml`: `v*` tags build six platform targets and publish archives with SHA256 checksums. Release packaging does not itself run tests or lint.

## Runtime/Tooling Preferences

Use Cargo and rustup: Rust edition 2024, minimum Rust 1.100. `rust-toolchain.toml` tracks floating `stable` with Clippy and rustfmt, matching CI. Git must be on `PATH`; the optional wrapper requires Bash.

The binary reads `OPENAI_BASE_URL` (default `https://api.openai.com/v1`), `OPENAI_MODEL` (default `gpt-4.1-mini`), and optional `OPENAI_API_KEY`. Empty/unset keys omit authorization. Supply the base URL through `/v1` without a trailing slash; the code appends `/chat/completions`.

`OPENAI_RESPONSE_FORMAT` accepts `json_object` (default), `json_schema`, or `none` (omit the field). Backend support varies. The tested LM Studio setup at `http://localhost:1235/v1` with `qwen/qwen3.6-27b` requires `none` for usable output: `json_object` was rejected, while schema mode returned empty `content`. The app reads `content`, not `reasoning_content`.

The wrapper defaults to `http://localhost:1235/v1`, `qwen/qwen3.6-27b`, and `OPENAI_RESPONSE_FORMAT=none` for LM Studio when the corresponding variables are unset or empty; nonempty overrides win. It leaves `OPENAI_API_KEY` unchanged. Keep wrapper defaults distinct from binary defaults.

## Testing & QA

Use Rust's built-in `#[test]` framework. Colocate tests in `src/cli.rs` for argument handling/early exits, `src/commit.rs` for formatting/deserialization/tolerant parsing, and `src/llm.rs` for response formats, wire serialization, and API-key header handling. Add concrete malformed-output regressions alongside existing parser cases. Request tests build production HTTP requests without sending them or changing environment variables.

`tests/cli.rs` launches the compiled binary from scratch directories outside Git repositories. Extend it when adding flags that must bypass staging/network access; assert exit status and stdout/stderr contracts. Git is required, but no live model server is needed. Current scratch paths are fixed per test, so concurrent suite processes can collide.

Successful Git mutation, HTTP/provider interaction, and interactive stdin paths are not covered by automated tests. Keep ordinary tests deterministic and independent of credentials or servers. Use an explicitly isolated repository for authorized end-to-end checks. CI enforces warning-free Clippy and formatting; preserve portability across Linux, macOS, and Windows.
