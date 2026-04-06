# ma-agentd v1 contract (agent-first)

Formaal: Gi andre agenter en stabil, minimal kontrakt for local integration.

## 1. Drift og scope

- Tjeneste: ma-agentd
- Bind: 127.0.0.1:5003 (default)
- API base: /api/v0
- Miljo: localhost only
- Rolle: ikke world; agent runtime og administrasjon

## 2. Identitetsmodell (target)

- En Kubo key for world/root identity per slug
- Hver agent er et fragment under samme DID-root
- DID eksempel: did:ma:<world_root>#<agent_id>
- Agentd eier publish lifecycle for DID dokument

## 3. Persistens

- Config: ~/.config/ma/<slug>.yaml
- Data: $XDG_DATA_HOME/ma/<slug>/
- Agent metadata: agents/<agent_id>.json
- Logs: logs/<agent_id>.log
- Event store (target): events.db (SQLite)

## 4. Event envelope (canonical)

Alle innkommende lanes foldes til ett felles eventformat per agent:

{
  "event_id": "evt_...",
  "agent_id": "npc_1",
  "received_at": "2026-04-05T12:34:56Z",
  "lane": "inbox",
  "content_type": "application/x-ma-*",
  "room": "lobby",
  "from_did": "did:ma:...#...",
  "to_did": "did:ma:...#npc_1",
  "reply_to": "msg_or_event_id_or_null",
  "message_id": "msg_...",
  "body": "text or encoded payload",
  "raw": { "optional": "transport specific" }
}

Regler:

1. FIFO per agent, sortert paa received_at asc.
2. reply_to beholdes alltid hvis tilgjengelig.
3. lane beholdes som metadata, men consumer kan ignorere lane.

## 5. Auth/session (v1)

- Token per agent session
- Token sendes i Authorization header: Bearer <token>
- Token scope: poll, send, admin
- localhost-only i v1; strengere policy senere

## 6. API endpoints (v1)

### 6.1 Health

GET /api/v0/health

Response:

{
  "ok": true,
  "slug": "panteia-agent",
  "listen": "127.0.0.1:5003",
  "kubo_api_url": "http://127.0.0.1:5001"
}

### 6.2 Create agent

POST /api/v0/agents/create

Request:

{
  "id": "npc_1"
}

Response:

{
  "ok": true,
  "message": "agent 'npc_1' created",
  "agent": {
    "id": "npc_1",
    "did": "did:ma:<root>#npc_1",
    "created_at": "...",
    "log_path": ".../logs/npc_1.log"
  },
  "session": {
    "token": "...",
    "expires_at": "..."
  }
}

### 6.3 List agents

GET /api/v0/agents

Response:

{
  "ok": true,
  "agents": [
    {
      "id": "npc_1",
      "did": "did:ma:<root>#npc_1",
      "updated_at": "...",
      "log_path": "..."
    }
  ]
}

### 6.4 Delete agent

DELETE /api/v0/agents/{id}

Response:

{
  "ok": true,
  "message": "agent 'npc_1' deleted"
}

### 6.5 Poll

POST /api/v0/agents/{id}/poll

Request:

{
  "max": 1,
  "wait_ms": 15000,
  "ack_mode": "manual|auto"
}

Response:

{
  "ok": true,
  "events": [
    {
      "event_id": "evt_...",
      "lane": "inbox",
      "body": "hello",
      "reply_to": null
    }
  ]
}

Semantics:

1. max default 1.
2. wait_ms gir long-poll.
3. manual ack krever eget kall.

### 6.6 Ack

POST /api/v0/agents/{id}/ack

Request:

{
  "event_ids": ["evt_1", "evt_2"]
}

Response:

{
  "ok": true,
  "acked": 2
}

### 6.7 Send

POST /api/v0/agents/{id}/send

Request:

{
  "room": "lobby",
  "to": "did:ma:...#target_or_null",
  "mode": "chat|whisper|broadcast|command",
  "body": "text payload",
  "reply_to": "evt_or_msg_id_or_null"
}

Response:

{
  "ok": true,
  "message_id": "msg_...",
  "transport": "ma/inbox/1"
}

## 7. Call model for AI

Agent consumer model (inbox-first):

1. poll()
2. call(event) -> action
3. if action.send then send()
4. ack(event)

Action schema (recommended):

{
  "action": "ignore|send",
  "reason": "short machine-readable reason",
  "send": {
    "mode": "chat|whisper|broadcast|command",
    "room": "lobby",
    "to": null,
    "body": "...",
    "reply_to": "evt_..."
  }
}

## 8. Logging contract

Per agent log line (append-only):

<iso8601> <level> <event_or_action> <json>

Eksempel:

2026-04-05T12:00:00Z INFO poll {"event_id":"evt_1","lane":"inbox"}
2026-04-05T12:00:01Z INFO send {"message_id":"msg_9","reply_to":"evt_1"}

Log search API:

GET /api/v0/agents/{id}/logs?q=<text>&limit=<n>

## 9. Invariants

1. Agentd skal ikke starte/stoppe iroh node per agent.
2. Agent data skal overleve restart.
3. Poll/send for ett token skal ikke lekke events mellom agenter.
4. reply_to skal aldri droppes hvis source har den.

## 10. Transition notes

Naa:

- Basic daemon routes finnes.
- Per-agent metadata/log file finnes.

Neste:

1. Root key + fragment DID path for create agent.
2. Unified FIFO event queue per agent.
3. Poll/send/ack with token scopes.
4. Explicit message_id/reply_to plumbing end-to-end.
