# ma-world: A Private World Server

A Rust binary that hosts rooms (spaces) over iroh P2P,
manages access control, and persists world state to IPFS via Kubo.

## Overview

`ma-world` provides:

- **Rooms** — named spaces (`lobby`, `study`, …) where avatars interact
- **Access Control** — per-room ACLs (owner / allow / deny lists with `*` wildcard)
- **Avatar Routing** — signed CBOR messages dispatched to targets via `@target` syntax
- **Room Persistence** — room IDs are nanoid-compatible; each room is serialized as YAML and pinned to IPFS; the world root index stores only `id -> CID` links
- **Exit Persistence** — each exit is serialized as an individual YAML object and pinned to its own CID
- **Avatar Definition Persistence** — each avatar definition is serialized as an individual YAML object and pinned to its own CID
- **Status Page** — axum HTTP server exposing `/` (HTML) and `/status.json` (JSON) with live room, avatar, and network info
- **Kubo Integration** — DID document fetching, DAG put/get, IPNS publish, actor secret bundle management
- **Presence Broadcasts** — periodic snapshots of room occupants pushed to all connected clients
- **Structured Logging** — tracing with optional log file output

## Architecture

```txt
Browser (ma-actor WASM)
    |
    | iroh (inbox-first ingress)
    v
ma-world
    |
    +-- Room "lobby"  (ACL: *)
    |       +-- Avatar: alice (did:ma:…)
    |       +-- Avatar: bob   (did:ma:…)
    |
    +-- Room "study"  (ACL: owner + allow-list)
            +-- Avatar: carol (did:ma:…)
```

ALPN lanes imported from `ma-core`:

| Lane | ALPN identifier |
| ------ | ----------------- |
| Inbox | `ma/inbox/1` |
| Broadcast | `ma/broadcast/1` |
| Presence | `ma/presence/1` |

## Runtime Configuration

Server mode requires `--slug <slug>`.

Supported run flags:

| Flag | Default | Purpose |
| ------ | --------- | --------- |
| `--slug` | (required) | slug used for runtime naming/path resolution |
| `--listen` | `127.0.0.1:5002` | Status HTTP bind address |
| `--kubo-url` | `http://127.0.0.1:5001` | Kubo HTTP API base URL |
| `--owner` | (none) | Set world owner DID at startup |
| `--log-level` | `info` | tracing level |
| `--log-file` | `$XDG_DATA_HOME/ma/worlds/<slug>/ma-world.log` | optional file sink for logs |

Runtime file config (optional):

- `XDG_CONFIG_HOME/ma/<slug>.yaml` (or `~/.config/ma/<slug>.yaml`)
- Keys:

```yaml
kubo_api_url: http://127.0.0.1:5001
listen: 127.0.0.1:5002
owner: did:ma:...
iroh_secret: /home/user/.config/ma/panteia_iroh.bin
log_level: info
log_file: /tmp/ma-world.log
actor_web_version: 0.1.0
actor_web_cid: bafy...
actor_web_dir: /home/user/src/ma/rust/ma-realms/actor/www
actor_web_listen: 127.0.0.1:8081
actor_web_cache_dir: /home/user/.config/ma/actor-web
actor_web_ipns_key: ma-actor
actor_web_enabled: true
actor_web_auto_build: true
actor_web_auto_publish_ipns: true
```

Set `actor_web_enabled: false` to run world without starting actor web runtime.

Optional environment fallback for owner in run mode:

- `MA_WORLD_OWNER=did:ma:...`

Remote/CLI unlock bundle generation (no status page needed):

```bash
cargo run --manifest-path world/Cargo.toml -- create-unlock-bundle --slug <slug> --passphrase '<secret>'
```

Default output path is `~/.config/ma/<slug>_bundle.json`.
Then set in runtime config:

```yaml
status_api_enabled: true
admin_api_enabled: false
unlock_passphrase: <secret>
unlock_bundle_file: /home/user/.config/ma/<slug>_bundle.json
```

Status API runtime options in `<slug>.yaml`:

- `status_api_enabled: true` => start status HTTP socket (`/`, `/status.json`, `/openapi.json`)
- `status_api_enabled: false` => disable status HTTP socket entirely
- `admin_api_enabled: true` => enable mutating POST endpoints under status API
- `admin_api_enabled: false` => disable mutating POST endpoints (status/read-only mode)
- `admin_api_password: <secret>` => required only when `admin_api_enabled: true`

`MA_WORLD_ADMIN_API_PASSWORD` can override `admin_api_password` at runtime.

When `admin_api_enabled: true`, status API access uses HTTP Basic auth:

- username = world slug
- password = `admin_api_password`

Sensitive runtime files are hardened after write:

- On Unix/macOS: `<slug>.yaml` uses mode `0600`, iroh secret uses `0400`
- On Windows: current-user ACL is applied via `windows-acl`
- Hardening failures are fatal (startup/config generation fails)

One-shot remote/headless setup (writes config + iroh key + unlock bundle):

```bash
cargo run --manifest-path world/Cargo.toml -- --gen-headless-config --slug <slug>
```

This command writes:

- `~/.config/ma/<slug>_iroh.bin`
- `~/.config/ma/<slug>_bundle.json`
- `~/.config/ma/<slug>.yaml`

Quick one-off override at startup:

```bash
cargo run --manifest-path ma-world/Cargo.toml -- run --slug <slug> --cid <bafy...>
```

Equivalent shorthand (top-level flags):

```bash
cargo run --manifest-path ma-world/Cargo.toml -- --slug <slug> --cid <bafy...>
```

Actor web metadata/CID resolution priority:

- explicit runtime override `actor_web_cid`
- automatic local build from `actor_web_dir` (or sibling `../actor/www`) when `actor_web_auto_build: true`
- authored `world_manifest.yaml` active actor web CID
- fallback resolve from Kubo IPNS key `actor_web_ipns_key`

If `actor_web_auto_build: true`, `ma-world` packages actor web files as a tar archive,
adds it to IPFS, and uses that CID for runtime serving. This means ma-world and ma-actor
are wired together automatically at startup without manual CID copying.

If `actor_web_auto_publish_ipns: true`, the new auto-built CID is also published to
`actor_web_ipns_key` (default `ma-actor`).

`actor_web_dir` enables static serving on `actor_web_listen` (default `127.0.0.1:8081`).

If an actor web CID is available, `ma-world` downloads and unpacks that CID at startup into
`actor_web_cache_dir` (default: `$XDG_CONFIG_HOME/ma/actor-web`) and serves from the cached CID directory.
On next startup (or CID change), the cached directory for that CID is refreshed.

If no actor web source dir is available and no CID is configured, `ma-world` attempts to
resolve actor web CID from local Kubo IPNS key `actor_web_ipns_key` (default: `ma-actor`).

Default iroh secret path when `iroh_secret` is not set:

- `XDG_CONFIG_HOME/ma/<slug>_iroh.bin` (or `~/.config/ma/<slug>_iroh.bin`)

Generate iroh secret explicitly (required before server startup):

```bash
ma-world --gen-iroh-secret ~/.config/ma/panteia_iroh.bin
ma-world --gen-iroh-secret --slug panteia
```

When path is omitted, `--gen-iroh-secret` resolves target path as:

1. `iroh_secret` from `XDG_CONFIG_HOME/ma/<slug>.yaml` (or `~/.config/ma/<slug>.yaml`)
2. fallback `XDG_CONFIG_HOME/ma/<slug>_iroh.bin` (or `~/.config/ma/<slug>_iroh.bin`)

`ma-world` does not auto-create the iroh secret at startup.

World master key for unlock bundle/save/load is derived in-memory from:

- machine-local iroh secret key
- world slug

No separate world master key file is written by default.

Environment fallbacks (used if not set by CLI/config file):

- `MA_LISTEN`
- `MA_KUBO_API_URL`
- `MA_LOG_LEVEL`
- `MA_LOG_FILE`

## Runtime Persistence Boundary

Runtime startup is intentionally minimal:

- iroh identity key (machine-local)
- derived in-memory world master key (from iroh identity + world slug)
- optional small runtime config YAML (`<slug>.yaml`)

Server-side filesystem writes are intentionally minimal:

- no automatic writes except optional log file output

Unlock bundle can be created via status API/UI or by CLI command (`create-unlock-bundle` / `--gen-headless-config`) which writes bundle files directly.

`ma-world` does not auto-bootstrap a world directory during server startup.
Moving world data between machines is supported at world-data level (CIDs/state),
while iroh/Kubo node identities remain manual and machine-local.

## Room ACL Model

```rust
pub struct RoomAcl {
    pub owner: Option<String>,       // always granted
    pub allow_all: bool,             // wildcard
    pub allow: HashSet<String>,      // explicit allow
    pub deny: HashSet<String>,       // priority deny
}
```

Evaluation order: **deny → allow (with `*` wildcard)**. The room owner is
always granted access regardless of the deny list.

`MA_WORLD_ENTRY_ACL` environment variable sets the default entry ACL
(default `*` = open to everyone).

## Building & Running

```bash
make build          # debug
make release        # optimised
make run            # debug + RUST_LOG

# Or directly:
cargo run --bin ma-world -- run --listen 127.0.0.1:5002 --kubo-url http://127.0.0.1:5001
cargo run --bin ma-world -- run --slug panteia
RUST_LOG=ma_world=debug cargo run --bin ma-world -- run --slug panteia
```

## Language Pack Defaults

The repository now includes starter language files for `lang_cid` workflows:

- `world/lang/manifest.yaml`
- `world/lang/en_UK.ftl`
- `world/lang/nb_NO.ftl`

`manifest.yaml` is a template. Replace `TODO_CID_*` values with published IPFS
CIDs for each language file when preparing a world language pack.

Publishing helper boundary:

- Runtime code paths use shared Kubo/publisher logic from ma-core.
- [scripts/publish-lang-map.sh](scripts/publish-lang-map.sh) is an ops-only helper and intentionally uses direct Kubo HTTP API calls via curl.

Language packs change world response text, not world command grammar.
Command tokens stay standard/invariant (`help`, `show`, `describe`, `lang`,
`private`, `knock`, `invite`, `room`, `dig`). If clients want localized
command words, they must translate input to standard tokens before sending to
world.

## Release Builds (GitHub Actions)

`ma-world` now includes a release workflow at:

- `.github/workflows/release-ma-world.yaml`

The workflow builds release binaries for:

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

Triggers:

- Push tag matching `v*` (for example `v0.1.0`)
- Manual `workflow_dispatch`

On tag builds it creates a GitHub Release and uploads archives plus SHA256 files:

- `ma-world-<tag>-<target>.tar.gz` (Linux/macOS)
- `ma-world-<tag>-<target>.zip` (Windows)
- corresponding `.sha256` files

The workflow also runs a binary smoke test (`ma-world --help`) on each platform before packaging.

Optional signing/notarization hooks are enabled when secrets are present:

- macOS codesign: `MA_WORLD_MACOS_CERT_P12_BASE64`, `MA_WORLD_MACOS_CERT_PASSWORD`, `MA_WORLD_MACOS_CODESIGN_IDENTITY`
- macOS notarization: `MA_WORLD_APPLE_ID`, `MA_WORLD_APPLE_TEAM_ID`, `MA_WORLD_APPLE_APP_PASSWORD`
- Windows signing: `MA_WORLD_WINDOWS_CERT_PFX_BASE64`, `MA_WORLD_WINDOWS_CERT_PASSWORD`

On startup the server:

1. Requires `--slug` and optionally reads `XDG_CONFIG_HOME/ma/<slug>.yaml`
2. Requires existing iroh secret file (create with `--gen-iroh-secret`)
3. Initialises an iroh endpoint with that machine-local secret key
4. Creates the default `lobby` room
5. Binds the status page to the configured listen address
6. Registers protocol handlers for inbox-first signed ingress (`ma/inbox/1`) and auxiliary transports
7. Prints the iroh endpoint id, status URL, and Kubo API URL

## World Admin Commands

Use `@world.<method>` commands from an owner-controlled avatar session:

- `@world.list` — lists world objects as `id => title`
- `@world.migrate-index` — re-pins all current room snapshots and republishes the world root CID index

`@world` commands are validated as world-targeted operations and must be sent to
this world's DID (root or configured world DID).

Room attributes are sourced from each room YAML CID. The world root index is
kept minimal (ID to CID link map) so updates happen by publishing updated YAML
objects and switching to their new CIDs.

Room YAML (`ma_room` v2) stores room attributes plus references (`exit_cids`,
`avatar_cids`) to exit and avatar-definition YAML documents. Loading a room CID
materializes exits from those referenced CIDs.

The runtime world DID document publishes bootstrap metadata under `ma`,
including `rootCid`, transport hints, and inbox routing data.

## Status Page

The axum status surface exposes:

- **`/`** — HTML dashboard: endpoint id, direct addresses, multiaddrs, relay URLs, rooms, avatars, recent events, owner DID, world CID, persisted room count
- **`/openapi.json`** — OpenAPI 3.1 document for status API endpoints
- **`/status.json`** — JSON `{ world, snapshot, runtime }` for programmatic access
- `world.capabilities` lists lane capabilities (`inbox`) with ALPN + supported request kinds
- Transport-level request rejections include `transport_ack` in `WorldResponse` (`lane`, `code`, `detail`)

When both `status_api_enabled: true` and `admin_api_enabled: true`, the status surface also exposes mutating POST endpoints:

- **`/world/slug`** — POST form endpoint to set world slug used as named pin alias
- **`/world/kubo`** — POST form endpoint to set runtime Kubo API URL
- **`/world/save`** — POST endpoint to save encrypted runtime state and update root CID
- **`/world/load`** — POST form endpoint to load encrypted runtime state by `state_cid`
- **`/world/load-root`** — POST form endpoint to load world rooms from root CID
- **`/bundle/create`** — POST form endpoint to create a compact encrypted unlock bundle from a passphrase
- **`/unlock`** — POST form endpoint to unlock runtime using passphrase + bundle

When `admin_api_enabled: true`, status HTTP endpoints require HTTP Basic auth with:

- username = world slug
- password = `admin_api_password`

Mutating endpoints use `application/x-www-form-urlencoded` and return JSON.

### Scripted API (curl)

Example base URL:

```bash
BASE="http://127.0.0.1:5002"
SLUG="panteia"
PASSWORD="your-admin-api-password"
```

Check status + OpenAPI:

```bash
curl -s "$BASE/status.json" | jq
curl -s "$BASE/openapi.json" | jq '.paths | keys'
```

Update world slug (pin alias):

```bash
curl -s -X POST "$BASE/world/slug" \
    -u "$SLUG:$PASSWORD" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "slug=panteia" | jq
```

Update runtime Kubo API URL:

```bash
curl -s -X POST "$BASE/world/kubo" \
    -u "$SLUG:$PASSWORD" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "kubo_url=http://127.0.0.1:5001" | jq
```

Create unlock bundle + unlock runtime:

```bash
PASS="your-passphrase-here"

BUNDLE_JSON=$(curl -s -X POST "$BASE/bundle/create" \
    -u "$SLUG:$PASSWORD" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "passphrase=$PASS" \
    | jq -r '.bundle')

curl -s -X POST "$BASE/unlock" \
    -u "$SLUG:$PASSWORD" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "passphrase=$PASS" \
    --data-urlencode "bundle=$BUNDLE_JSON" | jq
```

Save runtime state and capture CIDs:

```bash
SAVE_JSON=$(curl -s -X POST "$BASE/world/save" -u "$SLUG:$PASSWORD")
STATE_CID=$(echo "$SAVE_JSON" | jq -r '.state_cid')
ROOT_CID=$(echo "$SAVE_JSON" | jq -r '.root_cid')
echo "state_cid=$STATE_CID"
echo "root_cid=$ROOT_CID"
```

Load by encrypted state CID:

```bash
curl -s -X POST "$BASE/world/load" \
    -u "$SLUG:$PASSWORD" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "state_cid=$STATE_CID" | jq
```

Load by world root CID:

```bash
curl -s -X POST "$BASE/world/load-root" \
    -u "$SLUG:$PASSWORD" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "root_cid=$ROOT_CID" | jq
```

### Script Helper (shell)

A reusable helper script is available at [scripts/world-api.sh](scripts/world-api.sh).

Load it in your shell:

```bash
source scripts/world-api.sh
```

Then call helper functions:

```bash
mw_set_base http://127.0.0.1:5002
mw_set_basic_auth panteia "your-admin-api-password"
mw_status | jq
mw_set_slug panteia | jq
mw_set_kubo http://127.0.0.1:5001 | jq
mw_unlock_from_passphrase "your-passphrase-here" | jq

SAVE_JSON=$(mw_save)
echo "$SAVE_JSON" | jq
STATE_CID=$(echo "$SAVE_JSON" | jq -r '.state_cid')
ROOT_CID=$(echo "$SAVE_JSON" | jq -r '.root_cid')
mw_load_state "$STATE_CID" | jq
mw_load_root "$ROOT_CID" | jq
```

Requires `curl` and `jq` in your environment.

### Unlock Flow

`ma-world` starts locked and requires a passphrase plus encrypted bundle to unlock actor secrets.

1. Open `/` in browser.
2. In **Create Unlock Bundle**, enter passphrase and generate bundle JSON.
3. In **Unlock Runtime**, provide passphrase + bundle JSON.
4. World transitions to unlocked state and starts handling inbox traffic.

Bundle format uses Argon2id key derivation and XChaCha20-Poly1305 encryption.

The bundle payload includes encrypted world master key material.

Persisted runtime state envelopes are signed with world `sig` and encrypted using world `enc` key material.

### Portability Boundary

- Unlock bundle is intentionally minimal and only carries encrypted world master key material.
- iroh node identity (`iroh_secret_key.bin`) remains machine-local and is not included in bundle/state export.
- Kubo node identity/keys remain machine-local and are not included in bundle/state export.
- Moving a world between machines is supported at world-data level (CIDs/state), but iroh and Kubo migration is manual by design.

CORS is open (`*`) so `ma-actor` can fetch status from the browser.

## Project Layout

| File | Purpose |
| ------ | --------- |
| `src/main.rs` | Entry point, World struct, iroh protocol handlers, message dispatch |
| `src/room.rs` | `Room` and `RoomAcl` — room state, ACL evaluation, descriptions, exits |
| `src/actor.rs` | `Avatar` and `ActorAcl` — per-avatar state and per-actor access control |
| `src/status.rs` | axum routes (`/`, `/status.json`, world save/load/slug, unlock), HTML renderer |
| `src/kubo.rs` | ma-core Kubo shim/re-export used by world runtime |
| `src/schema.rs` | World skeleton init, authoring YAML, actor secret bundles, crypto |

## Key Dependencies

| Crate | Role |
| ------- | ------ |
| `iroh 0.97` | P2P endpoint and protocol routing |
| `ma-did` | DID/Document/Message types, signing, verification |
| `ma-actor` | Shared ALPN constants, wire types, command dispatch |
| `axum` + `tower-http` | Status page HTTP server with CORS |
| `reqwest` | Kubo HTTP API client |
| `chacha20poly1305` | Actor secret bundle encryption |
| `argon2` | Passphrase KDF for unlock bundle |
| `serde_yaml` | Config and room definition serialisation |
| `tracing` | Structured logging |

- **ma-did**: DID primitives (local path dependency)
- **tracing**: Structured logging
- **serde_json**: JSON serialization
- **reqwest**: HTTP client (for Kubo integration)

## Security Model (Current)

- Kubo is expected local/private unless intentionally exposed.
- Signed DID documents and signed messages are validated before command handling.
- `@world` admin commands are world-target DID validated.
- Runtime state save/load uses signed + encrypted envelopes.

## Cleanup

```bash
# Remove build artifacts
make clean

# Full clean + cargo clean
make distclean
```

## Future Work

- **Actor Network**: Accept connections from ma-actor browser clients
- **Message Protocol**: Define Iroh-based protocol for actor messages
- **Persistence**: Save room state between restarts (SQLite or similar)
- **ACLs**: Permission model for multi-user scenarios
- **Custom Actors**: Script-based or plugin actor definitions
