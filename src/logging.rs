use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Location of the log directory on macOS.
pub fn log_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot locate home directory")?;
    Ok(home.join("Library").join("Logs").join("zcc"))
}

fn log_path() -> Result<PathBuf> {
    Ok(log_dir()?.join("zcc.log"))
}

fn lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

pub fn init(_subcommand: &'static str) -> Result<()> {
    let dir = log_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create log dir {}", dir.display()))?;
    Ok(())
}

pub fn log(level: &str, subcommand: &str, msg: impl std::fmt::Display) {
    let Ok(path) = log_path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _guard = lock().lock();
    rotate_if_needed(&path);
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else { return };
    let ts = chrono::Utc::now().to_rfc3339();
    let pid = std::process::id();
    let raw = msg.to_string();
    let escaped = json_escape(&raw);
    let _ = writeln!(
        f,
        r#"{{"ts":"{ts}","level":"{level}","subcommand":"{subcommand}","pid":{pid},"msg":"{escaped}"}}"#,
    );
}

fn rotate_if_needed(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.len() < MAX_LOG_BYTES {
        return;
    }
    // Shift: zcc.log.2 -> delete; zcc.log.1 -> zcc.log.2; zcc.log -> zcc.log.1
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let base = path.file_name().and_then(|s| s.to_str()).unwrap_or("zcc.log");
    let p2 = parent.join(format!("{base}.2"));
    let p1 = parent.join(format!("{base}.1"));
    let _ = std::fs::remove_file(&p2);
    let _ = std::fs::rename(&p1, &p2);
    let _ = std::fs::rename(path, &p1);
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[macro_export]
macro_rules! log_info  { ($sub:expr, $($arg:tt)*) => { $crate::logging::log("info",  $sub, format_args!($($arg)*)) }; }
#[macro_export]
macro_rules! log_warn  { ($sub:expr, $($arg:tt)*) => { $crate::logging::log("warn",  $sub, format_args!($($arg)*)) }; }
#[macro_export]
macro_rules! log_error { ($sub:expr, $($arg:tt)*) => { $crate::logging::log("error", $sub, format_args!($($arg)*)) }; }
#[macro_export]
macro_rules! log_debug { ($sub:expr, $($arg:tt)*) => { $crate::logging::log("debug", $sub, format_args!($($arg)*)) }; }
