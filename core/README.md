# ma-core

Shared actor library for the ma stack — message parsing, protocol types, room command engine, domain model, addressing, and trait interfaces.

## What It Provides

### Message Parsing (`parser`)

- `MessageEnvelope`: normalized representation of user input (method token, target, payload, raw text)
- `parse_message()`: top-level parser that classifies input as room commands, actor messages, or spoken text
- Canonical command parsing without hardcoded per-language alias tables

Terminology used in parser/docs:

- `method` = command token/action name after routing (for example `show`, `apply`, `invite`)
- `attribute` = dotted field/member on a target actor/object (for example `avatar.name`)
- `path` = multi-segment dotted selector for nested data access (for example `document.ma.transports`)
- `verb` = reserved for natural-language interpretation layers; avoid using it for scripted command paths

### Protocol Types (`protocol`)

Single source of truth for shared wire types and ALPN constants used by both client (`ma-actor`) and server (`ma-world`):

- **ALPN identifiers:** `INBOX_ALPN` (signed ingress, actor→world), `AVATAR_ALPN` (avatar commands, actor→world), `PRESENCE_ALPN` (presence push, world→actor), `IPFS_ALPN`
- **Relay defaults:** `DEFAULT_WORLD_RELAY_URL`
- **Lane model:** `WorldLane` + `LaneCapability` for transport-capability negotiation
- **Transport ack:** `TransportAck` + `TransportAckCode` for standardized lane-level accept/reject feedback
- **Wire types:** `WorldResponse`, `WorldCommand`, `WorldRequest`, `RoomEvent`, `PresenceAvatar`

### Room Command Engine (`room_actor`)

Built-in room commands with ACL-based authorization:

- **Read-only:** `/help`, `/who`, `/l` (list exits), `/acl`, `/describe`, `/show`
- **Owner-only:** `/invite <did>`, `/deny <did>`, `/kick <handle>`, `/dig <direction> [to <dest>]`, `/set <owner|title|description|cid> <value>`
- Returns `RoomActorResult` with response text and optional `RoomActorAction` (Invite, Deny, Kick, Dig, SetAttribute)
- Includes a TODO hook point for evaluator-registered commands (Lua/Guile)

### Domain Model (`domain`)

- `ActorType` enum: World, Room, Avatar, Exit, Object
- `ExitData`: room exits with direction names, aliases, and visibility/lock flags
- `WorldActor`, `RoomActor`, `AvatarActor`: actor types with display names and command lists

### Addressing (`addressing`)

- DID root extraction, `/iroh/` address normalization, endpoint validation
- Relay URL normalization helper (`normalize_relay_url`)
- Endpoint extraction helpers for transport metadata (`endpoint_id_from_address`, `endpoint_id_from_transport_value`, `resolve_inbox_endpoint_id`)
- Alias resolution and reverse lookup
- Text humanization (replace DIDs/endpoints with aliases in prose)

### Interfaces (`interfaces`)

Abstract traits for pluggable backends:

- `DidPublisher`: publish DID documents
- `IpfsPublisher`: put JSON to IPFS and publish IPNS names
- `AclRuntime`: room ACL queries (can_enter, summary)

## Project Layout

- `src/addressing.rs`: address normalization and alias resolution
- `src/domain.rs`: core domain model types (ActorType, ExitData, actors)
- `src/interfaces.rs`: abstract trait interfaces
- `src/lib.rs`: module declarations, public re-exports, unit tests
- `src/parser.rs`: message parsing and envelope construction
- `src/protocol.rs`: shared protocol types and ALPN constants
- `src/room_actor.rs`: built-in room command engine

## Build and Cleanup

Use the Makefile:

```bash
make build
make test
make clean
make distclean
```

Equivalent cargo commands:

```bash
cargo build
cargo test
```

## Notes

- This is a library crate shared by `ma-actor` (WASM client) and `ma-world` (server).
- Protocol types in `protocol.rs` are the single source of truth — do not duplicate in consumer crates.
- The crate uses `serde` for serialization.
