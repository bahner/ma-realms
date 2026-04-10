# ma-agentd v1 contract (agent-first)

Formaal: gi AI-klienter en stabil, lokal kontrakt mot ma-agentd og MCP-broen.

## 1. Scope

- Service: ma-agentd
- Default bind: 127.0.0.1:5003
- API base: /api/v0
- Miljo: localhost
- Rolle i dagens v1: daemon/admin API for agent metadata, config og logger

Merk: full world message runtime (`poll/send/ack`) er ikke implementert i dagens kilde.

## 2. Identity model

- En Kubo key alias brukes som world/root DID for slug.
- Hver agent er et fragment under samme root.
- DID-format: did:ma:<root>#<agent_id>

## 3. Persistence

- Config: ~/.config/ma/agentd.yaml
- Data root: $XDG_DATA_HOME/ma/agentd/
- Agent metadata: agents/<id>.json
- Agent logs: logs/<id>.log

## 4. Implemented REST endpoints

### 4.1 Health

- Method: GET
- Path: /api/v0/health

### 4.2 Config read/update

- GET /api/v0/config
- POST /api/v0/config/update
- GET /api/v0/config/validate-key-alias?alias=<name>

### 4.3 Agent lifecycle

- GET /api/v0/agents
- POST /api/v0/agents/create
- DELETE /api/v0/agents/{id}

### 4.4 Agent logs

- GET /api/v0/agents/{id}/logs?q=<text>&limit=<n>
- POST /api/v0/agents/{id}/log

## 5. Not implemented yet (roadmap)

- POST /api/v0/agents/{id}/poll
- POST /api/v0/agents/{id}/ack
- POST /api/v0/agents/{id}/send

## 6. MCP bridge (stdio)

Binary:

- ma-agent (mcp mode)

Start manually:

```bash
cargo run --manifest-path agent/Cargo.toml --bin ma-agent -- --mcp --agentd-url http://127.0.0.1:5003
```

MCP tools provided:

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

`agentd_raw_request` gir en framtids-sikker pass-through mot nye /api/v0-ruter.

## 7. One-line runbook

1. Start daemon: `make MA_AGENT_LISTEN=127.0.0.1:5003 dev-agent`
2. Start MCP server: `make MA_AGENTD_URL=http://127.0.0.1:5003 dev-agent-mcp`
3. Connect MCP client via stdio server config in `.vscode/mcp.json`
