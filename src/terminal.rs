//! A root shell in the browser: System → Diagnostics → Terminal.
//!
//! Everything on this device runs as root, so this hands full control of the
//! Pi to whoever gets through. Every safeguard below is enforced here on the
//! Pi, never left to the page:
//!
//! - `Settings::terminal_enabled` (off by default) gates every terminal route.
//! - Step-up: opening a terminal needs the control password again, even with
//!   a valid session cookie (which lasts 30 days). Wrong guesses count
//!   against the same per-IP lockout as the login page (see `api.rs`). A
//!   right password yields a ticket: random, single-use, valid for
//!   `TICKET_TTL`, and only redeemable together with the same session cookie
//!   that asked for it.
//! - The WebSocket handshake must come from this page's own origin: the
//!   `X-DJI-Control` CSRF header can't be set on a browser WebSocket, so the
//!   `Origin` check is what stops another website from opening one.
//! - One terminal at a time. A second is refused, never queued or taken over.
//! - It closes after `IDLE` without input, after `MAX` regardless, when the
//!   setting is turned off, when this browser's session is signed out, when
//!   the browser stops answering pings or stops reading, and when the server
//!   stops. The shell is hung up, then everything still left in its session
//!   is killed. No send can stall the loop that checks all this for longer
//!   than `SEND_TIMEOUT`.
//!
//! The shell runs on a real pseudo-terminal (`openpty`, with the slave side
//! as the controlling terminal of a new session), so full-screen programs,
//! job control and window resizing behave as they do over SSH. Verified on
//! the Pi itself (static aarch64 musl build), not only on the build host.
use crate::sync::LockExt;
use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::ws::{CloseFrame, Message, WebSocket},
    http::{HeaderMap, Uri, header},
};
use serde_json::json;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    net::IpAddr,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// Closed after this long without any input from the browser.
pub const IDLE: Duration = Duration::from_secs(10 * 60);
/// Closed after this long no matter what.
pub const MAX: Duration = Duration::from_secs(30 * 60);
/// How long a step-up ticket stays redeemable.
pub const TICKET_TTL: Duration = Duration::from_secs(20);
/// Nothing at all from the browser (not even a pong) for this long means it
/// is gone — a phone that died or left the Wi-Fi — so the single slot frees
/// up quickly instead of only after `IDLE`.
const SILENT: Duration = Duration::from_secs(45);
const PING_EVERY: Duration = Duration::from_secs(15);
/// Longest one message to the browser (or one piece of typing to the shell)
/// may take. A browser that stops reading, or a shell that stops taking
/// input, must not stall the session loop: every limit here is checked by
/// that loop.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest single message accepted from the browser (a big paste).
pub const MAX_FRAME: usize = 64 * 1024;
/// Bound on outstanding tickets, so asking for tickets can't grow memory.
const MAX_TICKETS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub idle: Duration,
    pub max: Duration,
}
impl Limits {
    /// `IDLE`/`MAX`, optionally shortened (for testing) by
    /// `UNGOGGLED_TERMINAL_IDLE_SECS` / `UNGOGGLED_TERMINAL_MAX_SECS`. An
    /// override can only make a limit shorter, never longer.
    pub fn from_env() -> Self {
        Self::shortened(
            std::env::var("UNGOGGLED_TERMINAL_IDLE_SECS")
                .ok()
                .as_deref(),
            std::env::var("UNGOGGLED_TERMINAL_MAX_SECS").ok().as_deref(),
        )
    }
    fn shortened(idle: Option<&str>, max: Option<&str>) -> Self {
        let pick = |value: Option<&str>, default: Duration| {
            value
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|&secs| secs > 0)
                .map_or(default, |secs| Duration::from_secs(secs).min(default))
        };
        Self {
            idle: pick(idle, IDLE),
            max: pick(max, MAX),
        }
    }
}

struct Ticket {
    session: String,
    issued: Instant,
}
#[derive(Default)]
struct Inner {
    tickets: HashMap<String, Ticket>,
    /// Id of the one open terminal, if any.
    active: Option<u64>,
}

/// Tickets and the single terminal slot, shared by the HTTP handlers.
#[derive(Clone, Default)]
pub struct Terminals {
    inner: Arc<Mutex<Inner>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Another terminal is open.
    Busy,
    /// Unknown, already used, expired, or issued to a different session.
    InvalidTicket,
}

impl Terminals {
    pub fn active(&self) -> bool {
        self.inner.lock_ok().active.is_some()
    }
    /// Stores `ticket` for `session` (the caller's session cookie). Only call
    /// this after the password has been checked again. Refused while a
    /// terminal is open; replaces any earlier ticket of the same session.
    pub fn issue(&self, session: &str, ticket: String) -> Result<(), Refused> {
        let mut inner = self.inner.lock_ok();
        if inner.active.is_some() {
            return Err(Refused::Busy);
        }
        let now = Instant::now();
        inner.tickets.retain(|_, t| {
            now.duration_since(t.issued) < TICKET_TTL
                && !crate::auth::tokens_equal(&t.session, session)
        });
        if inner.tickets.len() >= MAX_TICKETS
            && let Some(oldest) = inner
                .tickets
                .iter()
                .min_by_key(|(_, t)| t.issued)
                .map(|(k, _)| k.clone())
        {
            inner.tickets.remove(&oldest);
        }
        inner.tickets.insert(
            ticket,
            Ticket {
                session: session.to_owned(),
                issued: now,
            },
        );
        Ok(())
    }
    /// Redeems `ticket` and claims the terminal slot, in one step so two
    /// tickets can't both open a terminal. The ticket is used up by this
    /// call whether or not it was valid.
    pub fn open(&self, ticket: &str, session: &str) -> Result<Slot, Refused> {
        let mut inner = self.inner.lock_ok();
        let valid = inner.tickets.remove(ticket).is_some_and(|t| {
            t.issued.elapsed() < TICKET_TTL && crate::auth::tokens_equal(&t.session, session)
        });
        if !valid {
            return Err(Refused::InvalidTicket);
        }
        if inner.active.is_some() {
            return Err(Refused::Busy);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        inner.active = Some(id);
        Ok(Slot {
            terminals: self.clone(),
            id,
        })
    }
}

/// The claim on the single terminal slot; frees it when dropped, however the
/// session ended (including a WebSocket upgrade that never completed).
pub struct Slot {
    terminals: Terminals,
    id: u64,
}
impl Drop for Slot {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.terminals.inner.lock()
            && inner.active == Some(self.id)
        {
            inner.active = None;
        }
    }
}

/// True if the WebSocket handshake was started by a page from this same
/// host. Browsers always send `Origin` on a WebSocket handshake; a missing
/// one is refused too.
pub fn same_origin(headers: &HeaderMap, uri: &Uri) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(origin_host) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    // HTTP/1.1 carries the target in Host, HTTP/2 in the URI authority.
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| uri.authority().map(|a| a.as_str()));
    host.is_some_and(|h| !h.is_empty() && h.eq_ignore_ascii_case(origin_host))
}

/// A shell on a pseudo-terminal.
pub struct Pty {
    master: File,
    child: Child,
}
impl Pty {
    pub fn spawn(rows: u16, cols: u16) -> Result<Self> {
        let (mut master, mut slave): (libc::c_int, libc::c_int) = (-1, -1);
        let size = winsize(rows, cols);
        // SAFETY: openpty writes two fds into the out-params; the termios
        // pointer may be null and the winsize pointer is a valid reference.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error()).context("openpty");
        }
        // SAFETY: both fds were just returned by openpty and are owned by
        // nothing else; wrapping them closes them on every return path.
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        // Neither end may leak into the shell beyond its stdio copies.
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            set_cloexec(fd)?;
        }
        let shell = if Path::new("/bin/bash").exists() {
            "/bin/bash"
        } else {
            "/bin/sh"
        };
        let home = home_dir();
        let mut command = Command::new(shell);
        command
            .arg("-l")
            .env_clear()
            .env("TERM", "xterm-256color")
            .env("HOME", &home)
            .env("SHELL", shell)
            .env("LANG", "C.UTF-8")
            .env(
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            )
            .current_dir(&home)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // SAFETY: runs in the forked child before exec, after std has set up
        // stdio (fd 0 is the pty slave); setsid and ioctl are both
        // async-signal-safe. The new session makes the slave its controlling
        // terminal, so job control and hangups work as in a real login.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().with_context(|| format!("start {shell}"))?;
        // Dropping the command closes this process's copies of the slave, so
        // the master sees end-of-file once the shell side is gone.
        drop(command);
        Ok(Self { master, child })
    }
    pub fn reader(&self) -> std::io::Result<File> {
        self.master.try_clone()
    }
    pub fn writer(&self) -> std::io::Result<File> {
        self.master.try_clone()
    }
    pub fn resize(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        let size = winsize(rows, cols);
        // SAFETY: TIOCSWINSZ reads a winsize struct from a valid reference.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    /// Hangs up the shell (which passes the hangup on to its jobs), waits up
    /// to two seconds, then kills whatever is left in its session. Blocking.
    /// Only a process that deliberately started a new session of its own
    /// (`setsid`, a daemon) outlives the terminal, as it would an SSH login.
    pub fn terminate(self) -> Option<ExitStatus> {
        let Pty { master, mut child } = self;
        let sid = child.id() as libc::pid_t;
        // SAFETY: plain signal delivery to the shell's own process group.
        unsafe { libc::killpg(sid, libc::SIGHUP) };
        drop(master);
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                _ => {
                    // SAFETY: as above.
                    unsafe { libc::killpg(sid, libc::SIGKILL) };
                    break child.wait().ok();
                }
            }
        };
        kill_session(sid);
        status
    }
}

fn winsize(rows: u16, cols: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}
fn set_cloexec(fd: RawFd) -> Result<()> {
    // SAFETY: fcntl on an fd this process owns.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error()).context("fcntl FD_CLOEXEC");
    }
    Ok(())
}
/// Root's home on the Pi; the running account's anywhere else (tests, a dev
/// machine).
fn home_dir() -> String {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        "/root".into()
    } else {
        std::env::var("HOME").unwrap_or_else(|_| "/".into())
    }
}
/// SIGKILLs every process whose session id is `sid` (the shell's own pid, as
/// it led the session it created). Reads `/proc/<pid>/stat`, where the
/// session id is the fourth field after the parenthesized command name.
fn kill_session(sid: libc::pid_t) {
    if sid <= 1 {
        return;
    }
    let Ok(entries) = fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // The command name may itself contain spaces and parentheses.
        let session = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(3))
            .and_then(|s| s.parse::<libc::pid_t>().ok());
        if session == Some(sid) {
            // SAFETY: plain signal delivery.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// `{"t":"resize","rows":R,"cols":C}` from the browser, clamped to sane sizes.
fn parse_resize(text: &str) -> Option<(u16, u16)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v["t"] != "resize" {
        return None;
    }
    let rows = v["rows"].as_u64()?.clamp(2, 500) as u16;
    let cols = v["cols"].as_u64()?.clamp(2, 1000) as u16;
    Some((rows, cols))
}

/// Runs one terminal session over an upgraded WebSocket until it closes for
/// any reason, then shuts the shell down and frees the slot.
///
/// Browser → Pi: binary messages are keyboard input; text messages are JSON
/// control (`resize`). Pi → browser: binary messages are terminal output;
/// text messages are JSON status, `hello` (the limits) first and `closed`
/// (with the reason) last.
///
/// `stop` is polled every second and names a reason when the session must
/// end for an outside cause (setting turned off, signed out, shutdown).
pub async fn serve<F>(mut socket: WebSocket, slot: Slot, ip: IpAddr, limits: Limits, mut stop: F)
where
    F: FnMut() -> Option<&'static str> + Send + 'static,
{
    eprintln!("terminal: opened by {ip}");
    let opened = Instant::now();
    let reason = match tokio::task::spawn_blocking(|| Pty::spawn(24, 80)).await {
        Ok(Ok(pty)) => {
            let reason = pump(&mut socket, &pty, limits, &mut stop).await;
            let _ = tokio::task::spawn_blocking(move || pty.terminate()).await;
            reason
        }
        Ok(Err(e)) => {
            eprintln!("terminal: cannot start a shell: {e:#}");
            "error"
        }
        Err(_) => "error",
    };
    // The shell is gone, so the slot is free, even if saying goodbye to a
    // browser that stopped reading takes until the send timeout.
    drop(slot);
    eprintln!(
        "terminal: closed ({reason}) for {ip} after {}s",
        opened.elapsed().as_secs()
    );
    let closed = json!({"t":"closed","reason":reason}).to_string();
    if send(&mut socket, Message::Text(closed.into())).await {
        let frame = CloseFrame {
            code: 1000,
            reason: reason.into(),
        };
        send(&mut socket, Message::Close(Some(frame))).await;
    }
}

/// `socket.send` bounded by `SEND_TIMEOUT`; false if it failed or timed out.
async fn send(socket: &mut WebSocket, msg: Message) -> bool {
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(msg)).await,
        Ok(Ok(()))
    )
}

async fn pump<F>(socket: &mut WebSocket, pty: &Pty, limits: Limits, stop: &mut F) -> &'static str
where
    F: FnMut() -> Option<&'static str>,
{
    let (Ok(mut reader), Ok(mut writer)) = (pty.reader(), pty.writer()) else {
        return "error";
    };
    // Blocking pty I/O lives on two plain threads; channels give both
    // directions backpressure instead of unbounded buffering. Both threads
    // end on their own once the shell is gone or the channels close.
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                // EIO once the shell and everything else on the tty exited.
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let (in_tx, mut in_rx) = mpsc::channel::<Bytes>(64);
    std::thread::spawn(move || {
        while let Some(data) = in_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });
    let hello = json!({
        "t": "hello",
        "idle_seconds": limits.idle.as_secs(),
        "max_seconds": limits.max.as_secs(),
    });
    if !send(socket, Message::Text(hello.to_string().into())).await {
        return "lost";
    }
    let opened = Instant::now();
    let (mut last_input, mut last_seen, mut last_ping) = (opened, opened, opened);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { return "closed" };
                last_seen = Instant::now();
                match msg {
                    Message::Binary(data) => {
                        last_input = last_seen;
                        match tokio::time::timeout(SEND_TIMEOUT, in_tx.send(data)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => return "exited",
                            // Megabytes of typing the shell hasn't taken in
                            // all that time: it's stuck.
                            Err(_) => return "stuck",
                        }
                    }
                    Message::Text(text) => {
                        if let Some((rows, cols)) = parse_resize(text.as_str()) {
                            let _ = pty.resize(rows, cols);
                        }
                    }
                    Message::Close(_) => return "closed",
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            out = out_rx.recv() => {
                let Some(out) = out else { return "exited" };
                if !send(socket, Message::Binary(out.into())).await {
                    return "lost";
                }
            }
            _ = tick.tick() => {
                if let Some(reason) = stop() {
                    return reason;
                }
                let now = Instant::now();
                if now.duration_since(opened) >= limits.max {
                    return "limit";
                }
                if now.duration_since(last_input) >= limits.idle {
                    return "idle";
                }
                if now.duration_since(last_seen) >= SILENT {
                    return "lost";
                }
                if now.duration_since(last_ping) >= PING_EVERY {
                    last_ping = now;
                    if !send(socket, Message::Ping(Bytes::new())).await {
                        return "lost";
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn limits_can_only_be_shortened() {
        assert_eq!(
            Limits::shortened(None, None),
            Limits {
                idle: IDLE,
                max: MAX
            }
        );
        let short = Limits::shortened(Some("15"), Some("40"));
        assert_eq!(short.idle, Duration::from_secs(15));
        assert_eq!(short.max, Duration::from_secs(40));
        let long = Limits::shortened(Some("999999"), Some("999999"));
        assert_eq!(long.idle, IDLE);
        assert_eq!(long.max, MAX);
        let junk = Limits::shortened(Some("0"), Some("soon"));
        assert_eq!(junk.idle, IDLE);
        assert_eq!(junk.max, MAX);
    }

    #[test]
    fn a_ticket_opens_exactly_one_terminal_for_its_own_session() {
        let t = Terminals::default();
        t.issue("session-a", "ticket-1".into()).unwrap();
        let slot = t.open("ticket-1", "session-a").expect("valid ticket");
        assert!(t.active());
        drop(slot);
        assert!(!t.active());
        assert_eq!(
            t.open("ticket-1", "session-a").err(),
            Some(Refused::InvalidTicket),
            "a ticket is single-use"
        );
    }

    #[test]
    fn a_ticket_is_useless_to_another_session_and_is_burned_by_trying() {
        let t = Terminals::default();
        t.issue("session-a", "ticket-1".into()).unwrap();
        assert_eq!(
            t.open("ticket-1", "session-b").err(),
            Some(Refused::InvalidTicket)
        );
        assert_eq!(
            t.open("ticket-1", "session-a").err(),
            Some(Refused::InvalidTicket)
        );
        assert!(!t.active());
    }

    #[test]
    fn expired_and_unknown_tickets_are_refused() {
        let t = Terminals::default();
        t.issue("session-a", "ticket-1".into()).unwrap();
        t.inner
            .lock()
            .unwrap()
            .tickets
            .get_mut("ticket-1")
            .unwrap()
            .issued -= TICKET_TTL + Duration::from_secs(1);
        assert_eq!(
            t.open("ticket-1", "session-a").err(),
            Some(Refused::InvalidTicket)
        );
        assert_eq!(
            t.open("never-issued", "session-a").err(),
            Some(Refused::InvalidTicket)
        );
    }

    #[test]
    fn only_one_terminal_at_a_time() {
        let t = Terminals::default();
        t.issue("session-a", "ticket-a".into()).unwrap();
        t.issue("session-b", "ticket-b".into()).unwrap();
        let slot = t.open("ticket-a", "session-a").unwrap();
        // A ticket issued before the first terminal opened still can't open
        // a second one...
        assert_eq!(t.open("ticket-b", "session-b").err(), Some(Refused::Busy));
        // ...and no new ticket is handed out while one is open.
        assert_eq!(
            t.issue("session-b", "ticket-c".into()).err(),
            Some(Refused::Busy)
        );
        drop(slot);
        t.issue("session-b", "ticket-d".into()).unwrap();
        assert!(t.open("ticket-d", "session-b").is_ok());
    }

    #[test]
    fn a_new_ticket_replaces_the_previous_one_of_the_same_session() {
        let t = Terminals::default();
        t.issue("session-a", "ticket-1".into()).unwrap();
        t.issue("session-a", "ticket-2".into()).unwrap();
        assert_eq!(
            t.open("ticket-1", "session-a").err(),
            Some(Refused::InvalidTicket)
        );
        assert!(t.open("ticket-2", "session-a").is_ok());
    }

    #[test]
    fn outstanding_tickets_are_bounded() {
        let t = Terminals::default();
        for i in 0..(MAX_TICKETS * 3) {
            t.issue(&format!("session-{i}"), format!("ticket-{i}"))
                .unwrap();
        }
        assert!(t.inner.lock_ok().tickets.len() <= MAX_TICKETS);
    }

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn websocket_handshakes_must_come_from_this_origin() {
        let uri: Uri = "/api/terminal/ws?ticket=x".parse().unwrap();
        let ok = |origin: &str, host: &str| {
            same_origin(
                &headers(&[(header::ORIGIN, origin), (header::HOST, host)]),
                &uri,
            )
        };
        assert!(ok("https://192.168.50.1", "192.168.50.1"));
        assert!(ok("http://localhost:8080", "localhost:8080"));
        assert!(ok("https://UNGOGGLED.local", "ungoggled.local"));
        assert!(!ok("https://evil.example", "192.168.50.1"));
        assert!(!ok("https://192.168.50.1.evil.example", "192.168.50.1"));
        assert!(!ok("http://localhost:9999", "localhost:8080"));
        assert!(!ok("null", "192.168.50.1"));
        assert!(!ok("file://", "192.168.50.1"));
        assert!(!same_origin(
            &headers(&[(header::HOST, "192.168.50.1")]),
            &uri
        ));
        // HTTP/2: no Host header, the target is in the URI authority.
        let h2: Uri = "https://192.168.50.1/api/terminal/ws".parse().unwrap();
        assert!(same_origin(
            &headers(&[(header::ORIGIN, "https://192.168.50.1")]),
            &h2
        ));
    }

    #[test]
    fn resize_messages_are_parsed_and_clamped() {
        assert_eq!(
            parse_resize(r#"{"t":"resize","rows":40,"cols":120}"#),
            Some((40, 120))
        );
        assert_eq!(
            parse_resize(r#"{"t":"resize","rows":0,"cols":99999}"#),
            Some((2, 1000))
        );
        assert_eq!(parse_resize(r#"{"t":"other","rows":4,"cols":4}"#), None);
        assert_eq!(parse_resize("not json"), None);
    }

    /// Reads from the pty until `needle` shows up (or 10 s pass).
    fn read_until(reader: &mut File, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        while Instant::now() < deadline {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
            }
            if String::from_utf8_lossy(&seen).contains(needle) {
                break;
            }
        }
        String::from_utf8_lossy(&seen).into_owned()
    }

    #[test]
    fn a_shell_runs_on_a_real_terminal_and_follows_resizes() {
        let mut pty = Pty::spawn(24, 80).unwrap();
        let mut reader = pty.reader().unwrap();
        let mut writer = pty.writer().unwrap();
        // Markers are computed by the shell so the echoed command line itself
        // can't match them.
        writer
            .write_all(b"tty; stty size; echo MARK$((40+2))\n")
            .unwrap();
        let out = read_until(&mut reader, "MARK42");
        assert!(out.contains("/dev/pts/"), "has a real tty: {out}");
        assert!(out.contains("24 80"), "initial size: {out}");
        pty.resize(30, 100).unwrap();
        writer
            .write_all(b"stty size; echo MARK$((50+3))\n")
            .unwrap();
        let out = read_until(&mut reader, "MARK53");
        assert!(out.contains("30 100"), "resized: {out}");
        writer.write_all(b"exit 7\n").unwrap();
        // Let the shell act on `exit` itself first: `terminate` hangs it up
        // straight away, which would report the signal instead.
        let deadline = Instant::now() + Duration::from_secs(10);
        while pty.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let status = pty.terminate().expect("exit status");
        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn ending_a_terminal_also_ends_its_background_jobs() {
        let pty = Pty::spawn(24, 80).unwrap();
        let mut reader = pty.reader().unwrap();
        let mut writer = pty.writer().unwrap();
        writer
            .write_all(b"sleep 300 & echo JOB=$! END$((1+1))\n")
            .unwrap();
        let out = read_until(&mut reader, "END2");
        // The echoed command line also contains "JOB=$!"; take the first
        // occurrence followed by an actual number.
        let pid: u32 = out
            .split("JOB=")
            .skip(1)
            .find_map(|s| s.split_whitespace().next()?.parse().ok())
            .unwrap_or_else(|| panic!("no job pid in: {out}"));
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        pty.terminate();
        let deadline = Instant::now() + Duration::from_secs(3);
        // A killed child of the shell may linger as a zombie until it is
        // reaped by init; what matters is that it is no longer running.
        let running = || {
            fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|s| {
                    s.rsplit_once(')')
                        .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_owned))
                })
                .is_some_and(|state| state != "Z" && state != "X")
        };
        while running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!running(), "background job {pid} survived the terminal");
    }
}
