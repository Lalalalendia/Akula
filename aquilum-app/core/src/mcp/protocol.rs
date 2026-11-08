use super::tools;
use serde::Deserialize;
use serde_json::{json, Value};

pub type ToolCaller<'a> = &'a dyn Fn(&str, &Value) -> Result<Value, String>;

pub(crate) const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "aquilum";

const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const PARSE_ERROR: i32 = -32700;

#[derive(Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

// The current Akula transport is a legacy, handshake-based MCP server.
// Require the fields of an InitializeRequest instead of silently treating
// a malformed initialization as a negotiated, supported protocol version.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    protocol_version: String,
    capabilities: Value,
    client_info: ClientInfo,
}

#[derive(Deserialize)]
struct ClientInfo {
    name: String,
    version: String,
}

// Admission is decided from the entire JSON-RPC message. A mixed batch,
// unknown method or malformed member must stay on the exclusive lane.
// This is a fixed server-side allowlist, never a claim supplied by the client.
pub fn allows_parallel_read(body: &str) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    match message {
        Value::Array(items) => !items.is_empty() && items.iter().all(parallel_read_member),
        item => parallel_read_member(&item),
    }
}

fn parallel_read_member(message: &Value) -> bool {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return false;
    };
    match method {
        "initialize" | "ping" | "tools/list" => true,
        "tools/call" => matches!(
            message
                .get("params")
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str),
            Some("read_note" | "note_outline")
        ),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum ToolVisibility {
    Full,
    None,
    #[cfg(test)]
    ScopedReadFixture,
}

pub fn handle_message(body: &str, call: ToolCaller<'_>) -> Option<String> {
    handle_with_scope(body, call, ToolVisibility::Full)
}

/// A newly authenticated agent has no note, metadata or tool authority until
/// host-granted vault scoping is implemented and proven. No tool is advertised
/// or executed in this mode; legacy dispatch is never a fallback.
pub(crate) fn handle_without_tools(body: &str) -> Option<String> {
    let never_call = |_name: &str, _args: &Value| -> Result<Value, String> {
        Err("MCP_AGENT_SCOPE_DENIED".to_owned())
    };
    handle_with_scope(body, &never_call, ToolVisibility::None)
}

/// An actual MCP request/reply, but ONLY with a disposable test-only
/// registration and no installed issuer or production MCP listener.
#[cfg(test)]
pub(crate) fn handle_fixture_scoped_read(body: &str, call: ToolCaller<'_>) -> Option<String> {
    handle_with_scope(body, call, ToolVisibility::ScopedReadFixture)
}

fn handle_with_scope(
    body: &str,
    call: ToolCaller<'_>,
    visibility: ToolVisibility,
) -> Option<String> {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(value) => value,
        Err(error) => {
            return Some(failure(Value::Null, PARSE_ERROR, &error.to_string()).to_string())
        }
    };

    match message {
        Value::Array(items) => {
            let responses = items
                .into_iter()
                .filter_map(|item| handle_single(item, call, visibility))
                .collect::<Vec<_>>();
            (!responses.is_empty()).then(|| Value::Array(responses).to_string())
        }
        single => handle_single(single, call, visibility).map(|value| value.to_string()),
    }
}

fn handle_single(
    message: Value,
    call: ToolCaller<'_>,
    visibility: ToolVisibility,
) -> Option<Value> {
    let request = match serde_json::from_value::<RpcRequest>(message) {
        Ok(request) => request,
        Err(error) => return Some(failure(Value::Null, PARSE_ERROR, &error.to_string())),
    };
    let id = request.id.filter(|id| !id.is_null())?;

    Some(match dispatch(&request.method, request.params, call, visibility) {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => failure(id, code, &message),
    })
}

fn dispatch(
    method: &str,
    params: Option<Value>,
    call: ToolCaller<'_>,
    visibility: ToolVisibility,
) -> Result<Value, (i32, String)> {
    match method {
        "initialize" => {
            let request = params.ok_or_else(|| {
                (INVALID_PARAMS, "initialize.params is required".to_owned())
            })?;
            let request: InitializeParams = serde_json::from_value(request)
                .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
            if request.protocol_version.trim().is_empty()
                || !request.capabilities.is_object()
                || request.client_info.name.trim().is_empty()
                || request.client_info.version.trim().is_empty()
            {
                return Err((INVALID_PARAMS, "Invalid initialize parameters".to_owned()));
            }

            // MCP 2025-06-18 lifecycle: match supported versions; otherwise
            // counter-offer a version this server actually implements.
            // Do not echo the caller's version and falsely claim 2026 support.
            // The client must disconnect if it cannot speak the counter-offer.
            Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                "instructions": match visibility {
                    ToolVisibility::None => {
                        "Authenticated MCP agent. No tools are authorized. Request access from the Akula host."
                    }
                    ToolVisibility::Full => tools::INSTRUCTIONS,
                    #[cfg(test)]
                    ToolVisibility::ScopedReadFixture => {
                        "Disposable CI-only scoped read fixture. Not an installed host issuer."
                    }
                },
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let visible = match visibility {
                ToolVisibility::None => Vec::new(),
                ToolVisibility::Full => tools::definitions(),
                #[cfg(test)]
                ToolVisibility::ScopedReadFixture => vec![json!({
                    "name": "read_note",
                    "description": "Disposable exact-path scoped-read fixture",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "vaultId": {"type": "string"},
                            "note": {"type": "string"}
                        },
                        "required": ["vaultId", "note"],
                        "additionalProperties": false
                    }
                })],
            };
            Ok(json!({ "tools": visible }))
        }
        "tools/call" => {
            if matches!(visibility, ToolVisibility::None) {
                return Ok(tool_result(Err("MCP_AGENT_SCOPE_DENIED".to_owned())));
            }
            let request = serde_json::from_value::<ToolCall>(params.unwrap_or(Value::Null))
                .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
            #[cfg(test)]
            if matches!(visibility, ToolVisibility::ScopedReadFixture)
                && request.name != "read_note"
            {
                return Ok(tool_result(Err("MCP_AGENT_SCOPE_DENIED".to_owned())));
            }
            Ok(tool_result(call(&request.name, &request.arguments)))
        }
        _ => Err((METHOD_NOT_FOUND, format!("Неизвестный метод: {method}"))),
    }
}

fn tool_result(outcome: Result<Value, String>) -> Value {
    let (text, is_error) = match outcome {
        Ok(value) => (
            serde_json::to_string(&value).unwrap_or_else(|error| error.to_string()),
            false,
        ),
        Err(message) => (message, true),
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_tools(_name: &str, _arguments: &Value) -> Result<Value, String> {
        Err("не должно вызываться".to_owned())
    }

    fn handle(body: &str) -> Option<Value> {
        handle_message(body, &no_tools).map(|value| serde_json::from_str(&value).unwrap())
    }

    #[test]
    fn scoped_mode_hides_tool_names_and_denies_every_batched_tool_call() {
        let body = json!([
            { "jsonrpc":"2.0", "id":1, "method":"tools/list" },
            { "jsonrpc":"2.0", "id":2, "method":"tools/call",
              "params": { "name":"read_note", "arguments": { "workspace":"foreign", "note":"secret.md" } } },
            { "jsonrpc":"2.0", "id":3, "method":"tools/call",
              "params": { "name":"list_workspaces" } },
        ]);
        let response: Value = serde_json::from_str(
            &handle_without_tools(&body.to_string()).unwrap()
        ).unwrap();
        let replies = response.as_array().unwrap();
        assert_eq!(replies.len(), 3);
        assert!(replies[0]["result"]["tools"].as_array().unwrap().is_empty());
        let initialized: Value = serde_json::from_str(
            &handle_without_tools(&json!({
                "jsonrpc": "2.0", "id": 9, "method": "initialize",
                "params": { "protocolVersion": PROTOCOL_VERSION,
                            "capabilities": {},
                            "clientInfo": {"name":"scoped","version":"1.0"} }
            }).to_string()).unwrap()
        ).unwrap();
        let instructions = initialized["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("No tools are authorized"));
        assert!(!instructions.contains("switch_workspace"));

        for result in [&replies[1], &replies[2]] {
            assert_eq!(result["result"]["isError"], true);
            let message = result["result"]["content"][0]["text"].as_str().unwrap();
            assert_eq!(message, "MCP_AGENT_SCOPE_DENIED");
            assert!(!message.contains("secret.md"));
            assert!(!message.contains("foreign"));
        }
    }

    #[test]
    fn parallel_read_allowlist_fails_closed_for_unknown_and_mixed_batches() {
        for name in ["read_note", "note_outline"] {
            let request = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": name, "arguments": { "note": "safe.md" } }
            });
            assert!(allows_parallel_read(&request.to_string()), "{name}");
        }
        for method in ["initialize", "ping", "tools/list"] {
            assert!(allows_parallel_read(
                &json!({ "jsonrpc": "2.0", "id": 1, "method": method }).to_string()
            ));
        }
        assert!(!allows_parallel_read("{ broken"));
        assert!(!allows_parallel_read("[]"));
        assert!(!allows_parallel_read(
            &json!({ "method": "tools/call", "params": { "name": "update_note" } }).to_string()
        ));
        assert!(!allows_parallel_read(
            &json!({ "method": "tools/call", "params": { "name": "search_notes" } }).to_string()
        ));
        assert!(!allows_parallel_read(
            &json!({ "method": "tools/call", "params": { "name": "switch_workspace" } }).to_string()
        ));
        assert!(!allows_parallel_read(
            &json!([
                { "id": 1, "method": "tools/call", "params": { "name": "read_note" } },
                { "id": 2, "method": "tools/call", "params": { "name": "update_note" } }
            ]).to_string()
        ));
        assert!(!allows_parallel_read(
            &json!([
                { "id": 1, "method": "tools/call", "params": { "name": "read_note" } },
                { "id": 2, "method": "unknown" }
            ]).to_string()
        ));
        assert!(!allows_parallel_read(
            &json!([
                { "id": 1, "method": "tools/call", "params": { "name": "read_note" } },
                4
            ]).to_string()
        ));
        assert!(allows_parallel_read(
            &json!([
                { "id": 1, "method": "tools/call", "params": { "name": "read_note" } },
                { "id": 2, "method": "tools/call", "params": { "name": "note_outline" } }
            ]).to_string()
        ));
    }

    fn initialize_with(version: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": { "name": "Akula test", "version": "1.0.0" }
            }
        })
    }

    #[test]
    fn negotiates_only_a_version_the_legacy_server_implements() {
        for requested in [
            "2025-06-18",
            "2025-03-26",
            "2025-11-25",
            "2026-07-28",
            "2099-01-01",
        ] {
            let response = handle(&initialize_with(requested).to_string()).unwrap();
            assert_eq!(
                response["result"]["protocolVersion"],
                PROTOCOL_VERSION,
                "initialize must not claim a version it cannot serve: {requested}"
            );
            assert!(response["result"]["capabilities"]["tools"].is_object());
            assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
            assert!(response["error"].is_null());
        }
    }

    #[test]
    fn rejects_malformed_initialization_instead_of_claiming_success() {
        for params in [
            json!(null),
            json!({}),
            json!({ "protocolVersion": "" }),
            json!({ "protocolVersion": 2026, "capabilities": {}, "clientInfo": {} }),
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": [],
                "clientInfo": { "name": "agent", "version": "1" }
            }),
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "", "version": "1" }
            }),
        ] {
            let body = json!({
                "jsonrpc": "2.0", "id": 5, "method": "initialize", "params": params
            });
            let response = handle(&body.to_string()).unwrap();
            assert_eq!(response["id"], 5);
            assert_eq!(response["error"]["code"], INVALID_PARAMS, "{body}");
            assert!(response["result"].is_null());
        }
    }

    #[test]
    fn keeps_silent_on_notifications() {
        assert!(handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert!(handle(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).is_none());
    }

    #[test]
    fn reports_unknown_method_as_rpc_error() {
        let response = handle(r#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#).unwrap();
        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["id"], 7);
    }

    #[test]
    fn reports_broken_json_without_panicking() {
        let response = handle("{ не json").unwrap();
        assert_eq!(response["error"]["code"], -32700);
    }

    #[test]
    fn answers_every_request_of_a_batch_and_skips_notifications() {
        let response = handle(
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},
                {"jsonrpc":"2.0","method":"notifications/initialized"},
                {"jsonrpc":"2.0","id":2,"method":"tools/list"}]"#,
        )
        .unwrap();
        let items = response.as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[1]["result"]["tools"].as_array().unwrap().len() > 10);
    }

    #[test]
    fn returns_tool_failure_as_result_with_error_flag() {
        let failing = |_name: &str, _arguments: &Value| Err("Заметка не найдена".to_owned());
        let raw = handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_note","arguments":{}}}"#,
            &failing,
        )
        .unwrap();
        let response: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["text"], "Заметка не найдена");
        assert!(response["error"].is_null());
    }

    #[test]
    fn every_tool_declares_a_json_schema() {
        for tool in tools::definitions() {
            assert!(tool["name"].as_str().is_some_and(|name| !name.is_empty()));
            assert!(tool["description"].as_str().is_some_and(|value| !value.is_empty()));
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }
}
