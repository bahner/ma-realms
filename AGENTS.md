# Agent Guidelines

Your name is Aurora Daarna.

Dont use /tmp use a .gitignored tmp folder in the current repo. Youre allowed to create it if missing

Only the world and agent binary must ever use Kubo RPC directly. No RPC calls to port 5001, only fetches via gateways.

All objects have an id. A nanoid. This is how we think:
id: nanoid
did: did:ma:51kipns
identity: the identity verified by a did document
url: did:ma:516ipns#id


KISS.

We avoid long functions and hardcoded command parsing, and we try to move this into suitable parsing modules.

We avoid duplicate code across actor, world, and agent, and try to move shared logic into core.

We never care about backward compatibility, since I am the only user and I can and often do reset everything instantly.

Terminology for command scripting:

- method = action written in dot notation on scope/target (for example @avatar, avatar.apply, actor.apply)
- attribute = named field on target (for example avatar.name, avatar.description)
- path = composed dotted selector for nested fields (for example actor.ma.transports, actor.apply)
- scoped commands must be expressed as dot-notated methods, not space form (for example avatar.apply, not "avatar apply")
- verb is only used for natural-language interpretation, not as the primary term in scripted paths


Adhere to Postel's Law: Be strict in what you send, but generous in what you receive

Don't mutate data that's is badly formed. Raise an error instead.

When validating input always check if input is valid and else fail. You should mot explicitly search for previously mention errors and just raise an error for arbitrary values, unless this is required. Don't create a set of error values, but of valid values and check for membership in that else fail.

## CRITICAL: Aliases are ALWAYS local to the actor. All messages to world use full DID urls

This is an absolute, non-negotiable rule:

- **Aliases (`@world`, `@here`, `@me`, `@avatar`, user-defined aliases) exist ONLY in the actor.**
- **The world NEVER receives, parses, dispatches, or interprets alias names.**
- **The actor MUST resolve every alias to a full `did:ma:...` DID before sending ANY message to the world.**
- **No alias string (e.g. "here", "avatar", "world", "me") may ever appear as a target in a MessageEnvelope sent over the wire.**
- **The world receives only full DIDs as targets. Period.**

If you find yourself adding alias-name matching (`"here"`, `"avatar"`, `"room"`, etc.) to world code, you are doing it wrong. Stop and fix the actor instead.

Actor runtime invariants (chat/input routing):

- `@` in user input means target routing. Resolve aliases behind `@...` to DID before evaluation.
- Explicit DID targets (`@did:ma:...`) must be preserved as typed in send paths and must not be rewritten to alias labels.
- For DID path routing, `@did:ma:<world>.<method>` routes as world method.
- For DID path routing, `@did:ma:<world>.<object>.<method>` routes as object method (`did root + #object` target).
- DID/object commands should work statelessly (without requiring active room presence state). Avatar/room presence commands may still require presence.
- Dynamic special aliases are actor-local conveniences only: `@world`, `@here`, `@me`, `@avatar`.
- Dynamic aliases are system-managed and may be auto-refreshed in actor `.aliases`.
- World runtime must not parse, dispatch, or depend on dynamic alias names.
- Actor must resolve dynamic aliases to DID targets before send.
- Actor must never send `@world/@here/@me/@avatar` (or any other alias label) over the wire; outbound targets must always be full `did:ma:...` values.
- Error text for target resolution should prefer raw DID visibility over alias-humanized display when debugging unknown-target failures.

## DID Identity and DID URL Fragments

- DIDs are always fully qualified and must be treated as real identities in the system.
- The world DID is fragment-less: `did:ma:<ipns-key>`.
- Object identity inside a world is expressed with DID URL fragments: `did:ma:<ipns-key>#<object-id>`.
- Fragments are stable object identifiers, not aliases, slugs, labels, or nicknames.
- The world slug is local convenience only and is never part of a DID.
- `root_cid` and `base_id` are not identity fields in ma-realms and must not be used as substitutes for DIDs.
- Shared IPNS key material is an implementation detail; identity handling still uses full DIDs and DID URLs.

## Security: Secret File Storage

Sensitive files must only be written under XDG home roots (for example
`XDG_CONFIG_HOME/ma` and `XDG_DATA_HOME/ma`, with HOME-based fallbacks).

- Never write sensitive runtime files to `/tmp`.
- Always use shared secure file helpers from core for sensitive writes.
- Treat runtime config, unlock bundles, and iroh secret files as sensitive.

Cross-platform hardening policy:

- Unix/macOS directories: `0700`
- Unix/macOS runtime/sensitive files: `0600`
- Unix/macOS iroh secret files: `0400`
- Windows files and directories: current-user ACL only

Fail hard if secure permission/ACL hardening fails.
