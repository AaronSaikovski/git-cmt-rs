//! Real-binary safety checks using disposable repositories and a loopback API.

use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{self, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_git-cmt-rs");
static NEXT_REPO: AtomicU64 = AtomicU64::new(0);

struct Repo {
    root: PathBuf,
    dir: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let root = env::temp_dir().join(format!(
            "git-cmt-safety-{}-{}",
            process::id(),
            NEXT_REPO.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("isolated test directory should be created");
        let dir = root.join("repo");
        fs::create_dir(&dir).expect("isolated test directory should be created");
        fs::write(root.join("git-config"), "").expect("test fixture should be writable");
        let repo = Self { root, dir };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "Safety Test"]);
        repo.git(&["config", "user.email", "safety@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&[
            "config",
            "core.hooksPath",
            repo.root
                .join("no-hooks")
                .to_str()
                .expect("test hook directory path should be UTF-8"),
        ]);
        fs::write(repo.dir.join("tracked.txt"), "base\n").expect("test fixture should be writable");
        repo.git(&["add", "tracked.txt"]);
        repo.git(&["commit", "-qm", "initial"]);
        repo
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.current_dir(&self.dir);
        for (key, _) in env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("GIT_") || name.starts_with("OPENAI_") {
                command.env_remove(key);
            }
        }
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("git-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1");
        command
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        let output = self
            .command("git")
            .args(args)
            .output()
            .expect("test subprocess should launch and exit");
        assert!(output.status.success(), "{args:?}: {:?}", output.stderr);
        output.stdout
    }

    fn partial_stage(&self) {
        fs::write(self.dir.join("tracked.txt"), "base\nselected\n")
            .expect("test fixture should be writable");
        self.git(&["add", "tracked.txt"]);
        fs::write(
            self.dir.join("tracked.txt"),
            "base\nselected\nunstaged-marker\n",
        )
        .expect("test fixture should be writable");
        fs::write(self.dir.join("untracked.txt"), "untracked-marker\n")
            .expect("test fixture should be writable");
    }

    fn run(&self, args: &[&str], base: &str) -> Output {
        let mut child = self
            .command(BIN)
            .args(args)
            .env("OPENAI_BASE_URL", base)
            .env("OPENAI_MODEL", "test")
            .env("OPENAI_RESPONSE_FORMAT", "none")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("CLI subprocess should launch");
        let deadline = Instant::now() + Duration::from_secs(10);
        while child
            .try_wait()
            .expect("CLI exit status should be available")
            .is_none()
        {
            if Instant::now() >= deadline {
                child
                    .kill()
                    .expect("timed-out CLI subprocess should be stopped");
                let output = child
                    .wait_with_output()
                    .expect("CLI subprocess should be reaped");
                panic!("CLI did not terminate: {:?}", output.stderr);
            }
            thread::sleep(Duration::from_millis(10));
        }
        child
            .wait_with_output()
            .expect("CLI subprocess should be reaped")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn server(status: &str, body: String) -> (String, JoinHandle<Value>) {
    use std::io::{BufRead, Read, Write};

    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback test listener should bind");
    listener
        .set_nonblocking(true)
        .expect("test socket mode should be configurable");
    let address = listener
        .local_addr()
        .expect("bound listener should have an address");
    let base = format!("http://{address}/v1");
    let status = status.to_string();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "CLI did not contact the API");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream
            .set_nonblocking(false)
            .expect("test socket mode should be configurable");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("test socket read timeout should be configurable");
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("test socket write timeout should be configurable");
        let mut reader = BufReader::new(&mut stream);
        let mut length: Option<usize> = None;
        loop {
            let mut line = String::new();
            assert_ne!(
                reader
                    .read_line(&mut line)
                    .expect("HTTP headers should be readable"),
                0,
                "client disconnected before sending complete HTTP headers"
            );
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = Some(
                    value
                        .trim()
                        .parse()
                        .expect("Content-Length should be a valid byte count"),
                );
            }
        }
        let length = length.expect("HTTP request should declare Content-Length");
        assert!(length <= 64 * 1024);
        let mut request = vec![0; length];
        reader
            .read_exact(&mut request)
            .expect("complete HTTP request body should be readable");
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("test response should be writable");
        serde_json::from_slice(&request).expect("HTTP request body should be valid JSON")
    });
    (base, handle)
}

#[must_use]
fn model_response(message: &str) -> String {
    json!({"choices": [{"message": {"content": json!({
        "type": "feat", "scope": "test", "message": message
    }).to_string()}}]})
    .to_string()
}

fn assert_selected_request(request: &Value) {
    let diff = request["messages"][1]["content"]
        .as_str()
        .expect("request message content should be a string");
    assert!(diff.contains("+selected"));
    assert!(!diff.contains("unstaged-marker"));
    assert!(!diff.contains("untracked-marker"));
}

#[test]
fn dry_run_preserves_staging_and_escapes_model_terminal_controls() {
    let repo = Repo::new();
    repo.partial_stage();
    let index = fs::read(repo.dir.join(".git/index")).expect("test index should be readable");
    let head = repo.git(&["rev-parse", "HEAD"]);
    for flags in [
        vec!["--dry-run"],
        vec!["--dry-run", "--auto"],
        vec!["--staged", "--dry-run", "--auto"],
    ] {
        let (base, server) = server("200 OK", model_response("café \u{1b}[2J\r\n\u{202e}hidden"));
        let output = repo.run(&flags, &base);
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(
            String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8"),
            "feat(test): café \\u{1b}[2J\\r\\n\\u{202e}hidden\n"
        );
        let stderr = String::from_utf8(output.stderr).expect("CLI and Git output should be UTF-8");
        assert!(!stderr.contains(['\u{1b}', '\r', '\u{202e}']));
        assert_eq!(
            fs::read(repo.dir.join(".git/index")).expect("test index should be readable"),
            index
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read_to_string(repo.dir.join("tracked.txt"))
                .expect("tracked fixture should be readable"),
            "base\nselected\nunstaged-marker\n"
        );
        assert_eq!(
            fs::read_to_string(repo.dir.join("untracked.txt"))
                .expect("untracked fixture should be readable"),
            "untracked-marker\n"
        );
        assert_selected_request(
            &server
                .join()
                .expect("loopback server should finish successfully"),
        );
    }
}

#[test]
fn staged_commit_cancels_push_on_eof_and_preserves_unstaged_changes() {
    let repo = Repo::new();
    repo.partial_stage();
    let (base, server) = server("200 OK", model_response("selected \u{1b}[2J"));
    let output = repo.run(&["--staged"], &base);
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(repo.git(&["show", "HEAD:tracked.txt"]), b"base\nselected\n");
    assert_eq!(
        repo.git(&["log", "-1", "--format=%s"]),
        b"feat(test): selected \\u{1b}[2J\n"
    );
    assert!(
        String::from_utf8(repo.git(&["diff"]))
            .expect("Git diff should contain UTF-8 text")
            .contains("unstaged-marker")
    );
    assert_eq!(
        repo.git(&["ls-files", "--others", "--exclude-standard"]),
        b"untracked.txt\n"
    );
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stderr.contains(&0x1b));
    assert_selected_request(
        &server
            .join()
            .expect("loopback server should finish successfully"),
    );
}

#[test]
fn provider_error_is_sanitized_without_mutating_the_repository() {
    let repo = Repo::new();
    repo.partial_stage();
    let index = fs::read(repo.dir.join(".git/index")).expect("test index should be readable");
    let head = repo.git(&["rev-parse", "HEAD"]);
    let (base, server) = server(
        "500 Internal Server Error",
        "bad\u{1b}[2J\rresponse".to_string(),
    );
    let output = repo.run(&["--dry-run"], &base);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("CLI and Git output should be UTF-8");
    assert!(stderr.contains("500"));
    assert!(stderr.contains("bad\\u{1b}[2J\\rresponse"));
    assert!(!stderr.contains(['\u{1b}', '\r']));
    assert_eq!(
        fs::read(repo.dir.join(".git/index")).expect("test index should be readable"),
        index
    );
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    assert_selected_request(
        &server
            .join()
            .expect("loopback server should finish successfully"),
    );
}

#[test]
fn empty_index_flags_leave_untracked_files_unstaged() {
    let repo = Repo::new();
    fs::write(repo.dir.join("untracked.txt"), "not selected\n")
        .expect("test fixture should be writable");
    let index = fs::read(repo.dir.join(".git/index")).expect("test index should be readable");
    for flags in [
        vec!["--staged"],
        vec!["--dry-run"],
        vec!["--dry-run", "--staged", "--auto"],
    ] {
        let output = repo.run(&flags, "http://127.0.0.1:0/v1");
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .expect("CLI stderr should be UTF-8")
                .contains("no staged changes")
        );
        assert!(output.stdout.is_empty());
        assert_eq!(
            fs::read(repo.dir.join(".git/index")).expect("test index should be readable"),
            index
        );
        assert_eq!(
            repo.git(&["ls-files", "--others", "--exclude-standard"]),
            b"untracked.txt\n"
        );
    }
}
