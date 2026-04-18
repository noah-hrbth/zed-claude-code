//! Standalone test client: simulates Claude Code's WebSocket handshake against a
//! running `zcc daemon`. Expects to be pointed at an existing lockfile.
//!
//! Usage:
//!   cargo run --example fake_claude -- <path/to/60682.lock>
//!
//! It connects, sends `initialize`, then `tools/list`, prints the responses,
//! holds the connection open for 35s (long enough to prove the daemon no longer
//! drops the connection at the 30s mark), and exits.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Deserialize)]
struct Lockfile {
    port: u16,
    #[serde(rename = "authToken")]
    auth_token: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let lockfile_path = std::env::args().nth(1).expect("pass lockfile path");
    let raw = std::fs::read_to_string(&lockfile_path)?;
    let lf: Lockfile = serde_json::from_str(&raw)?;

    let url = format!("ws://127.0.0.1:{}/", lf.port);
    println!("connecting to {url} with token {}...", &lf.auth_token[..8]);

    let mut req = url.into_client_request()?;
    req.headers_mut().insert(
        "x-claude-code-ide-authorization",
        lf.auth_token.parse()?,
    );

    let (ws, resp) = tokio_tungstenite::connect_async(req).await?;
    println!("connected. status={}", resp.status());

    let (mut tx, mut rx) = ws.split();

    // initialize
    let init = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "fake-claude", "version": "0" }
        }
    });
    tx.send(Message::Text(init.to_string().into())).await?;
    println!("-> initialize");

    // tools/list
    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    tx.send(Message::Text(list.to_string().into())).await?;
    println!("-> tools/list");

    // read until we have 2 responses or timeout
    let read_task = async {
        let mut got = 0usize;
        while let Some(msg) = rx.next().await {
            match msg? {
                Message::Text(t) => {
                    println!("<- {}", t);
                    got += 1;
                    if got >= 2 { break; }
                }
                Message::Ping(p) => println!("<- ping({} bytes)", p.len()),
                Message::Pong(_) => println!("<- pong"),
                Message::Close(_) => { println!("<- close"); break; }
                other => println!("<- (other: {:?})", other),
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(5), read_task).await??;

    println!("\nHolding connection open for 35s to ensure no drop at 30s...");
    for i in 1..=35 {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if i % 5 == 0 { println!("  {i}s elapsed; still alive"); }
            }
            msg = rx.next() => {
                match msg {
                    Some(Ok(Message::Ping(p))) => println!("  received ping at {i}s"),
                    Some(Ok(Message::Close(_))) | None => {
                        anyhow::bail!("connection closed at {}s — handshake fix not working", i);
                    }
                    Some(Ok(m)) => println!("  received {:?} at {i}s", m),
                    Some(Err(err)) => anyhow::bail!("recv error at {}s: {}", i, err),
                }
            }
        }
    }

    println!("\nSUCCESS: connection survived > 30s.");
    Ok(())
}
