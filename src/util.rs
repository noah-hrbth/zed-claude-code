use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, Permissions};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
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

/// Real uid of this process. zcc never drops privileges, so this equals the
/// effective uid and is the identity to compare IPC peers against.
pub fn current_uid() -> u32 {
    // safe: getuid is always successful and has no preconditions
    unsafe { libc::getuid() }
}

/// Resolve the `$TMPDIR` base, falling back to `/tmp`. The fallback is only
/// safe because [`runtime_dir`] nests a `0700` per-uid subdir under it and
/// verifies ownership/mode before use.
fn tmp_base() -> PathBuf {
    std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Private per-user runtime directory holding the IPC socket and message queue.
/// Created `0700` and parent of both, so neither relies on `$TMPDIR`'s mode.
pub fn runtime_dir() -> Result<PathBuf> {
    runtime_dir_in(&tmp_base())
}

/// [`runtime_dir`] with an injectable base, for tests. Creates `<base>/zcc-<uid>`,
/// forces it to `0700`, then re-stats (no symlink follow) and refuses unless it
/// is a real dir owned by us with mode exactly `0700`. Fails closed: a squatted
/// path errors rather than being reclaimed.
pub fn runtime_dir_in(base: &Path) -> Result<PathBuf> {
    let uid = current_uid();
    let dir = base.join(format!("zcc-{uid}"));
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    fs::set_permissions(&dir, Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0700 {}", dir.display()))?;

    // verify after chmod; symlink_metadata so a symlinked path is rejected, not followed
    let meta = fs::symlink_metadata(&dir).with_context(|| format!("stat {}", dir.display()))?;
    if !meta.file_type().is_dir() {
        bail!("{} is not a directory (possible squatting)", dir.display());
    }
    if meta.uid() != uid {
        bail!(
            "{} owned by uid {} not {} (possible squatting)",
            dir.display(),
            meta.uid(),
            uid
        );
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o700 {
        bail!(
            "{} mode is {mode:o} not 700 (possible squatting)",
            dir.display()
        );
    }
    Ok(dir)
}

/// Path to the per-worktree Unix IPC socket, inside the private runtime dir.
pub fn ipc_socket_path(worktree: &Path) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("zcc-{}.sock", worktree_hash(worktree))))
}

/// Per-worktree directory holding queued payloads for the daemon to drain on
/// startup. One file per message (uuid-named) so concurrent writers are atomic
/// regardless of payload size. Lives inside the private runtime dir.
pub fn queue_dir(worktree: &Path) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("zcc-{}.queue", worktree_hash(worktree))))
}

/// Directory containing Claude Code IDE lockfiles.
pub fn claude_ide_dir() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Ok(PathBuf::from(custom).join("ide"));
    }
    let home = dirs::home_dir().context("no home directory")?;
    Ok(home.join(".claude").join("ide"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_dir_0700() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = runtime_dir_in(tmp.path()).unwrap();
        assert!(dir.is_dir());
        let mode = fs::symlink_metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn idempotent_on_correct_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let first = runtime_dir_in(tmp.path()).unwrap();
        let second = runtime_dir_in(tmp.path()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn rejects_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let uid = current_uid();
        let target = tmp.path().join("elsewhere");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join(format!("zcc-{uid}"))).unwrap();
        assert!(runtime_dir_in(tmp.path()).is_err());
    }

    #[test]
    fn repairs_loose_mode_we_own() {
        // a too-open dir we own is chmod-repaired to 0700, not rejected;
        // the verify-reject path is exercised by rejects_symlink (chmod
        // cannot fix a symlinked or foreign-owned path)
        let tmp = tempfile::tempdir().unwrap();
        let uid = current_uid();
        let dir = tmp.path().join(format!("zcc-{uid}"));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
        let out = runtime_dir_in(tmp.path()).unwrap();
        let mode = fs::symlink_metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
