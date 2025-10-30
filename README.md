# git-cmt-rs

An AI-powered Git commit message generator written in Rust that automatically stages your changes and creates [Conventional Commit](https://www.conventionalcommits.org/) messages using the OpenAI API.

## Overview

`git-cmt-rs` automatically stages all changes with `git add .`, analyzes the diff, and generates meaningful commit messages using OpenAI's chat completion models. It follows the Conventional Commits specification and provides an interactive commit experience with editor review.

Feel free to tweak the code to try different models, providers, or prompt templates. The implementation is simple and hackable.

This project is a Rust implementation of the Go based git-cmd project found [here](https://github.com/appliedgocode/git-cmt/)

#### All respects and credits to the original authors.

## Features

- 🤖 **AI-powered**: Uses OpenAI models (default: `gpt-4.1-mini`) to analyze code changes
- 📝 **Conventional Commits**: Generates messages in the `type(scope): description` format
- 🎯 **Smart Analysis**: Understands code changes and suggests contextually appropriate messages
- ✅ **Push Confirmation**: Asks for y/n confirmation before pushing to remote
- ⚡ **Interactive**: Opens your editor for final review and editing before committing
- 📦 **Auto-staging**: Automatically stages all changes with `git add .` before analysis
- 🔍 **Diff-aware**: Analyzes changes to generate contextually appropriate messages
- 📏 **Length-aware**: Keeps commit messages concise (50 chars max for description)

## Installation

### Prerequisites

- Rust (1.70+ recommended)
- Git
- An OpenAI API key

### Build from source

```bash
git clone https://github.com/AaronSaikovski/git-cmt-rs
cd git-cmt-rs
cargo build --release
```

### Install globally

```bash
# Move binary to a directory in your PATH
sudo mv target/release/git-cmt-rs /usr/local/bin/git-cmt-rs
```

## Usage

### Setup

1. Set your OpenAI API key:
   ```bash
   export OPENAI_API_KEY="your-api-key-here"

   or  
   $env:export OPENAI_API_KEY="your-api-key-here"

   ```
2. (Optional) Override model or base URL:
   ```bash
   export OPENAI_MODEL="gpt-4.1-mini"
   export OPENAI_BASE_URL="https://api.openai.com/v1"
   ```

### Basic Usage

1. Run the tool (stages all changes automatically):
   ```bash
   git-cmt-rs
   ```
2. The editor opens for final review and editing of the commit message.
3. Save and close the editor to create the commit.
4. After commit, confirm whether to push to remote (y/n).
5. If confirmed, changes are pushed; if declined, commit stays local.

The tool automatically stages all changes with `git add .` before analyzing and generating a commit message.

## How it works

1. **Auto-staging**: Stages all changes with `git add .`
2. **Diff Analysis**: Reads staged changes with `git diff --cached -b` (truncated to 3072 chars if necessary)
3. **AI Processing**: Sends the diff to OpenAI with structured prompts and JSON schema enforcement
4. **Message Generation**: Produces a commit object with `type`, `scope`, and `message`
5. **Interactive Commit**: Opens your editor with the message for final review and editing
6. **Create Commit**: Runs `git commit` with the approved message
7. **Push Confirmation**: Asks user to confirm push to remote (y/n)
8. **Final Push**: Runs `git push` if confirmed, or exits with commit saved locally if declined

## Commit Message Format

```
type(scope): description
```

- **Types**: feat, fix, docs, style, refactor, test, chore
- **Scope**: Optional component/module name
- **Description**: Clear, concise summary (max 50 chars)

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

### Keeping Commit Local

Users can respond with `n` or `no` at the push confirmation to keep the commit local without pushing to remote:

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

- `OPENAI_API_KEY` – required for API access
- `OPENAI_MODEL` – model to use (default: `gpt-4.1-mini`)
- `OPENAI_BASE_URL` – API endpoint (default: `https://api.openai.com/v1`)
- `EDITOR` – editor for reviewing commits (defaults to system default)

## Error Handling

- **Failed to stage changes** → exits if `git add .` fails
- **No staged changes** → exits with helpful message if no changes exist
- **Missing API key** → exits with message to set `OPENAI_API_KEY`
- **API failures** → shows HTTP status and response body
- **Invalid JSON** → shows raw model output for debugging
- **Commit creation failed** → exits with error message if `git commit` fails
- **Push declined** → exits gracefully with "Push cancelled. Commit saved locally." when user responds with `n` or `no`
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
├── src/main.rs      # Core logic
├── Cargo.toml       # Dependencies and metadata
└── README.md        # This file
```

### Building

```bash
cargo build
```

## License

This project is open source. See the repository for details.

## Troubleshooting

### Common Issues

**"No staged changes found"**

- This occurs when there are no modified files in your working directory. Make sure you have uncommitted changes before running `git-cmt-rs`.

**"OPENAI_API_KEY not set"**

- Export your OpenAI API key.

**"OpenAI request failed"**

- Check your internet connection.
- Verify your API key.
- Ensure your account has credits and API access.

**Editor not opening**

- Set your editor explicitly:
  ```bash
  export EDITOR="code --wait"
  ```

**"Push cancelled. Commit saved locally." message**

- This is expected behavior. The user can respond `n` or `no` at the push confirmation prompt to keep the commit local.
- The commit is already created and saved; the push is simply skipped.
- You can push manually later with `git push`.

**"Push failed" error**

- The commit was created successfully, but the push to remote failed (network issues, authentication, etc.)
- Your commit is safely saved locally
- You can try pushing again manually or resolve any issues before retrying
