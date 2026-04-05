# ma-realms

Build playful, shared worlds on top of `did:ma`.

`ma-realms` is a practical, developer-friendly implementation layer for actor-based worlds. It uses the `did:ma` messaging protocol for secure and fast message delivery between actors, then turns those messages into rooms, movement, presence, and social world mechanics.

## Why "ma"

The name comes from the Japanese concept **間** (*ma*): the meaningful space between things.

In this project, that idea maps naturally to actor communication:
- messages create the "between"
- worlds emerge from those relationships
- simple primitives produce rich interactions

## Protocol Positioning

`did:ma` is intentionally minimalist:
- secure signed messages
- fast transport-friendly framing
- actor-first communication model

The syntax and document style are inspired by W3C DID conventions, while remaining an independent, evolving protocol (not formally registered yet).

## What You Get Here

- [core](core): shared realm protocol/domain code used by actor and world
- [actor](actor): WASM browser client for humans and bots
- [world](world): world runtime/server

## Command Terminology

To keep scripting predictable across actor/world/core layers, this repo uses:

- `method`: command/action token (for example `show`, `apply`, `invite`)
- `attribute`: dotted member on a target (for example `avatar.name`)
- `path`: dotted nested selector (for example `document.ma.transports`)

`verb` is reserved for natural-language interpretation layers and should not be used as the primary name for scripted dotted command segments.

## Quick Start

Prerequisites:
- Rust toolchain
- wasm-pack
- Python 3
- Kubo/IPFS running locally at `http://127.0.0.1:5001`

From this directory (`ma-realms`):

```bash
make check
make dev
```

`make dev` will:
- build core, actor, and world
- generate the current actor web CID
- start world in dev mode with that CID wired in

From the parent repo root, use:

```bash
make -C ma-realms dev
```

## Common Commands

```bash
make help
make core-build
make actor-build
make world-build
make actor-cid
make clean
make distclean
```

## Release Automation

Cross-platform release builds for `ma-world` live in:
- [.github/workflows/release-ma-world.yaml](.github/workflows/release-ma-world.yaml)

Targets include Linux, macOS, and Windows artifacts.

## Join In

If you enjoy actor systems, decentralized identity, and world-building, this is made for tinkering.

Clone it, run it, fork it, build a strange world with it.
