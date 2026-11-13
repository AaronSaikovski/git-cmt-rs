# git-cmt-rs

An AI-powered Git commit message generator written in Rust that automatically stages your changes and creates [Conventional Commit](https://www.conventionalcommits.org/) messages. Supports [LM Studio](https://lmstudio.ai/), hosted OpenAI, [Ollama](https://ollama.com/), and other OpenAI-compatible endpoints.

## Overview

`git-cmt-rs` stages changes with `git add .` by default, analyzes the staged diff, and generates a commit message using the configured chat-completions backend. Use `--staged` to preserve your staging selection, or `--dry-run` to preview a message without changing the repository. The optional `git-cmt` wrapper defaults to LM Studio; the bare binary defaults to hosted OpenAI. Both provide editor review and push confirmation.

Feel free to tweak the code to try different models, providers, or prompt templates. The implementation is simple and hackable.

This project is a Rust implementation of the Go based git-cmd project found [here](https://github.com/appliedgocode/git-cmt/)

#### All respects and credits to the original authors.

## Features

- 🤖 **AI-powered**: Works with hosted OpenAI (default: `gpt-4.1-mini`), local Ollama, or any OpenAI-compatible proxy
- 🔌 **Backend-agnostic**: Configure the endpoint, model, and response format without code changes
- 📝 **Conventional Commits**: Generates messages in the `type(scope): description` format
- 🎯 **Smart Analysis**: Understands code changes and suggests contextually appropriate messages
- ✅ **Push Confirmation**: Asks for y/n confirmation before pushing to remote
- ⚡ **Interactive**: Opens your editor for final review and editing before committing
- 🚀 **Auto mode**: `-a` / `--auto` skips the editor and the push prompt for a one-command commit-and-push
- 📦 **Auto-staging**: Automatically stages all changes with `git add .` before analysis
- 🔍 **Diff-aware**: Analyzes changes to generate contextually appropriate messages
- 📏 **Length-aware**: Prompts for descriptions of at most 50 characters; local parsing does not enforce this limit

## Installation

### Prerequisites

- Rust 1.100.0 or newer, installed through [rustup](https://rustup.rs/)
- Git
- A running LM Studio, Ollama, or other compatible server, or an OpenAI API key

### Build from source

```bash
git clone https://github.com/AaronSaikovski/git-cmt-rs
cd git-cmt-rs
rustup update stable
cargo build --release --locked
```

`rust-toolchain.toml` selects stable Rust with Clippy and rustfmt, matching CI
and release builds. Run `rustup update stable` to install the latest stable release.

### Install globally

```bash
# Copy the binary into your PATH, retaining the local build
sudo install -m 755 target/release/git-cmt-rs /usr/local/bin/git-cmt-rs
```

### Optional: `git-cmt` wrapper for LM Studio

`scripts/git-cmt` defaults to LM Studio at `http://localhost:1235/v1`, model
`google/gemma-4-12b`, and `OPENAI_RESPONSE_FORMAT=none`. Load that model in LM Studio
or override `OPENAI_MODEL` with a loaded model identifier. Install or refresh the
wrapper after installing the binary:

```bash
sudo install -m 755 scripts/git-cmt /usr/local/bin/git-cmt
```

It forwards all arguments with `exec "${GIT_CMT_RS_BIN:-git-cmt-rs}" "$@"`, so `git-cmt -a`,
`git-cmt -v` and `git-cmt -h` behave identically to the bare binary and the exit
code propagates unchanged.

> **If you hand-rolled a wrapper, check it ends in `git-cmt-rs "$@"`.** A wrapper
> whose last line is a bare `git-cmt-rs` silently discards every flag you pass —
> `git-cmt -v` then runs the default commit flow instead of printing the version.

Defaults apply when variables are unset or empty; nonempty environment overrides
win. The wrapper leaves `OPENAI_API_KEY` unchanged. Unset it when LM Studio
authentication is disabled, or supply the server's token when authentication is
enabled.

```bash
# Use the wrapper's LM Studio defaults (interactive commit review)
git-cmt

# Override the server address
OPENAI_BASE_URL=http://localhost:1234/v1 git-cmt

# Test a local build with a specific bin path
GIT_CMT_RS_BIN=./target/release/git-cmt-rs scripts/git-cmt -v

# Check the installed wrapper without staging or committing
git-cmt --version
```

## Usage

### Setup

#### Hosted OpenAI

1. Set your OpenAI API key:
   ```bash
   export OPENAI_API_KEY="your-api-key-here"
   ```
   In PowerShell:
   ```powershell
   $env:OPENAI_API_KEY = "your-api-key-here"
   ```
2. Set the model and base URL explicitly when switching from a local backend:
   ```bash
   export OPENAI_MODEL="gpt-4.1-mini"
   export OPENAI_BASE_URL="https://api.openai.com/v1"
   ```
3. (Optional) For strict JSON Schema enforcement:
   ```bash
   export OPENAI_RESPONSE_FORMAT="json_schema"
   ```

#### Local Ollama instance

`git-cmt-rs` speaks the OpenAI HTTP shape, so it works against Ollama's
OpenAI-compatible endpoint with no code changes:

```bash
# Start Ollama and pull a code-capable model
ollama serve &
ollama pull qwen2.5-coder

# Point git-cmt-rs at the local endpoint
export OPENAI_BASE_URL="http://localhost:11434/v1"
export OPENAI_MODEL="qwen2.5-coder"   # or llama3.2, mistral, codellama, ...
export OPENAI_RESPONSE_FORMAT="json_object"
unset OPENAI_API_KEY                   # Ollama ignores auth
```

Set the response format explicitly when switching from LM Studio so an earlier
`none` setting does not carry over. Response-format support depends on the
backend version and model.

#### Local LM Studio

Start LM Studio's local server and load a chat model. These settings match the
wrapper defaults on port `1235`:

```bash
export OPENAI_BASE_URL="http://localhost:1235/v1"
export OPENAI_MODEL="google/gemma-4-12b"  # use your loaded model's identifier
export OPENAI_RESPONSE_FORMAT="none"
unset OPENAI_API_KEY
```

An API key is not needed when server authentication is disabled. If you enable
authentication in LM Studio, set `OPENAI_API_KEY` to the server's token.
Unset or empty keys omit the `Authorization` header entirely.

Use `curl http://localhost:1235/v1/models` to list model identifiers when
authentication is disabled. These exports also override the `scripts/git-cmt`
wrapper's backend defaults.

The tested server rejects the default `json_object` format with
`'response_format.type' must be 'json_schema' or 'text'`. Setting `none` omits
the field; the prompt still requests JSON. In an earlier check with
`qwen/qwen3.6-27b`, `json_schema` returned empty `content` and put the answer in
`reasoning_content`, which this app does not read; `none` returned usable JSON.
That observation is specific to the tested Qwen setup, not a claim about Gemma.

Run `git-cmt-rs --help` for a non-mutating smoke check. Run `git-cmt-rs` only
when ready to stage changes and create a commit; `--auto` also pushes without
confirmation.

#### Local MLX server (Apple Silicon)

Apple's [MLX](https://github.com/ml-explore/mlx) (`mlx_lm.server`, and wrappers
such as omlx) also expose an OpenAI-compatible endpoint:

```bash
# Start the MLX server (example port; use whatever yours serves)
mlx_lm.server --model Qwen3-14B-MLX-4bit --port 11436

# Point git-cmt-rs at it
export OPENAI_BASE_URL="http://127.0.0.1:11436/v1"
export OPENAI_MODEL="Qwen3-14B-MLX-4bit"   # must match the loaded model
unset OPENAI_API_KEY                        # local server ignores auth
```

MLX enforces `response_format` less strictly than Ollama, so models may return
JSON wrapped in prose or markdown fences — `git-cmt-rs` tolerates this (see
[Robust JSON parsing](#robust-json-parsing)). If your MLX build rejects
`response_format`, set `OPENAI_RESPONSE_FORMAT=none`.

> **Qwen3 note:** Qwen3 emits `<think>…</think>` reasoning before its answer by
> default. The parser skips it, but you can suppress it for faster, cleaner
> output by appending `/no_think` to your prompt or disabling thinking in the
> server's chat template.

#### Local OpenAI-compatible proxy (LiteLLM, LocalAI, vLLM, ...)

```bash
export OPENAI_BASE_URL="http://localhost:4000/v1"   # whatever your proxy serves
export OPENAI_MODEL="your-model-name"
# Set OPENAI_API_KEY only if your proxy requires it; otherwise leave it unset.
```

If your proxy supports strict JSON Schema, you can opt in with
`OPENAI_RESPONSE_FORMAT=json_schema`. If it rejects `response_format`
entirely, use `OPENAI_RESPONSE_FORMAT=none`.

### Robust JSON parsing

Local models don't always honor `response_format` and may wrap their JSON in
markdown code fences, prefix it with a `json` label, or emit reasoning text
first (e.g. Qwen3's `<think>` blocks, Gemma's fenced output). Some even return
the fields in the wrong shape — an object with decorated keys like `_type`, a
flattened `["_type", "chore", "_scope", "deps", ...]` array instead of an
object, or an array of one commit object per changed file. Rather than failing,
`git-cmt-rs` first tries to parse the raw response, then coerces generic JSON
into a commit — normalizing decorated keys, pairing up flattened key/value
arrays, and taking the first entry from an array of commit objects. As a last
resort it extracts the first balanced
`{ ... }` object or `[ ... ]` array embedded in the text. The scan is string-
and escape-aware, so braces inside the commit message don't throw it off. In
practice this means the tool works with a wide range of local models regardless
of how chatty or malformed their formatting is.

### Basic Usage

1. Run the tool from the repository root to stage changes across the repository:
   ```bash
   git-cmt-rs
   ```
2. The editor opens for final review and editing of the commit message.
3. Save and close the editor to create the commit.
4. After commit, confirm whether to push to remote (y/n).
5. If confirmed, changes are pushed; if declined, commit stays local.

By default, the tool runs `git add .` in your current directory before analysis.
This includes untracked files and can overwrite partial staging; use `--staged`
or `--dry-run` to skip this step. Failures do not undo staging or an
already-created commit. The staged diff is sent to the configured backend,
including in dry-run mode.

### Command-line options

```
Usage: git-cmt-rs [OPTIONS]

Options:
  -a, --auto     Accept the generated message without opening the editor and
                 push without prompting for confirmation
      --dry-run  Print a message for staged changes without staging, committing,
                 or pushing (overrides --auto; still calls the model)
      --staged   Use only staged changes; do not run git add
  -v, --version  Print the version and exit
  -h, --help     Print this help message
```

The version is read from `Cargo.toml` at build time:

```bash
$ git-cmt-rs -v
git-cmt-rs 1.0.5
```

### Per-commit staging and previews

These flags apply only to the current invocation; no configuration is saved.
Select the changes yourself, then choose whether to preview or commit:

```bash
git add -p
git-cmt-rs --dry-run  # Print the generated message without changing the repository
git-cmt-rs --staged   # Review and commit only the staged changes
```

`--staged` skips `git add .`, preserving unstaged hunks and untracked files.
Editor review and push confirmation still apply; combine it with `--auto`
to commit the staged changes and push without either prompt.

`--dry-run` always uses only the existing staged diff, even without `--staged`.
It prints the generated commit message to stdout and diagnostics to stderr,
then exits without opening an editor, committing, prompting, or pushing.
It overrides `--auto`; combining it with `--staged` is also supported.
Dry-run disables Git text-conversion and external diff helpers to prevent their
side effects, so custom-rendered or binary-file previews may differ from commits.
It still sends the staged diff to the configured model, which may be hosted.
Neither flag generates a message when there are no staged changes.
Previewing and committing are separate model calls; the message may differ.
The `git-cmt` wrapper forwards both flags unchanged.

### Auto mode

Pass `-a` (or `--auto`) to commit and push in a single step — no editor to save
and quit, no `y` to type at the push prompt:

```bash
git-cmt-rs -a
```

This runs `git commit -m "<generated message>"` (without `-e`) and then pushes
immediately. Use it when you trust the generated message; omit it when you want
to review or edit before committing.

### Safety and resource limits

- HTTP requests have a 10-second connection timeout and a 120-second total
  deadline, including response-body reads. These are fixed application limits;
  slower local models must finish within that deadline.
- Success and error responses are limited to 1 MiB of decoded body data,
  including compressed, chunked, and lengthless responses. Oversized responses
  fail before committing; oversized error bodies are truncated in diagnostics.
- Model content also has a 1 MiB parser limit. Embedded JSON extraction runs
  in linear time while preserving tolerant recovery from prose, fences, and
  stray delimiters.
- Git diff output is drained using bounded buffers. Only the first 3,072 Unicode
  characters are retained, plus a truncation marker when needed. Invalid UTF-8
  anywhere in the diff is still rejected.
- Generated fields escape terminal controls and Unicode direction overrides
  before diagnostics, previews, editor review, or commit creation. For example,
  an escape character becomes literal `\u{1b}` and a newline becomes literal
  `\n`; ordinary Unicode text remains unchanged. Provider-error diagnostics
  are escaped too.
- EOF at push confirmation cancels the push, leaving the commit local.
  `--auto` still pushes without reading stdin.

These limits do not detect secrets. Review staged changes before sending them
to a hosted backend.

## How it works

1. **Auto-staging**: Runs `git add .` in the current directory unless `--staged` or `--dry-run` is passed
2. **Diff Analysis**: Reads staged changes with `git diff --cached -b`, limiting the input to 3,072 Unicode characters plus a truncation marker
3. **AI Processing**: Sends the diff to the configured backend. The bare binary defaults to `json_object`; the LM Studio wrapper defaults to `none`. Use `json_schema` only where supported by the backend and model.
4. **Message Generation**: Produces a commit object with `type`, `scope`, and `message`, tolerating fenced or prose-wrapped JSON from local models (see [Robust JSON parsing](#robust-json-parsing)). With `--dry-run`, prints the formatted message and exits here.
5. **Interactive Commit**: Opens your editor with the message for final review and editing (skipped with `-a`)
6. **Create Commit**: Runs `git commit` with the approved message
7. **Push Confirmation**: Asks user to confirm push to remote (y/n) (skipped with `-a`, which always pushes)
8. **Final Push**: Runs `git push` if confirmed, or exits with commit saved locally if declined

## Commit Message Format

```
type(scope): description
```

- **Types**: feat, fix, docs, style, refactor, test, chore
- **Scope**: Optional component/module name
- **Description**: Clear, concise summary (the prompt requests a maximum of 50 characters)

## Examples

### Feature Addition (Push Confirmed)

```bash
$ git-cmt-rs
Staged all changes with `git add .`
Staged diff found; generating message for changes...
Parsed commit: type='feat', scope='auth', message='add OAuth2 login integration'

# Opens editor for final review
# Save and close editor to commit

Commit created successfully.
Push commit to remote? (y/n): y
Changes pushed successfully!
```

### Bug Fix (Push Declined)

```bash
$ git-cmt-rs
Staged all changes with `git add .`
Staged diff found; generating message for changes...
Parsed commit: type='fix', scope='api', message='resolve null pointer in validation'

# Opens editor for final review
# Save and close editor to commit

Commit created successfully.
Push commit to remote? (y/n): n
Push cancelled. Commit saved locally.
```

### Auto Mode (No Prompts)

```bash
$ git-cmt-rs -a
Staged all changes with `git add .`
Staged diff found; generating message for changes...
Parsed commit: type='chore', scope='deps', message='bump reqwest to 0.12.9'
Commit created successfully.
Auto mode: pushing without confirmation.
Changes pushed successfully!
```

### Keeping Commit Local

Users can respond with `n` or `no`, or close stdin (EOF), at the push confirmation to keep the commit local without pushing to remote:

```bash
$ git-cmt-rs
Staged all changes with `git add .`
...
Commit created successfully.
Push commit to remote? (y/n): n
Push cancelled. Commit saved locally.
```

## Configuration

### Environment Variables

The defaults below apply to the **bare binary**. See the
[LM Studio wrapper](#optional-git-cmt-wrapper-for-lm-studio) for its different
endpoint, model, and response-format defaults.

- `OPENAI_API_KEY` – API key (required for hosted OpenAI; optional for LM Studio,
  Ollama, and other local servers unless authentication is enabled)
- `OPENAI_MODEL` – model to use (default: `gpt-4.1-mini`)
- `OPENAI_BASE_URL` – API endpoint (default: `https://api.openai.com/v1`)
- `OPENAI_RESPONSE_FORMAT` – one of:
  - `json_object` (default) – broad compatibility (OpenAI, Ollama, most proxies)
  - `json_schema` – strict structured outputs where supported by the backend/model
  - `none` – omit `response_format` entirely (recommended for the LM Studio setup above)
- `EDITOR` – editor for reviewing commits (defaults to system default)

## Error Handling

- **Failed to stage changes** → exits if `git add .` fails
- **No staged changes** → exits with helpful message if no changes exist
- **Missing API key** → only an issue when the configured backend requires one; against hosted OpenAI you'll see a 401 with the API's response body
- **Invalid `OPENAI_RESPONSE_FORMAT`** → exits with the list of valid values (`json_object`, `json_schema`, `none`)
- **API failures** → shows HTTP status and a bounded, terminal-safe response body
- **HTTP timeout** → exits without committing when connecting or reading exceeds the deadline
- **Oversized response/model content** → rejects input above the 1 MiB limit before committing
- **Invalid JSON** → shows escaped model output for debugging
- **Commit creation failed** → exits with error message if `git commit` fails
- **Push declined or EOF** → exits gracefully with "Push cancelled. Commit saved locally." when user responds with `n` or `no`, or stdin closes
- **Push failed** → shows error if `git push` fails (commit is already saved locally)
- **Invalid push confirmation input** → prompts user to answer `y/n` again

## Development

### Dependencies

- [`reqwest`](https://docs.rs/reqwest/) – HTTP client
- [`serde` / `serde_json`](https://serde.rs/) – JSON parsing
- [`tokio`](https://tokio.rs/) – async runtime
- [`anyhow`](https://docs.rs/anyhow/) – error handling

### Project Structure

```
├── src/
│   ├── main.rs      # Workflow orchestration
│   ├── cli.rs       # Arguments, help/version, and push confirmation
│   ├── commit.rs    # Commit data, tolerant parsing, and formatting
│   ├── llm.rs       # Backend configuration and HTTP requests/responses
│   └── git.rs       # Staging, diffs, commits, and pushes
├── tests/
│   ├── cli.rs       # Early exits and dry-run diff-helper safety
│   └── workflow.rs  # Isolated Git/API workflow regressions
├── scripts/git-cmt  # Optional local-backend launcher
├── Cargo.toml      # Dependencies and metadata
├── rust-toolchain.toml
└── README.md
```

`main` coordinates the flow; `llm` uses `commit` to parse model output.
Git subprocesses and terminal interaction stay in their respective modules.
Cross-module interfaces are crate-visible, while implementation details remain
private. Unit tests live alongside the code they exercise.

### Rust style

Follow the general application-code rules in the
[Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html),
alongside this repository's conventions:

- Keep the four focused modules and small crate-visible interfaces; keep helpers
  private and orchestration in `main`.
- Import items from one module per `use` statement, import types by name, and
  qualify cross-module function calls.
- Prefer borrowed data, explicit mode enums instead of ambiguous Boolean
  parameters, contextual errors, and descriptive `expect` messages for test
  invariants.
- Use stable Rust, rustfmt's 100-column default, and warning-free Clippy.

Chromium/Fuchsia-specific build layouts, concurrency restrictions, and test
frameworks do not apply here. This project retains Cargo, Tokio, and Rust's
built-in tests; unstable rustfmt options are not required.

### Building

```bash
cargo build
```

### Testing and linting

```bash
cargo test --all --locked --no-default-features
cargo test backend_request              # authentication and LM Studio request regressions
cargo test --test cli                   # early exits and dry-run diff-helper safety
cargo test --test workflow              # staging, terminal safety, and EOF cancellation
cargo test llm::http_tests              # HTTP deadlines and decoded-body limits
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Unit tests alongside all four modules cover CLI parsing, EOF confirmation,
terminal escaping, tolerant commit parsing, linear extraction edge cases,
streamed UTF-8 diff truncation, and HTTP request/response behavior. Request
serialization tests verify authorization handling and response-format
compatibility without sending requests. HTTP tests use ephemeral loopback
servers to exercise stalled headers/bodies, response-size boundaries, chunked
and lengthless bodies, and compressed responses.

`tests/cli.rs` verifies informational exits outside Git repositories and checks
that dry-run does not invoke diff helpers or write textconv cache refs.
`tests/workflow.rs` uses disposable repositories and a loopback API to verify
partial staging, empty indexes, terminal-safe previews and errors, and local
commit creation with push cancellation on EOF. Test scratch directories are
isolated across concurrent suite processes.

Git must be on `PATH`; tests require no API keys or running model servers.
Live provider behavior, model accuracy, and successful remote pushes are not
covered by these tests.

### CI and releases

`.github/workflows/rust-ci.yml` runs formatting, warning-free Clippy, locked
builds/tests, and wrapper help/version smoke checks on Linux, macOS, and Windows.
A separate Linux job audits dependencies. Build caches distinguish the runner
architecture and installed Rust compiler version.

`.github/workflows/release.yml` builds six platform targets for `v*` tags and
publishes archives with SHA256 checksums. Each archive includes the binary,
`scripts/git-cmt`, `README.md`, and `LICENSE`; the wrapper requires Bash.
Only the publishing job receives repository write permission. Release packaging
does not itself run tests or lint.

## License

This project is open source. See the repository for details.

## Troubleshooting

### Common Issues

**"No staged changes found"**

- With `--staged` or `--dry-run`, select changes first using `git add` or `git add -p`.
- Without those flags, the invocation directory must contain changes for `git add .` to stage. Whitespace-only differences ignored by `git diff -b` do not produce a message.

**Authentication errors (HTTP 401)**

- Hosted OpenAI requires an API key. Local servers only require one when
  authentication is enabled. Set `OPENAI_API_KEY` to the server's token, or unset
  it for an unauthenticated local server.

**"LLM request failed"**

- Check that the backend is reachable, for example
  `curl http://localhost:1235/v1/models` for an unauthenticated LM Studio server.
  Wrapper defaults are scoped to the child process; they do not export variables
  back into your calling shell.
- For hosted OpenAI: verify your API key and that your account has credits.
- For LM Studio: confirm the server port and loaded model identifier. After
  changing `scripts/git-cmt`, reinstall the wrapper if using `/usr/local/bin/git-cmt`.
- For Ollama: confirm `ollama serve` is running and the model is pulled
  (`ollama list`).
- If LM Studio rejects `response_format.type`, set
  `OPENAI_RESPONSE_FORMAT=none`; see [Local LM Studio](#local-lm-studio).
- Requests must complete within 120 seconds, with at most 10 seconds to connect.
  Slow model loading or generation can exceed this deadline.
- Responses larger than 1 MiB after decompression are rejected. Reduce verbose
  model output or check whether the backend is returning a large error page.

**Editor not opening**

- Set your editor explicitly:
  ```bash
  export EDITOR="code --wait"
  ```

**"Push cancelled. Commit saved locally." message**

- This is expected after `n`, `no`, or EOF at the push confirmation prompt.
- The commit is already created and saved; the push is simply skipped.
- You can push manually later with `git push`.

**"Push failed" error**

- The commit was created successfully, but the push to remote failed (network issues, authentication, etc.)
- Your commit is safely saved locally
- You can try pushing again manually or resolve any issues before retrying
