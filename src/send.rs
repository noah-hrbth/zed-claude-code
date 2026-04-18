use crate::daemon::ipc::{IpcPayload, IpcReply};
use crate::{log_debug, log_info, log_warn};
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const SUB: &str = "send";

/// Budget for bringing up a fresh daemon on first-ever `zcc send`.
const SPAWN_BUDGET: Duration = Duration::from_millis(1500);
const SPAWN_TICK: Duration = Duration::from_millis(25);

pub fn run(worktree: PathBuf, file: PathBuf, row: u32, selection: String) -> Result<()> {
    let worktree = worktree
        .canonicalize()
        .with_context(|| format!("canonicalize {}", worktree.display()))?;
    let sock = crate::util::ipc_socket_path(&worktree)?;

    let range = crate::selection::derive_from_path(&file, &selection, row)
        .unwrap_or(crate::selection::LineRange { start: row, end: row });
    log_info!(SUB, "selection file={} row={row} start={} end={} selection_len={}",
        file.display(), range.start, range.end, selection.len());

    let payload = IpcPayload::AtMentioned {
        file_path: file.to_string_lossy().into_owned(),
        line_start: range.start,
        line_end: range.end,
    };

    // Try existing daemon first.
    if let Ok(stream) = UnixStream::connect(&sock) {
        send_payload(stream, &payload)?;
        log_info!(SUB, "forwarded selection to existing daemon sock={}", sock.display());
        return Ok(());
    }

    log_info!(SUB, "no daemon running; spawning for sock={}", sock.display());
    crate::fork::spawn_daemon(&worktree)?;

    let deadline = Instant::now() + SPAWN_BUDGET;
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(&sock) {
            break stream;
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "daemon failed to come up within {:?}; see {}",
                SPAWN_BUDGET,
                crate::logging::log_dir()?.join("zcc.log").display()
            );
        }
        std::thread::sleep(SPAWN_TICK);
    };
    send_payload(stream, &payload)?;
    log_info!(SUB, "forwarded selection to freshly spawned daemon sock={}", sock.display());
    Ok(())
}

fn send_payload(mut stream: UnixStream, payload: &IpcPayload) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let bytes = serde_json::to_vec(payload)?;
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while buf.len() < 4096 {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(_) => break,
        }
    }
    match serde_json::from_slice::<IpcReply>(&buf) {
        Ok(IpcReply::Ok { clients }) => {
            if clients == 0 {
                log_warn!(SUB, "daemon accepted message but no Claude client is attached (run /ide inside Claude Code)");
            }
        }
        Ok(IpcReply::Err { message }) => anyhow::bail!("daemon rejected: {message}"),
        Err(err) => log_debug!(SUB, "reply parse failed (ignored): {err}"),
    }
    Ok(())
}
