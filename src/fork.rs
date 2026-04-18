use anyhow::{Context, Result};
use nix::sys::wait::waitpid;
use nix::unistd::{fork, setsid, ForkResult};
use std::path::{Path, PathBuf};

/// Launch a `zcc daemon` for the given worktree, fully detached from this process.
/// Implemented as a double fork + setsid so the grandchild has no controlling terminal
/// and no parent to signal it.
pub fn spawn_daemon(worktree: &Path) -> Result<()> {
    let worktree_owned: PathBuf = worktree.to_path_buf();

    // SAFETY: `zcc send` has not started a tokio runtime or other threads yet. The single-
    // threaded parent is allowed to fork.
    let first = unsafe { fork() }.context("first fork failed")?;
    match first {
        ForkResult::Parent { child } => {
            let _ = waitpid(child, None);
            Ok(())
        }
        ForkResult::Child => intermediate(worktree_owned),
    }
}

fn intermediate(worktree: PathBuf) -> ! {
    let _ = setsid();
    // SAFETY: single-threaded child from above.
    let second = unsafe { fork() };
    match second {
        Ok(ForkResult::Parent { .. }) => std::process::exit(0),
        Ok(ForkResult::Child) => run_daemon(worktree),
        Err(_) => std::process::exit(1),
    }
}

fn run_daemon(worktree: PathBuf) -> ! {
    redirect_stdio_to_null();
    let _ = crate::logging::init("daemon");
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(err) => {
            crate::logging::log("error", "daemon", format!("runtime build failed: {err}"));
            std::process::exit(1);
        }
    };
    let result = rt.block_on(crate::daemon::run(worktree));
    let code = if result.is_ok() { 0 } else { 1 };
    if let Err(err) = result {
        crate::logging::log("error", "daemon", format!("daemon ended with error: {err}"));
    }
    std::process::exit(code);
}

fn redirect_stdio_to_null() {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;
    let Ok(dev_null) = OpenOptions::new().read(true).write(true).open("/dev/null") else {
        return;
    };
    let fd = dev_null.as_raw_fd();
    unsafe {
        libc::dup2(fd, 0);
        libc::dup2(fd, 1);
        libc::dup2(fd, 2);
    }
    // Keep dev_null alive until here; it is closed on drop.
}
