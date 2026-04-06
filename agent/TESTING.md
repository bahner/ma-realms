# ma-agent / future agent testing

This document is an operator-first test matrix for local development.

## Scope

- Current executable: ma-agent (CLI)
- Near-term target: ma-agentd (local daemon)
- Local-only assumptions: localhost networking, local Kubo API

## Important architectural note

Current ma-agent flow still assumes one IPNS key per agent onboarding flow.

Target architecture (requested):

- One Kubo key for the world/root identity
- Agents represented as fragments under that root
- Agent daemon owns identity lifecycle and can publish DID updates at startup

This note is here so test expectations are explicit while transition work is in progress.

## Preconditions

1. Kubo API reachable on http://127.0.0.1:5001
2. World running and status endpoint reachable (default http://127.0.0.1:5002/status.json)
3. Rust workspace builds

## Quick compile checks

From ma-realms root:

- cargo check -p ma-core -p ma-world
- cargo check -p ma-agent
- make -C actor build

Expected: all commands succeed.

## Current runtime smoke tests (ma-agent)

### Test A: onboarding happy path

Command:

- cargo run -p ma-agent -- --status-url http://127.0.0.1:5002 --name npc-a --description "npc a"

Expected:

1. Closet session started
2. DID assigned
3. Enter result ok=true
4. Metadata file written under ~/.config/ma/<fragment>.json

### Test B: stateless call loop output

Command:

- cargo run -p ma-agent -- --status-url http://127.0.0.1:5002 --name npc-loop --agent-loop --poll-ms 1000

Expected:

1. Process stays alive
2. hear polling runs repeatedly
3. JSON line output appears when events trigger simple call rules

### Test C: restart behavior

1. Stop process (ctrl-c)
2. Start same command again

Expected:

- Startup succeeds without source edits
- New closet session is created

## Functional checks for room transition reliability

These checks verify the recent closet routing fix in actor flow.

1. Enter closet
2. Complete apply/publish
3. Run go out
4. Immediately send a normal command

Expected:

- No "Send failed: Room <world-name> not found"

## Multi-agent checks (current behavior)

Run two ma-agent processes with distinct names:

- cargo run -p ma-agent -- --status-url http://127.0.0.1:5002 --name npc-1 --agent-loop
- cargo run -p ma-agent -- --status-url http://127.0.0.1:5002 --name npc-2 --agent-loop

Expected:

- Both sessions onboard
- Both loops continue polling

Note: with current onboarding path this still creates/imports per-agent key material.

## Target-state test matrix (one world key + fragment agents)

Inbox-first rule:

- Agents consume events from a single inbox stream (`ma/inbox/1`), then decide routing/action locally.

These tests become mandatory when agentd mode is implemented.

### Target Test 1: create fragment agent without creating new Kubo key

Inputs:

- world slug
- fragment name

Expected:

1. Agent created as fragment identity
2. No new key appears in Kubo key list
3. DID publish succeeds using world/root key

### Target Test 2: per-agent persistent queue and logs

Expected:

1. FIFO queue persists across restart
2. Log file persists at XDG_DATA_HOME/ma/<slug>/<agent_id>.log (or logs/<agent_id>.log)
3. Search by string returns deterministic results

### Target Test 3: token/session isolation

Expected:

1. Poll/send with wrong token rejected
2. Correct token can only access its agent session

### Target Test 4: reply_to chain integrity

Expected:

1. Incoming event has reply_to metadata when present
2. Outgoing reply references source event/message id

## Suggested operator runbook for first integrated test day

1. Start world and verify status endpoint.
2. Run compile checks.
3. Run Test A and Test B.
4. Trigger manual room/chat events from actor UI.
5. Confirm call-loop JSON lines.
6. Run restart test.
7. Run multi-agent test.
8. Record any mismatch between current and target architecture.

## Gaps to close for target architecture

1. Replace per-agent key import path with single world-key fragment publishing path.
2. Introduce local agent daemon API on 127.0.0.1:5003/api/v0.
3. Add persistent event store (SQLite recommended) and per-agent log search.
4. Add explicit admin operations: create/list/delete/inspect agents.

## Exit criteria for renaming ma-agent to stable agent runtime

All should be true:

1. One-node, multi-agent runtime implemented
2. Persistent per-agent state and logs implemented
3. Token/session isolation implemented
4. reply_to aware send/poll flow implemented
5. Test matrix above passes in CI/dev
