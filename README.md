# git-cmt-rs

An AI-powered Git commit message generator written in Rust that analyses your staged changes and creates [Conventional Commit](https://www.conventionalcommits.org/) messages using the OpenAI API.

## Overview

`git-cmt-rs` automatically generates meaningful commit messages based on your staged changes using OpenAI’s chat completion models. It follows the Conventional Commits specification and provides an interactive commit experience.

Feel free to tweak the code to try different models, providers, or prompt templates. The implementation is simple and hackable.

This project is a Rust implementation of the Go based git-cmd project found [here](https://github.com/appliedgocode/git-cmt/)

#### All respects and credits to the original authors.

## Features

- 🤖 **AI-powered**: Uses OpenAI models (default: `gpt-4.1-mini`) to analyze code changes
- 📝 **Conventional Commits**: Generates messages in the `type(scope): description` format
- 🎯 **Smart Analysis**: Understands code changes and suggests contextually appropriate messages
- ⚡ **Interactive**: Opens your editor for final review before committing
- 🔍 **Diff-aware**: Only analyzes staged changes
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

1. Stage your changes:
   ```bash
   git add <files>
   ```
2. Generate and commit:
   ```bash
   git-cmt-rs
   ```
3. Review and edit the generated message in your editor.
4. Save and close to complete the commit.

## How it works

1. **Diff Analysis**: Reads staged changes with `git diff --cached -b`
2. **AI Processing**: Sends the diff to OpenAI with structured prompts and JSON schema enforcement
3. **Message Generation**: Produces a commit object with `type`, `scope`, and `message`
4. **Interactive Commit**: Opens your editor with the generated message
5. **Final Commit**: Runs `git commit` with the approved message

## Commit Message Format

```
type(scope): description
```

- **Types**: feat, fix, docs, style, refactor, test, chore
- **Scope**: Optional component/module name
- **Description**: Clear, concise summary (max 50 chars)

## Examples

### Feature Addition

```bash
$ git-cmt-rs
# Generated: feat(auth): add OAuth2 login integration
```

### Bug Fix

```bash
$ git-cmt-rs
# Generated: fix(api): resolve null pointer in user validation
```

### Documentation

```bash
$ git-cmt-rs
# Generated: docs(readme): update installation instructions
```

## Configuration

### Environment Variables

- `OPENAI_API_KEY` – required for API access
- `OPENAI_MODEL` – model to use (default: `gpt-4.1-mini`)
- `OPENAI_BASE_URL` – API endpoint (default: `https://api.openai.com/v1`)
- `EDITOR` – editor for reviewing commits (defaults to system default)

## Error Handling

- **No staged changes** → exits with helpful message
- **Missing API key** → prompts to set `OPENAI_API_KEY`
- **API failures** → shows HTTP status and response body
- **Invalid JSON** → shows raw model output for debugging

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

- Run `git add` before using `git-cmt-rs`.

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
