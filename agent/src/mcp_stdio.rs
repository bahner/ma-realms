use std::io::{self, BufRead, BufReader, Write};

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde_json::{json, Map, Value};

const SERVER_NAME: &str = "ma-agentd-mcp";
const SERVER_VERSION: &str = "0.1.0";
const DEFAULT_AGENTD_URL: &str = "http://127.0.0.1:5003";

#[derive(Clone)]
struct AppState {
    client: Client,
    agentd_url: String,
}

pub async fn run_mcp(agentd_url_override: Option<String>) -> Result<()> {
    let agentd_url = agentd_url_override
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_AGENTD_URL)
        .trim_end_matches('/')
        .to_string();

    let state = AppState {
        client: Client::new(),
        agentd_url,
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    while let Some(msg) = read_framed_json(&mut reader)? {
        handle_message(&state, msg, &mut writer).await?;
    }

    Ok(())
}

async fn handle_message(state: &AppState, msg: Value, writer: &mut impl Write) -> Result<()> {
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

    match method.as_str() {
        "notifications/initialized" => Ok(()),
        "initialize" => {
            if let Some(id) = id {
                let result = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                });
                write_result(writer, id, result)?;
            }
            Ok(())
        }
        "ping" => {
            if let Some(id) = id {
                write_result(writer, id, json!({}))?;
            }
            Ok(())
        }
        "tools/list" => {
            if let Some(id) = id {
                write_result(writer, id, json!({ "tools": tool_definitions() }))?;
            }
            Ok(())
        }
        "tools/call" => {
            if let Some(id) = id {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let args = params
                    .get("arguments")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();

                match call_tool(state, &name, &args).await {
                    Ok(payload) => write_result(writer, id, payload)?,
                    Err(err) => {
                        let payload = json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": err.to_string()
                                }
                            ],
                            "isError": true
                        });
                        write_result(writer, id, payload)?;
                    }
                }
            }
            Ok(())
        }
        "shutdown" => {
            if let Some(id) = id {
                write_result(writer, id, json!({}))?;
            }
            Ok(())
        }
        "exit" => Ok(()),
        _ => {
            if let Some(id) = id {
                write_error(writer, id, -32601, format!("method not found: {}", method))?;
            }
            Ok(())
        }
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "agentd_health",
            "Read ma-agentd health info from /api/v0/health.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }),
        ),
        tool(
            "agentd_get_config",
            "Read active ma-agentd runtime config.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }),
        ),
        tool(
            "agentd_update_config",
            "Update selected ma-agentd config fields.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kubo_key_alias": { "type": "string" },
                    "lock_ttl": { "type": "integer", "minimum": 0 }
                }
            }),
        ),
        tool(
            "agentd_validate_key_alias",
            "Validate whether a Kubo key alias exists.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "alias": { "type": "string" }
                }
            }),
        ),
        tool(
            "agentd_list_agents",
            "List registered agent metadata rows.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }),
        ),
        tool(
            "agentd_create_agent",
            "Create a new agent id + DID fragment.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": { "type": "string" }
                }
            }),
        ),
        tool(
            "agentd_delete_agent",
            "Delete an existing agent metadata + log files.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" }
                }
            }),
        ),
        tool(
            "agentd_get_logs",
            "Fetch agent logs with optional query + limit.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "q": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 5000 }
                }
            }),
        ),
        tool(
            "agentd_append_log",
            "Append one line to an agent log.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id", "line"],
                "properties": {
                    "id": { "type": "string" },
                    "line": { "type": "string" }
                }
            }),
        ),
        tool(
            "agentd_raw_request",
            "Low-level escape hatch for current/future /api/v0 endpoints.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["method", "path"],
                "properties": {
                    "method": { "type": "string", "enum": ["GET", "POST", "DELETE"] },
                    "path": { "type": "string" },
                    "query": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    },
                    "body": {
                        "type": "object",
                        "additionalProperties": true
                    }
                }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

async fn call_tool(state: &AppState, name: &str, args: &Map<String, Value>) -> Result<Value> {
    let result = match name {
        "agentd_health" => do_get(state, "/api/v0/health", None).await?,
        "agentd_get_config" => do_get(state, "/api/v0/config", None).await?,
        "agentd_update_config" => {
            let mut body = Map::new();
            if let Some(value) = string_arg(args, "kubo_key_alias") {
                body.insert("kubo_key_alias".to_string(), Value::String(value));
            }
            if let Some(value) = int_arg(args, "lock_ttl") {
                body.insert("lock_ttl".to_string(), Value::Number(value.into()));
            }
            do_post(state, "/api/v0/config/update", Value::Object(body)).await?
        }
        "agentd_validate_key_alias" => {
            let mut query = Vec::new();
            if let Some(alias) = string_arg(args, "alias") {
                query.push(("alias".to_string(), alias));
            }
            do_get(state, "/api/v0/config/validate-key-alias", Some(query)).await?
        }
        "agentd_list_agents" => do_get(state, "/api/v0/agents", None).await?,
        "agentd_create_agent" => {
            let mut body = Map::new();
            if let Some(id) = string_arg(args, "id") {
                body.insert("id".to_string(), Value::String(id));
            }
            do_post(state, "/api/v0/agents/create", Value::Object(body)).await?
        }
        "agentd_delete_agent" => {
            let id = required_string_arg(args, "id")?;
            do_delete(state, &format!("/api/v0/agents/{}", id)).await?
        }
        "agentd_get_logs" => {
            let id = required_string_arg(args, "id")?;
            let mut query = Vec::new();
            if let Some(q) = string_arg(args, "q") {
                query.push(("q".to_string(), q));
            }
            if let Some(limit) = int_arg(args, "limit") {
                query.push(("limit".to_string(), limit.to_string()));
            }
            do_get(state, &format!("/api/v0/agents/{}/logs", id), Some(query)).await?
        }
        "agentd_append_log" => {
            let id = required_string_arg(args, "id")?;
            let line = required_string_arg(args, "line")?;
            do_post(
                state,
                &format!("/api/v0/agents/{}/log", id),
                json!({ "line": line }),
            )
            .await?
        }
        "agentd_raw_request" => {
            let method = required_string_arg(args, "method")?.to_uppercase();
            let path = required_string_arg(args, "path")?;

            let query = args
                .get("query")
                .and_then(Value::as_object)
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|sv| (k.clone(), sv.to_string())))
                        .collect::<Vec<_>>()
                });

            match method.as_str() {
                "GET" => do_get(state, &path, query).await?,
                "DELETE" => do_delete(state, &path).await?,
                "POST" => {
                    let body = args
                        .get("body")
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Map::new()));
                    do_post(state, &path, body).await?
                }
                _ => {
                    return Err(anyhow!(
                        "agentd_raw_request.method must be one of GET, POST, DELETE"
                    ));
                }
            }
        }
        _ => return Err(anyhow!("unknown tool: {}", name)),
    };

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": pretty_json(&result)
            }
        ]
    }))
}

async fn do_get(
    state: &AppState,
    path: &str,
    query: Option<Vec<(String, String)>>,
) -> Result<Value> {
    let url = endpoint(&state.agentd_url, path)?;
    let req = state.client.get(url);
    let req = if let Some(query) = query {
        req.query(&query)
    } else {
        req
    };
    parse_response(req.send().await?).await
}

async fn do_post(state: &AppState, path: &str, body: Value) -> Result<Value> {
    let url = endpoint(&state.agentd_url, path)?;
    let response = state.client.post(url).json(&body).send().await?;
    parse_response(response).await
}

async fn do_delete(state: &AppState, path: &str) -> Result<Value> {
    let url = endpoint(&state.agentd_url, path)?;
    let response = state.client.delete(url).send().await?;
    parse_response(response).await
}

async fn parse_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let text = response.text().await?;
    let parsed = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "raw": text }));

    if status.is_success() {
        Ok(parsed)
    } else {
        Err(anyhow!(
            "request failed with status {}: {}",
            status,
            pretty_json(&parsed)
        ))
    }
}

fn endpoint(base: &str, path: &str) -> Result<String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(anyhow!("agentd base url is empty"));
    }

    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    Ok(format!("{}{}", base, normalized_path))
}

fn required_string_arg(args: &Map<String, Value>, key: &str) -> Result<String> {
    string_arg(args, key).ok_or_else(|| anyhow!("missing required argument '{}'", key))
}

fn string_arg(args: &Map<String, Value>, key: &str) -> Option<String> {
    let value = args.get(key)?.as_str()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn int_arg(args: &Map<String, Value>, key: &str) -> Option<i64> {
    args.get(key)?.as_i64()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn write_result(writer: &mut impl Write, id: Value, result: Value) -> Result<()> {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    });
    write_framed_json(writer, &msg)
}

fn write_error(writer: &mut impl Write, id: Value, code: i64, message: String) -> Result<()> {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    });
    write_framed_json(writer, &msg)
}

fn write_framed_json(writer: &mut impl Write, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn read_framed_json(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    let mut saw_any_header = false;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            if saw_any_header {
                return Err(anyhow!("unexpected EOF while reading MCP headers"));
            }
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        saw_any_header = true;

        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                let parsed = value.trim().parse::<usize>()
                    .with_context(|| format!("invalid Content-Length header: {}", value.trim()))?;
                content_length = Some(parsed);
            }
        }
    }

    let len = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;

    let value = serde_json::from_slice::<Value>(&body)
        .with_context(|| "failed to parse MCP JSON payload")?;
    Ok(Some(value))
}