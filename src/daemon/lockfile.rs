use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Lockfile {
    pub port: u16,
    pub pid: u32,
    #[serde(rename = "workspaceFolders")]
    pub workspace_folders: Vec<String>,
    #[serde(rename = "ideName")]
    pub ide_name: String,
    pub transport: String,
    #[serde(rename = "authToken")]
    pub auth_token: String,
}

/// Write a lockfile for this daemon. Uses atomic tmpfile + rename.
/// Returns the lockfile path.
pub fn write(port: u16, worktree: &Path, auth_token: &str) -> Result<PathBuf> {
    let dir = crate::util::claude_ide_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    fs::set_permissions(&dir, Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0700 {}", dir.display()))?;

    let lockfile = Lockfile {
        port,
        pid: std::process::id(),
        workspace_folders: vec![worktree.to_string_lossy().into_owned()],
        ide_name: "Zed".to_string(),
        transport: "ws".to_string(),
        auth_token: auth_token.to_string(),
    };
    let json = serde_json::to_vec_pretty(&lockfile)?;

    let final_path = dir.join(format!("{}.lock", port));
    let tmp_path = dir.join(format!(".{}.lock.tmp", port));

    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), final_path.display()))?;
    Ok(final_path)
}

/// Scan the lockfile directory for a lockfile matching this worktree whose PID is alive.
#[allow(dead_code)]
pub fn find_for_worktree(worktree: &Path) -> Result<Option<Lockfile>> {
    let dir = crate::util::claude_ide_dir()?;
    if !dir.exists() {
        return Ok(None);
    }
    let canonical = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_path_buf());
    let canonical_str = canonical.to_string_lossy();
    for entry in fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.path().extension().and_then(|s| s.to_str()) != Some("lock") {
            continue;
        }
        let content = match fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lf: Lockfile = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if lf.ide_name != "Zed" {
            continue;
        }
        if !lf.workspace_folders.iter().any(|w| *w == *canonical_str) {
            continue;
        }
        use nix::sys::signal;
        use nix::unistd::Pid;
        if signal::kill(Pid::from_raw(lf.pid as i32), None).is_err() {
            // Stale.
            let _ = fs::remove_file(entry.path());
            continue;
        }
        return Ok(Some(lf));
    }
    Ok(None)
}
