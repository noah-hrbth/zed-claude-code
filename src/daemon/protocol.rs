//! Minimal JSON-RPC / MCP request handling for the Claude Code IDE protocol.
//!
//! Claude Code treats the IDE as an MCP server. On connect it will send `initialize`
//! and `tools/list` requests and expect responses within a few seconds, otherwise it
//! drops the connection. We answer both with well-formed responses so the connection
//! stays open and our pushed `at_mentioned` notifications reach Claude.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Deserialize)]
pub struct Incoming {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

#[derive(Debug, Serialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
}

/// Build a response to an incoming request. Returns None if the message is a
/// notification (no id) — notifications get no reply.
pub fn handle(raw: &str) -> Option<String> {
    let msg: Incoming = match serde_json::from_str(raw) {
        Ok(m) => m,
        Err(_) => return None,
    };
    let id = msg.id?;
    let method = msg.method.unwrap_or_default();

    let response = match method.as_str() {
        "initialize" => Response {
            jsonrpc: "2.0",
            id,
            result: Some(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "zcc", "version": env!("CARGO_PKG_VERSION") },
            })),
            error: None,
        },
        "tools/list" => Response {
            jsonrpc: "2.0",
            id,
            result: Some(json!({ "tools": [] })),
            error: None,
        },
        "tools/call" => Response {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(ErrorObject {
                code: -32601,
                message: "no tools available".into(),
            }),
        },
        _ => Response {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(ErrorObject {
                code: -32601,
                message: format!("method not found: {method}"),
            }),
        },
    };

    serde_json::to_string(&response).ok()
}
