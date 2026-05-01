use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub mod paths;

use paths::ZedPaths;

pub fn run() -> Result<()> {
    let zed = ZedPaths::detect()?;
    fs::create_dir_all(&zed.config_dir)
        .with_context(|| format!("create {}", zed.config_dir.display()))?;
    patch_file(&zed.tasks, &zcc_task(), is_zcc_task, "tasks.json")?;
    patch_file_multi(
        &zed.keymap,
        &zcc_keymap_entries(),
        is_zcc_keymap_block,
        "keymap.json",
    )?;
    println!("zcc installed.");
    println!("  tasks:  {}", zed.tasks.display());
    println!("  keymap: {}", zed.keymap.display());
    println!("Originals backed up as *.bak alongside each file.");
    println!("Press Cmd+L in any Zed editor to send the selection to Claude Code.");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let zed = ZedPaths::detect()?;
    remove_from_file(&zed.tasks, is_zcc_task)?;
    remove_from_file(&zed.keymap, is_zcc_keymap_block)?;
    println!("zcc uninstalled.");
    Ok(())
}

fn patch_file(path: &Path, entry: &Value, matches: fn(&Value) -> bool, label: &str) -> Result<()> {
    backup(path)?;
    let mut arr = read_array(path, label)?;
    arr.retain(|v| !matches(v));
    arr.push(entry.clone());
    write_array(path, &arr)
}

fn patch_file_multi(
    path: &Path,
    entries: &[Value],
    matches: fn(&Value) -> bool,
    label: &str,
) -> Result<()> {
    backup(path)?;
    let mut arr = read_array(path, label)?;
    arr.retain(|v| !matches(v));
    for e in entries {
        arr.push(e.clone());
    }
    write_array(path, &arr)
}

fn remove_from_file(path: &Path, matches: fn(&Value) -> bool) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: Value =
        json5::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    let Value::Array(mut arr) = parsed else {
        return Ok(()); // not an array; don't touch
    };
    let before = arr.len();
    arr.retain(|v| !matches(v));
    if arr.len() == before {
        return Ok(());
    }
    write_array(path, &arr)
}

pub fn backup(path: &Path) -> Result<()> {
    let bak = backup_path(path);
    if path.exists() && !bak.exists() {
        fs::copy(path, &bak)
            .with_context(|| format!("backup {} -> {}", path.display(), bak.display()))?;
    }
    Ok(())
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

fn read_array(path: &Path, label: &str) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Value = json5::from_str(&content).with_context(|| {
        format!(
            "parse {} as JSONC (restore from {}.bak if needed)",
            label, label
        )
    })?;
    match parsed {
        Value::Array(a) => Ok(a),
        _ => anyhow::bail!("{} is not a JSON array (top-level must be `[...]`)", label),
    }
}

fn write_array(path: &Path, arr: &[Value]) -> Result<()> {
    let pretty = serde_json::to_string_pretty(arr)?;
    let mut bytes = pretty.into_bytes();
    bytes.push(b'\n');
    fs::write(path, &bytes).with_context(|| format!("write {}", path.display()))
}

// -------------------- zcc entries --------------------

/// The task that `Cmd+L` spawns. Selection is read from the system clipboard
/// (populated by a synthetic Cmd+C the keymap fires immediately before this
/// task). We cannot use `$ZED_SELECTED_TEXT` because Zed does not re-evaluate
/// that variable when a task is spawned from a keybinding
/// (upstream issue zed-industries/zed#40118).
///
/// Invokes `zcc` directly with `--read-clipboard` rather than going through
/// `/bin/sh -c 'pbpaste | zcc send …'`. The shell wrapper and `pbpaste` fork
/// added ~30–60 ms per press; reading the pasteboard in-process via NSPasteboard
/// is materially faster.
pub fn zcc_task() -> Value {
    json!({
        "label": "zcc-send",
        "command": "zcc",
        "args": [
            "send",
            "--read-clipboard",
            "--worktree", "$ZED_WORKTREE_ROOT",
            "--file", "$ZED_FILE",
            "--row", "$ZED_ROW"
        ],
        "use_new_terminal": false,
        "allow_concurrent_runs": true,
        "reveal": "never",
        "hide": "always",
        "show_summary": false,
        "show_command": false
    })
}

/// Three keymap blocks:
///   1. Cmd+L in Editor -> SendKeystrokes "cmd-f18 cmd-f19 cmd-f20"
///   2. Editor rendezvous: cmd-f18 -> editor::Copy, cmd-f19 -> task::Spawn zcc-send
///   3. Global rendezvous: cmd-f20 -> terminal_panel::ToggleFocus
///
/// The indirection exists because Zed keybindings have no action-array syntax
/// AND task::Spawn from a keybinding does not re-capture $ZED_SELECTED_TEXT
/// (upstream issue zed-industries/zed#40118). The synthetic Cmd+C puts the
/// selection on the system clipboard, which `zcc send --read-clipboard` reads
/// in-process via NSPasteboard.
/// F18/F19 are Editor-scoped so the copy + spawn only fire when an editor has
/// focus. F20 is global so the terminal focus shift works from any context.
/// These are unused private-use keys chosen to minimise conflict risk.
pub fn zcc_keymap_entries() -> Vec<Value> {
    vec![
        json!({
            "context": "Editor",
            "bindings": {
                "cmd-l": ["workspace::SendKeystrokes", "cmd-f18 cmd-f19 cmd-f20"]
            }
        }),
        json!({
            "context": "Editor",
            "bindings": {
                "cmd-f18": "editor::Copy",
                "cmd-f19": ["task::Spawn", { "task_name": "zcc-send" }]
            }
        }),
        json!({
            "bindings": {
                "cmd-f20": "terminal_panel::ToggleFocus"
            }
        }),
    ]
}

pub fn is_zcc_task(v: &Value) -> bool {
    v.get("label").and_then(|s| s.as_str()) == Some("zcc-send")
}

pub fn is_zcc_keymap_block(v: &Value) -> bool {
    let Some(bindings) = v.get("bindings").and_then(|b| b.as_object()) else {
        return false;
    };
    if let Some(cmd_l) = bindings.get("cmd-l").and_then(|a| a.as_array()) {
        // Any "Cmd+L -> SendKeystrokes ..." we emitted:
        if cmd_l.first().and_then(|s| s.as_str()) == Some("workspace::SendKeystrokes") {
            if let Some(seq) = cmd_l.get(1).and_then(|s| s.as_str()) {
                if seq == "cmd-f18 cmd-f19 cmd-f20"
                    || seq == "cmd-f18 cmd-f19"
                    || seq == "cmd-f19 cmd-f20"
                {
                    return true;
                }
            }
        }
        // Earlier: Cmd+L -> task::Spawn zcc-send (direct, no rendezvous).
        let is_task_spawn = cmd_l.first().and_then(|s| s.as_str()) == Some("task::Spawn")
            && cmd_l
                .get(1)
                .and_then(|p| p.get("task_name"))
                .and_then(|n| n.as_str())
                == Some("zcc-send");
        if is_task_spawn {
            return true;
        }
    }
    // Current rendezvous: cmd-f18 -> editor::Copy, cmd-f19 -> task::Spawn zcc-send
    let has_copy = bindings.get("cmd-f18").and_then(|v| v.as_str()) == Some("editor::Copy");
    let has_spawn = bindings
        .get("cmd-f19")
        .and_then(|a| a.as_array())
        .and_then(|a| a.get(1))
        .and_then(|p| p.get("task_name"))
        .and_then(|n| n.as_str())
        == Some("zcc-send");
    if has_copy && has_spawn {
        return true;
    }
    // Terminal focus block: cmd-f20 -> terminal_panel::ToggleFocus (current and legacy).
    if bindings.get("cmd-f20").and_then(|v| v.as_str()) == Some("terminal_panel::ToggleFocus") {
        return true;
    }
    // Legacy rendezvous target: cmd-f19 + cmd-f20 both in one block.
    let has_f19 = bindings.get("cmd-f19").is_some();
    let has_f20 =
        bindings.get("cmd-f20").and_then(|v| v.as_str()) == Some("terminal_panel::ToggleFocus");
    has_f19 && has_f20
}
