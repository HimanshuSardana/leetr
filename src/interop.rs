//! Loopback control API.
//!
//! Leetr hosts a tiny HTTP server on `127.0.0.1:<interop_port>` so external
//! tooling — in practice the bundled nvim plugin — can talk to a running
//! instance:
//!
//! | Method | Path      | Purpose                                        |
//! |--------|-----------|------------------------------------------------|
//! | GET    | `/health` | liveness + version                             |
//! | GET    | `/state`  | current problem, language, code, solved state  |
//! | GET    | `/code`   | current buffer, as text                        |
//! | POST   | `/code`   | replace the current buffer                     |
//! | POST   | `/open`   | open a problem by slug                         |
//!
//! Reads are served straight from a shared [`Snapshot`] the UI publishes each
//! frame, so they never block. Writes are queued as [`Command`]s and applied by
//! the UI on its next frame, then acknowledged immediately — the plugin stays
//! responsive and the UI stays single-threaded for state changes.
//!
//! The server binds to loopback only; it is not reachable off the machine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Response, Server, StatusCode};

/// Reported by `/health`.
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the control API can report about the running instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Slug of the problem currently open, if any.
    pub slug: Option<String>,
    pub title: Option<String>,
    /// Language slug of the current buffer.
    pub language: Option<String>,
    /// Path of the file the buffer is backed by.
    pub file: Option<String>,
    pub code: String,
    /// Whether the open problem is already solved on this account.
    pub solved: bool,
    /// Whether a statement is loaded and ready to edit.
    pub ready: bool,
    /// Free-form status line, mirrored from the app.
    pub status: String,
    pub version: String,
}

/// A state change requested over the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Replace the buffer for the current (or given) problem.
    SetCode {
        slug: Option<String>,
        language: Option<String>,
        code: String,
    },
    /// Open a problem.
    Open { slug: String },
}

/// Requests accepted from the nvim plugin.
#[derive(Debug, Deserialize)]
pub struct SetCodeRequest {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenRequest {
    pub slug: String,
}

/// Called when a command is queued, so the UI can wake up and apply it.
///
/// egui only runs a frame when something asks it to, so without this a queued
/// command would sit unapplied until the user happened to interact.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// State shared between the UI thread and the server thread.
pub struct Shared {
    snapshot: RwLock<Snapshot>,
    commands: Mutex<Vec<Command>>,
    waker: Mutex<Option<Waker>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

impl Shared {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(Snapshot {
                version: API_VERSION.to_string(),
                ..Snapshot::default()
            }),
            commands: Mutex::new(Vec::new()),
            waker: Mutex::new(None),
        }
    }

    /// Register the callback that wakes the UI when a command arrives.
    pub fn set_waker(&self, waker: Waker) {
        if let Ok(mut guard) = self.waker.lock() {
            *guard = Some(waker);
        }
    }

    /// Replace the published snapshot (called by the UI each frame).
    pub fn publish(&self, snapshot: Snapshot) {
        if let Ok(mut guard) = self.snapshot.write() {
            *guard = snapshot;
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Queue a command for the UI to apply, waking it so it runs a frame.
    pub fn push(&self, command: Command) {
        if let Ok(mut guard) = self.commands.lock() {
            guard.push(command);
        }
        let waker = self.waker.lock().ok().and_then(|guard| guard.clone());
        if let Some(waker) = waker {
            waker();
        }
    }

    /// Take every queued command (called by the UI each frame).
    pub fn take_commands(&self) -> Vec<Command> {
        self.commands
            .lock()
            .map(|mut guard| std::mem::take(&mut *guard))
            .unwrap_or_default()
    }
}

/// A running control-API server.
pub struct RunningServer {
    port: u16,
    server: Arc<Server>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RunningServer {
    /// The bound port. Useful when `bind` was `127.0.0.1:0`.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Stop accepting requests and join the server thread.
    pub fn shutdown(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        // `unblock` makes the blocking `incoming_requests()` iterator return.
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Start the control API.
///
/// `bind` is a `host:port` string; pass `127.0.0.1:0` for an ephemeral port.
pub fn start(bind: &str, shared: Arc<Shared>) -> Result<RunningServer> {
    let server =
        Arc::new(Server::http(bind).map_err(|err| anyhow::anyhow!("binding {bind}: {err}"))?);

    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .ok_or_else(|| anyhow::anyhow!("control API must bind to an IP socket"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let server = Arc::clone(&server);
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let shared = Arc::clone(&shared);
                // Handling inline keeps ordering deterministic and the payloads
                // are tiny; a panic here must not kill the server.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = handle(request, &shared);
                }));
            }
        })
    };

    Ok(RunningServer {
        port,
        server,
        stop,
        thread: Some(thread),
    })
}

fn handle(mut request: tiny_http::Request, shared: &Shared) -> Result<()> {
    let method = request.method().clone();
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let mut body = String::new();
    if method == Method::Post {
        let _ = request.as_reader().read_to_string(&mut body);
    }

    let result = match (&method, path.as_str()) {
        (Method::Get, "/health") => respond_json(
            request,
            200,
            serde_json::json!({ "ok": true, "version": API_VERSION }),
        ),
        (Method::Get, "/state") => {
            let snapshot = shared.snapshot();
            match serde_json::to_value(&snapshot) {
                Ok(value) => respond_json(request, 200, value),
                Err(err) => respond_error(request, 500, &err.to_string()),
            }
        }
        (Method::Get, "/code") => respond_text(request, 200, shared.snapshot().code),
        (Method::Post, "/code") => match serde_json::from_str::<SetCodeRequest>(&body) {
            Ok(payload) => {
                shared.push(Command::SetCode {
                    slug: payload.slug,
                    language: payload.language,
                    code: payload.code,
                });
                respond_json(
                    request,
                    202,
                    serde_json::json!({ "ok": true, "queued": true }),
                )
            }
            Err(err) => respond_error(request, 400, &format!("invalid JSON body: {err}")),
        },
        (Method::Post, "/open") => match serde_json::from_str::<OpenRequest>(&body) {
            Ok(payload) if !payload.slug.trim().is_empty() => {
                shared.push(Command::Open {
                    slug: payload.slug.trim().to_string(),
                });
                respond_json(
                    request,
                    202,
                    serde_json::json!({ "ok": true, "queued": true }),
                )
            }
            Ok(_) => respond_error(request, 400, "`slug` must not be empty"),
            Err(err) => respond_error(request, 400, &format!("invalid JSON body: {err}")),
        },
        (_, "/health" | "/state" | "/code" | "/open") => {
            respond_error(request, 405, "method not allowed")
        }
        _ => respond_error(request, 404, "no such endpoint"),
    };

    result.context("writing response")
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header is valid")
}

fn respond_json(request: tiny_http::Request, status: u16, value: serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec(&value)?;
    request
        .respond(
            Response::from_data(body)
                .with_status_code(StatusCode(status))
                .with_header(json_header()),
        )
        .context("sending JSON response")
}

fn respond_text(request: tiny_http::Request, status: u16, body: String) -> Result<()> {
    request
        .respond(
            Response::from_string(body)
                .with_status_code(StatusCode(status))
                .with_header(
                    Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..])
                        .expect("static header is valid"),
                ),
        )
        .context("sending text response")
}

fn respond_error(request: tiny_http::Request, status: u16, message: &str) -> Result<()> {
    respond_json(
        request,
        status,
        serde_json::json!({ "ok": false, "error": message }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    /// Minimal one-shot HTTP/1.1 client, so the tests do not need a dependency.
    fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
        if let Some(body) = body {
            request.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        request.push_str("\r\n");
        if let Some(body) = body {
            request.push_str(body);
        }

        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let raw = String::from_utf8_lossy(&raw).into_owned();

        let status = raw
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let body = raw
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default();
        (status, body)
    }

    fn server() -> (RunningServer, Arc<Shared>) {
        let shared = Arc::new(Shared::new());
        let server = start("127.0.0.1:0", Arc::clone(&shared)).expect("start");
        (server, shared)
    }

    #[test]
    fn binds_an_ephemeral_port_on_loopback() {
        let (server, _) = server();
        assert_ne!(server.port(), 0);
        assert!(server.base_url().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn health_reports_the_version() {
        let (server, _) = server();
        let (status, body) = request(server.port(), "GET", "/health", None);
        assert_eq!(status, 200);
        assert!(body.contains("\"ok\":true"), "got {body}");
        assert!(body.contains(API_VERSION), "got {body}");
    }

    #[test]
    fn state_reflects_published_snapshots() {
        let (server, shared) = server();

        let (status, body) = request(server.port(), "GET", "/state", None);
        assert_eq!(status, 200);
        assert!(body.contains("\"ready\":false"), "got {body}");

        shared.publish(Snapshot {
            slug: Some("two-sum".into()),
            title: Some("Two Sum".into()),
            language: Some("rust".into()),
            code: "impl Solution {}".into(),
            solved: true,
            ready: true,
            status: "ok".into(),
            version: API_VERSION.into(),
            file: None,
        });

        let (status, parsed) = request(server.port(), "GET", "/state", None);
        assert_eq!(status, 200);
        let parsed: Snapshot = serde_json::from_str(&parsed).expect("valid snapshot json");
        assert_eq!(parsed.slug.as_deref(), Some("two-sum"));
        assert_eq!(parsed.language.as_deref(), Some("rust"));
        assert!(parsed.solved);
        assert!(parsed.ready);
    }

    #[test]
    fn code_endpoint_returns_the_raw_buffer() {
        let (server, shared) = server();
        shared.publish(Snapshot {
            code: "fn main() {}\n".into(),
            ..Snapshot::default()
        });

        let (status, body) = request(server.port(), "GET", "/code", None);
        assert_eq!(status, 200);
        assert_eq!(body, "fn main() {}\n");
    }

    #[test]
    fn pushing_a_command_wakes_the_ui() {
        use std::sync::atomic::AtomicUsize;

        let shared = Shared::new();
        let woke = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woke);
        shared.set_waker(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));

        shared.push(Command::Open {
            slug: "two-sum".into(),
        });
        assert_eq!(woke.load(Ordering::SeqCst), 1, "push must wake the UI");

        shared.take_commands();
        assert_eq!(
            woke.load(Ordering::SeqCst),
            1,
            "draining must not wake anything"
        );
    }

    #[test]
    fn commands_queue_without_a_registered_waker() {
        let shared = Shared::new();
        shared.push(Command::Open {
            slug: "two-sum".into(),
        });
        assert_eq!(shared.take_commands().len(), 1);
    }

    #[test]
    fn a_request_wakes_the_ui() {
        let shared = Arc::new(Shared::new());
        let woke = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&woke);
        shared.set_waker(Arc::new(move || flag.store(true, Ordering::SeqCst)));

        let server = start("127.0.0.1:0", Arc::clone(&shared)).expect("start");
        let (status, _) = request(
            server.port(),
            "POST",
            "/open",
            Some(r#"{"slug":"two-sum"}"#),
        );
        assert_eq!(status, 202);
        assert!(
            woke.load(Ordering::SeqCst),
            "an API request must wake the UI so the command is applied"
        );
    }

    #[test]
    fn posting_code_queues_a_command() {
        let (server, shared) = server();
        let (status, body) = request(
            server.port(),
            "POST",
            "/code",
            Some(r#"{"slug":"two-sum","language":"rust","code":"// hi"}"#),
        );
        assert_eq!(status, 202, "got {body}");

        let commands = shared.take_commands();
        assert_eq!(
            commands,
            vec![Command::SetCode {
                slug: Some("two-sum".into()),
                language: Some("rust".into()),
                code: "// hi".into(),
            }]
        );
        assert!(shared.take_commands().is_empty(), "commands are drained");
    }

    #[test]
    fn posting_open_queues_an_open_command() {
        let (server, shared) = server();
        let (status, _) = request(
            server.port(),
            "POST",
            "/open",
            Some(r#"{"slug":"two-sum"}"#),
        );
        assert_eq!(status, 202);
        assert_eq!(
            shared.take_commands(),
            vec![Command::Open {
                slug: "two-sum".into()
            }]
        );
    }

    #[test]
    fn malformed_json_is_rejected() {
        let (server, shared) = server();
        let (status, body) = request(server.port(), "POST", "/code", Some("{not json"));
        assert_eq!(status, 400);
        assert!(body.contains("invalid JSON"), "got {body}");
        assert!(shared.take_commands().is_empty(), "nothing must be queued");
    }

    #[test]
    fn empty_slug_is_rejected() {
        let (server, shared) = server();
        let (status, _) = request(server.port(), "POST", "/open", Some(r#"{"slug":"   "}"#));
        assert_eq!(status, 400);
        assert!(shared.take_commands().is_empty());
    }

    #[test]
    fn unknown_endpoints_404() {
        let (server, _) = server();
        let (status, body) = request(server.port(), "GET", "/nope", None);
        assert_eq!(status, 404);
        assert!(body.contains("no such endpoint"));
    }

    #[test]
    fn wrong_method_is_405() {
        let (server, _) = server();
        assert_eq!(request(server.port(), "DELETE", "/code", None).0, 405);
        assert_eq!(request(server.port(), "GET", "/open", None).0, 405);
    }

    #[test]
    fn shutting_down_releases_the_port() {
        let (server, _) = server();
        let port = server.port();
        server.shutdown();

        // The listener must be gone. Retry briefly: because these tests run in
        // parallel, a sibling test can transiently hold the same recycled
        // ephemeral port. A genuinely broken shutdown never succeeds.
        let mut last_error = None;
        for _ in 0..60 {
            match Server::http(("127.0.0.1", port)) {
                Ok(_) => return,
                Err(err) => {
                    last_error = Some(err.to_string());
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        panic!("port {port} was still held after shutdown: {last_error:?}");
    }
}
