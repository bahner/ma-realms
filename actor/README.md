# ma-actor

A WebAssembly home client built on top of `ma-did`.

It creates/unlocks local encrypted identity bundles, publishes DID documents to IPNS via IPFS API, and provides a command-driven browser UI.

## Features

- WASM exports for identity lifecycle
  - create identity
  - create identity bound to existing IPNS key
  - unlock encrypted identity bundle
- Passphrase-based local encryption (`argon2id` + `XChaCha20Poly1305`)
- BIP39 recovery phrase generation/normalization
- Browser UI with slash commands
- IPFS API integration for key management and IPNS publish

## Repository Layout

- `src/lib.rs`: wasm-bindgen exports and crypto/identity logic
- `www/index.html`: UI shell
- `www/style.css`: UI styling
- `www/app.js`: app logic and slash command handling
- `www/pkg/`: generated wasm-pack output (ignored)

## Prerequisites

- Rust toolchain
- `wasm-pack`
- Python 3 (for local static server)
- IPFS API reachable at `http://127.0.0.1:5001`

## Build and Run

```bash
make build
make serve
make publish
make release
```

`make build` now also computes current IPFS CID for `www/` and writes it to:

- `.cid` (in the `ma-actor` directory)

This makes it easy to launch ma-world with the latest explicit actor-web CID:

```bash
cargo run --manifest-path ../ma-world/Cargo.toml -- run --world-slug <slug> --cid "$(cat .cid)"
```

The app is served on:

- `http://127.0.0.1:8081`

The build can be published to IPFS + IPNS with a stable URL:

- `make publish` builds and publishes without `--release` (faster for active development)
- `make release` builds with release profile before publish
- Publishes `www/` to `/ipfs/<cid>`
- Ensures key alias `ma-actor` exists (creates it if missing)
- Publishes IPNS record `/ipns/<key-id>` to point to the new CID
- Prints key alias, IPNS path, CID path, a local gateway view URL (`:8080`), and API/runtime URL (`:8081`)

Important runtime note:

- This app requires local IPFS API access at runtime (key lookup and DID publish).
- Preferred runtime: use `http://127.0.0.1:8081`.
- `http://127.0.0.1:8080/ipns/<key>/` is the published gateway URL, but IPFS API calls from that origin can be blocked by browser/API-origin policy.
- Public gateways (for example `ipfs.io`) are not suitable for this app's local API workflow.

### CID consistency notes

To maximize the chance that two developers get the same CID from the same commit:

- use the same Rust and wasm-pack versions
- run `make build` / `make release` on clean trees
- keep generated output deterministic (`SOURCE_DATE_EPOCH`, remapped paths, release build)

The Makefile already sets reproducibility-friendly build flags, but exact wasm bytes can still differ across toolchain versions.

## Cleanup

```bash
make clean
```

For a full Rust clean as well, run `cargo clean` manually.

## Command Surface

Self/config commands use the `my.*` namespace. Operational local commands still use dot prefix (`.`). Bare text is gameplay sent to the world.

### Dot Commands (local/client)

- `.help`
- `my.did`
- `my.identity`
- `my.identity.publish <did:ma:<world>>`
- `my.home <did:ma:<world>#<room>>`
- `my.aliases`
- `my.aliases add <name> <address>`
- `my.aliases del <name>`
- `my.aliases.<name>`
- `my.aliases.rewrite [on|off]`
- `my.mail [list|pick|reply|delete|clear]`
- `.inspect @here` (inspect room DID/content CID and exit CID references)
- `.inspect @exit <name|alias>` (inspect one exit document by name)
- `.edit [@here|@me|did:ma:<world>#<room>]`
- `.eval <cid|alias>`
- `.refresh` (force immediate room/object/event refresh)
- `.publish` (publishes DID document to IPNS)
- `.block <did|alias|handle>`
- `.unblock <did|alias|handle>`
- `.blocks`
- `.smoke [alias]` (diagnostic smoke test)
- `.debug [on|off]`

### Gameplay (bare, no prefix)

- `go north` — navigate via server-resolved exits
- `look` — describe current room
- `attack goblin` — gameplay command sent to world
- `'Hello world` — shorthand for @me say Hello world

### Actor Targeting

- `@target command args` — send command to actor
- `@target 'message` — whisper to actor (E2E encrypted)
- `@world.<command>` — world-admin command

## Edit Modes

- `.edit` opens local script mode
- `.edit @here` edits current room YAML
- `.edit @me` edits your avatar profile text
- `.edit did:ma:<world>#<room>` edits a specific room by DID
- Saving in local script mode publishes the script to IPFS and returns a CID
- In local script mode, `Close and Eval` saves/publishes, closes editor, and runs `.eval` on the new CID
- `.eval <cid|alias>` loads script text from IPFS and executes it line by line
- In `.eval` scripts, use explicit actor targets (`@me`, `@here`) for references
- Room editor sanitizes metadata before publish (`cid` and `did` are stripped from YAML)

Unqualified `.edit` is intentionally local-only. Network-backed editing requires an explicit target (`@here`, `@me`, or `did:...`).

Alias example:

- `my.aliases add oppsett bafyabcdefoppsettcid`
- `.eval oppsett`

Home target example:

- `my.home did:ma:<world>#lobby`
- `go home`

## World Connection Over Iroh

Navigation is gameplay — the server resolves exits and directs the client to new endpoints:

- `go north` may cross world boundaries (exits can point to any `did:ma:world#room`)
- The client auto-follows server directives to connect/reconnect Iroh endpoints
- The browser WASM client uses `iroh` directly
- Room chatter is fanned out by `ma-world` and polled by each avatar

## Identity and Publish Model

- Encrypted bundle is local/private (browser storage + export file)
- DID document is public and publishable
- `.publish` uploads DID document JSON to IPFS and updates IPNS record
- Browser storage is namespaced per alias, so one browser profile can keep multiple local homes
- The currently active alias is remembered per browser tab, which allows concurrent homes in separate tabs/windows on the same origin

## IPFS API CORS

Browser calls require IPFS API CORS headers allowing your app origin (for example `http://127.0.0.1:8081`).

## Protocol & Transport

The WASM client uses ALPN constants and content types from `ma-did` directly
(e.g. `CONTENT_TYPE_CHAT`, `CONTENT_TYPE_PRESENCE`, `CONTENT_TYPE_WHISPER`, `CONTENT_TYPE_BROADCAST`).
Connection caches are maintained inbox-first (signed world traffic over
`ma/inbox/1`) so repeated interactions with the same world reuse the iroh
connection.

An inbox listener registers protocol handlers for the `ma/inbox/1`,
`ma/whisper/1`, `ma/broadcast/1`, and `ma/presence/1` ALPN lanes so the
browser can receive inbound signed messages (presence snapshots, whispers,
broadcasts) from the world and other actors.

If `/publish` or IPFS API check fails in-browser, verify:

1. IPFS daemon is running
2. API endpoint is correct
3. CORS origins include your host/port

The setup screen also shows:

- current published IPNS path
- current published CID path
- an IPFS install hint with a link to docs when IPFS API is not reachable

Local serving on `http://127.0.0.1:8081` remains the recommended runtime origin.
Use `make publish` for fast iteration, and `make release` when you want an optimized published build.
