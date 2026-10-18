//! Durable, privacy-conscious lifecycle diagnostics.
//!
//! A force quit (`SIGKILL`) cannot run cleanup code.  The on-disk session
//! marker lets the next launch report that the preceding session did not
//! finish cleanly.  macOS crash reports are summarized by filename and crash
//! reason so an AppKit abort remains diagnosable even when Rust never regains
//! control.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const LOG_FILE: &str = "diagnostics.jsonl";
const PREVIOUS_LOG_FILE: &str = "diagnostics.previous.jsonl";
const SESSION_FILE: &str = "active-session.json";

static SESSION: OnceLock<Session> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Serialize, Deserialize)]
struct SessionMarker {
    session_id: String,
    pid: u32,
    version: String,
    started_unix_ms: u64,
}

struct Session {
    id: String,
    log_path: PathBuf,
    marker_path: PathBuf,
    finished: AtomicBool,
    cleanup_completed: AtomicBool,
}

/// Start diagnostics for the interactive application process.
pub fn start_session() {
    let Some(root) = crate::settings::app_data_dir() else {
        return;
    };
    if SESSION.set(initialize_at(&root, now_unix_ms(), std::process::id())).is_ok() {
        #[cfg(target_os = "macos")]
        install_normal_exit_hook();
    }
}

/// App cleanup is necessary but not sufficient for a clean process exit.
/// Keep the marker until main returns or the normal C exit hook actually runs.
pub fn cleanup_completed() {
    if let Some(session) = SESSION.get() {
        append_event(session, "shutdown_complete", Value::Null);
        session.cleanup_completed.store(true, Ordering::Release);
    }
}

#[cfg(target_os = "macos")]
fn install_normal_exit_hook() {
    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> std::ffi::c_int;
    }
    extern "C" fn normal_exit() {
        // Never unwind through the C runtime. This is not a signal handler:
        // allocation and synchronized file writes are safe during normal exit.
        let _ = std::panic::catch_unwind(|| {
            if let Some(session) = SESSION.get() {
                finish_native_exit(session);
            }
        });
    }
    // SAFETY: The callback has static lifetime and the C ABI. Registration is
    // limited to the interactive session, not isolated PDF/speech workers.
    if unsafe { atexit(normal_exit) } != 0 {
        event("normal_exit_hook_registration_failed");
    }
}

#[cfg(any(target_os = "macos", test))]
fn finish_native_exit(session: &Session) {
    if session.finished.load(Ordering::Acquire) {
        return;
    }
    if session.cleanup_completed.load(Ordering::Acquire) {
        finish_session_at(session, "clean_native_exit");
    } else {
        append_event(session, "native_exit_before_cleanup", Value::Null);
    }
}

/// Record a lifecycle milestone without document contents or credentials.
pub fn event(name: &str) {
    event_details(name, Value::Null);
}

/// Record a bounded error description. Callers must not pass PDF text or keys.
pub fn error(name: &str, detail: &str) {
    event_details(name, Value::String(bounded_detail(detail)));
}

/// Record a panic even if it happens in the isolated PDF worker, which does
/// not own an interactive application session.
pub fn panic(thread: &str, location: &str) {
    let details = json!({
        "thread": bounded_detail(thread),
        "location": bounded_detail(location),
    });
    if let Some(session) = SESSION.get() {
        append_event(session, "panic", details);
        return;
    }
    if let Some(root) = crate::settings::app_data_dir() {
        let logs = root.join("logs");
        let _ = fs::create_dir_all(&logs);
        append_line(
            &logs.join(LOG_FILE),
            &event_line("panic", &format!("process-{}", std::process::id()), details),
        );
    }
}

/// Mark the current session as handled. Calling this more than once is safe.
pub fn finish_session(outcome: &str) {
    let Some(session) = SESSION.get() else {
        return;
    };
    finish_session_at(session, outcome);
}

fn finish_session_at(session: &Session, outcome: &str) {
    if session.finished.swap(true, Ordering::AcqRel) {
        return;
    }
    append_event(
        session,
        "session_finished",
        json!({ "outcome": bounded_detail(outcome) }),
    );
    if let Err(error) = fs::remove_file(&session.marker_path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        append_event(
            session,
            "session_marker_remove_error",
            Value::String(bounded_detail(&error.to_string())),
        );
    }
    if let Some(parent) = session.marker_path.parent() {
        let _ = crate::atomic_file::sync_directory(parent);
    }
}

fn event_details(name: &str, details: Value) {
    if let Some(session) = SESSION.get() {
        append_event(session, name, details);
    }
}

fn initialize_at(root: &Path, now_ms: u64, pid: u32) -> Session {
    let logs = root.join("logs");
    let _ = fs::create_dir_all(&logs);
    let log_path = logs.join(LOG_FILE);
    rotate_if_needed(&log_path);
    let marker_path = logs.join(SESSION_FILE);
    let previous = fs::read(&marker_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SessionMarker>(&bytes).ok());
    let id = format!("{pid}-{now_ms}");
    let session = Session {
        id: id.clone(),
        log_path,
        marker_path,
        finished: AtomicBool::new(false),
        cleanup_completed: AtomicBool::new(false),
    };

    if let Some(previous) = previous {
        append_event(
            &session,
            "previous_session_interrupted",
            json!({
                "previous_session": previous.session_id,
                "previous_pid": previous.pid,
                "previous_version": previous.version,
                "previous_started_unix_ms": previous.started_unix_ms,
                "explanation": "No clean shutdown marker was written; this includes force quit, process kill, power loss, and native crashes.",
            }),
        );
    }

    let marker = SessionMarker {
        session_id: id,
        pid,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        started_unix_ms: now_ms,
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&marker) {
        let _ = crate::atomic_file::write(&session.marker_path, &bytes);
    }
    append_event(
        &session,
        "session_started",
        json!({
            "platform": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
        }),
    );
    #[cfg(target_os = "macos")]
    if !cfg!(test) {
        import_new_macos_crash_reports(&session, &logs);
    }
    session
}

fn append_event(session: &Session, name: &str, details: Value) {
    append_line(&session.log_path, &event_line(name, &session.id, details));
}

fn event_line(name: &str, session_id: &str, details: Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({
        "timestamp": timestamp(),
        "event": bounded_detail(name),
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "session": session_id,
        "details": details,
    }))
    .unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

fn append_line(path: &Path, bytes: &[u8]) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        if file.write_all(bytes).is_ok() {
            let _ = file.sync_data();
        }
    }
}

fn rotate_if_needed(path: &Path) {
    if !fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        return;
    }
    let previous = path.with_file_name(PREVIOUS_LOG_FILE);
    let _ = fs::remove_file(&previous);
    let _ = fs::rename(path, previous);
}

fn bounded_detail(value: &str) -> String {
    value.chars().take(2_000).collect()
}

fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown-time".to_owned())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(target_os = "macos")]
fn import_new_macos_crash_reports(session: &Session, logs: &Path) {
    const IMPORTED_FILE: &str = "imported-macos-crashes.json";
    let imported_path = logs.join(IMPORTED_FILE);
    let mut imported = fs::read(&imported_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default();
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let reports_dir = PathBuf::from(home).join("Library/Logs/DiagnosticReports");
    let Ok(entries) = fs::read_dir(reports_dir) else {
        return;
    };
    let mut reports = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("LawPDF-") && name.ends_with(".ips"))
        })
        .collect::<Vec<_>>();
    reports.sort();
    for path in reports.into_iter().rev().take(20).rev() {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if imported.iter().any(|seen| seen == name) {
            continue;
        }
        append_event(
            session,
            "macos_crash_report_found",
            crash_report_summary(&path),
        );
        imported.push(name.to_owned());
    }
    if imported.len() > 100 {
        imported.drain(..imported.len() - 100);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&imported) {
        let _ = crate::atomic_file::write(&imported_path, &bytes);
    }
}

#[cfg(target_os = "macos")]
fn crash_report_summary(path: &Path) -> Value {
    use std::io::{BufRead, BufReader, Read};

    let fallback = || {
        json!({
            "report": path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown"),
            "summary": "macOS recorded a LawPDF crash; inspect the named .ips report for details.",
        })
    };
    let Ok(file) = fs::File::open(path) else {
        return fallback();
    };
    let mut reader = BufReader::new(file);
    let mut header_line = String::new();
    if reader.read_line(&mut header_line).is_err() {
        return fallback();
    }
    let mut body = String::new();
    if reader
        .take(4 * 1024 * 1024)
        .read_to_string(&mut body)
        .is_err()
    {
        return fallback();
    }
    let Ok(header) = serde_json::from_str::<Value>(&header_line) else {
        return fallback();
    };
    let Ok(report) = serde_json::from_str::<Value>(&body) else {
        return fallback();
    };
    json!({
        "report": path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown"),
        "timestamp": header.get("timestamp").cloned().unwrap_or(Value::Null),
        "app_version": header.get("app_version").cloned().unwrap_or(Value::Null),
        "os_version": header.get("os_version").cloned().unwrap_or(Value::Null),
        "exception": report.get("exception").cloned().unwrap_or(Value::Null),
        "termination": report.get("termination").cloned().unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_session_is_reported_and_clean_finish_removes_marker() {
        let root = tempfile::tempdir().unwrap();
        let first = initialize_at(root.path(), 100, 41);
        assert!(first.marker_path.exists());

        let second = initialize_at(root.path(), 200, 42);
        let log = fs::read_to_string(&second.log_path).unwrap();
        assert!(log.contains("previous_session_interrupted"));
        assert!(log.contains("41-100"));

        append_event(&second, "shutdown_complete", Value::Null);
        assert!(second.marker_path.exists());
        finish_session_at(&second, "clean");
        assert!(!second.marker_path.exists());
    }

    #[test]
    fn native_exit_requires_completed_cleanup_and_finishes_only_once() {
        let root = tempfile::tempdir().unwrap();
        let session = initialize_at(root.path(), 100, 41);
        finish_native_exit(&session);
        assert!(session.marker_path.exists());
        assert!(!session.finished.load(Ordering::Acquire));
        session.cleanup_completed.store(true, Ordering::Release);
        // A crash/kill after app cleanup must still leave an interrupted marker.
        assert!(session.marker_path.exists());
        finish_native_exit(&session);
        finish_session_at(&session, "duplicate");
        assert!(!session.marker_path.exists());
        let log = fs::read_to_string(&session.log_path).unwrap();
        assert!(log.contains("native_exit_before_cleanup"));
        assert!(log.contains("clean_native_exit"));
        assert_eq!(log.matches("session_finished").count(), 1);
        assert!(!log.contains("duplicate"));
    }

    #[test]
    fn diagnostic_details_are_bounded() {
        assert_eq!(bounded_detail(&"x".repeat(2_500)).len(), 2_000);
    }
}
