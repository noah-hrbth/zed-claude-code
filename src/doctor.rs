use crate::install::paths::ZedPaths;
use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn run() -> Result<()> {
    println!("zcc doctor");
    check_binary_on_path();
    let zed = ZedPaths::detect()?;
    check_file(
        "tasks.json",
        &zed.tasks,
        crate::install::is_zcc_task,
        "task entry labelled 'zcc-send'",
    );
    check_file(
        "keymap.json",
        &zed.keymap,
        crate::install::is_zcc_keymap_block,
        "keymap block with Cmd+L rendezvous",
    );
    check_claude_ide_dir();
    check_runtime_dir();
    tail_log();
    Ok(())
}

fn check_binary_on_path() {
    print!("  binary on PATH... ");
    let out = std::process::Command::new("which").arg("zcc").output();
    match out {
        Ok(o) if o.status.success() => {
            let p = String::from_utf8_lossy(&o.stdout);
            println!("ok ({})", p.trim());
        }
        _ => {
            println!("not found");
            println!("    -> `brew install noah-hrbth/zcc/zcc` or add the binary to PATH");
        }
    }
}

fn check_file(label: &str, path: &Path, matches: fn(&serde_json::Value) -> bool, desc: &str) {
    print!("  {label} ({})... ", path.display());
    if !path.exists() {
        println!("missing");
        println!("    -> run `zcc install`");
        return;
    }
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(err) => {
            println!("read error: {err}");
            return;
        }
    };
    let parsed: serde_json::Value = match json5::from_str(&content) {
        Ok(v) => v,
        Err(err) => {
            println!("parse error: {err}");
            return;
        }
    };
    let arr = match parsed {
        serde_json::Value::Array(a) => a,
        _ => {
            println!("not an array");
            return;
        }
    };
    let count = arr.iter().filter(|v| matches(v)).count();
    if count == 0 {
        println!("zcc entry missing");
        println!("    -> run `zcc install`");
    } else {
        println!("ok ({count} {desc})");
    }
}

fn check_claude_ide_dir() {
    print!("  ~/.claude/ide dir... ");
    let dir = match crate::util::claude_ide_dir() {
        Ok(d) => d,
        Err(err) => {
            println!("cannot determine: {err}");
            return;
        }
    };
    if !dir.exists() {
        println!("not present yet (will be created on first Cmd+L)");
        return;
    }
    let meta = match fs::metadata(&dir) {
        Ok(m) => m,
        Err(err) => {
            println!("stat failed: {err}");
            return;
        }
    };
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    let perm_ok = mode == 0o700;
    let lockfiles: Vec<_> = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("lock"))
        .collect();
    println!(
        "{} lockfile(s), mode {:o}{}",
        lockfiles.len(),
        mode,
        if perm_ok { "" } else { " (expected 700)" }
    );
}

/// Read-only inspection of the private runtime dir. Recomputes the path inline
/// rather than calling `util::runtime_dir()`, which would create+chmod it as a
/// side effect during diagnostics.
fn check_runtime_dir() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = crate::util::current_uid();
    let base = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let dir = base.join(format!("zcc-{uid}"));
    print!("  runtime dir ({})... ", dir.display());
    let meta = match fs::symlink_metadata(&dir) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("not present yet (created on first send)");
            return;
        }
        Err(err) => {
            println!("stat failed: {err}");
            return;
        }
    };
    if !meta.file_type().is_dir() {
        println!("NOT A DIRECTORY (possible squatting)");
        return;
    }
    let mode = meta.permissions().mode() & 0o777;
    let owner_ok = meta.uid() == uid;
    let socks = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str());
            ext == Some("sock") || ext == Some("queue")
        })
        .count();
    println!(
        "mode {:o}{}{}, {} socket/queue entr{}",
        mode,
        if mode == 0o700 { "" } else { " (expected 700)" },
        if owner_ok { "" } else { ", WRONG OWNER" },
        socks,
        if socks == 1 { "y" } else { "ies" }
    );
}

fn tail_log() {
    let Ok(dir) = crate::logging::log_dir() else {
        return;
    };
    let path = dir.join("zcc.log");
    if !path.exists() {
        println!("  log: no entries yet ({})", path.display());
        return;
    }
    println!("  log tail ({}):", path.display());
    let Ok(content) = fs::read_to_string(&path) else {
        return;
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(20);
    for line in &lines[start..] {
        println!("    {line}");
    }
}
