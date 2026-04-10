# ma-agentd MCP setup (localhost:5003)

This document describes how to expose ma-agentd as an MCP server for AI clients.

## 1. Start ma-agentd

From ma-realms root:

```bash
make MA_AGENT_LISTEN=127.0.0.1:5003 dev-agent
```

If port 5003 is already in use, verify that an existing ma-agentd process is healthy:

```bash
curl -sS http://127.0.0.1:5003/api/v0/health
```

## 2. Start MCP server (stdio)

From ma-realms root:

```bash
cargo run --manifest-path agent/Cargo.toml --bin ma-agent -- --mcp --agentd-url http://127.0.0.1:5003
```

`ma-agent --mcp` speaks MCP over stdio and proxies calls into `/api/v0/*` on ma-agentd.

## 3. Supported MCP tools

1. agentd_health
2. agentd_get_config
3. agentd_update_config
4. agentd_validate_key_alias
5. agentd_list_agents
6. agentd_create_agent
7. agentd_delete_agent
8. agentd_get_logs
9. agentd_append_log
10. agentd_raw_request

## 4. Important scope note

Current ma-agentd v1 provides daemon/admin APIs. The world message loop endpoints (`poll/send/ack`) are not implemented yet in current source.

That means this MCP server is production-usable for:

- daemon health/config management
- agent identity lifecycle
- per-agent log IO
- future endpoint access via agentd_raw_request

## 5. VS Code MCP config

A ready-to-use config is provided in `.vscode/mcp.json`.

If your MCP client supports env overrides, set `MA_AGENTD_URL` and pass through to `--agentd-url` when needed.
