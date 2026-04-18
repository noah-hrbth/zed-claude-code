use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Stable short hash of a worktree path. Used to name per-worktree IPC sockets.
pub fn worktree_hash(worktree: &Path) -> String {
    let canonical = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..6])
}

/// Path to the per-worktree Unix IPC socket.
pub fn ipc_socket_path(worktree: &Path) -> Result<PathBuf> {
    let tmp = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let name = format!("zcc-{}.sock", worktree_hash(worktree));
    Ok(tmp.join(name))
}

/// Directory containing Claude Code IDE lockfiles.
pub fn claude_ide_dir() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Ok(PathBuf::from(custom).join("ide"));
    }
    let home = dirs::home_dir().context("no home directory")?;
    Ok(home.join(".claude").join("ide"))
}
