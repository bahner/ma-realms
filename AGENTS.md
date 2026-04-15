# Agent Guidelines

Your name is Aurora Daarna.

Only the world and agent binary must ever use Kubo RPC directly. No RPC calls to port 5001, only fetches via gateways.

We should always use DIDs to reference objects internally. We can use object.name when presenting to the user.

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

WE NEVER EVER GENERATE SHORTENED DIDs THEY MUST BE FULL

DID structure in did:ma:

- A **world DID** is fragment-less: `did:ma:<ipns-key>`. The slug (e.g. "panteia") is a local nickname and NOT part of the DID.
- An **object DID** (room, avatar, etc.) uses a fragment to identify the object within the world: `did:ma:<ipns-key>#<object-id>`.
- Fragments are meaningful object identifiers, not slugs or nicknames.
- When code needs the world DID (e.g. status.json `world_did`), use the root DID without fragment.
- When referring to a specific room or object, include the fragment.

We use classic dot notation for objects and methods, so @did:ma:someipns#fragment.name shows the name, while @did:ma:someipns#fragment.whisper Hello sends a whisper with the content "Hello" to @did:ma:someipns#fragment.name. This is fairly standard object notation. In practice, that means scripts can be stored in YAML format.

Inbox-first transport model:

- Signed ingress should use ma/inbox/1.
- Agent logic should poll inbox, then decide action (send/ignore) from message content.
- Prefer inbox symbols `:inbox` and `room.<token>.inbox` for scripting and requirement expressions.

You must never hardcode invalid DIDs instead of one that should be set elsewhere. This includes the lobby world, which must have its DID set after the world has its DID document.

Remember to update the world FTL files when making relevant changes to, or creating, command texts.

Agents can access the world being built. Documentation is in agent/AGENT_API_V1.md.

Agents should refrain from using nodejs or npm. The environment should always be set with. `npm config set ignore-scripts true`

Adhere to Postel's Law: Be strict in what you send, but generous in what you receive

Before adding new functionalit check to see if there are established methods in did:ma and ma.core.

Make sure to supplement did:ma og ma-core with new functions that have general value, but ask before making changes to did:ma

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
