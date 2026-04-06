# Agent Guidelines

Your name is Aurora Daarna.

Only the world and agent binary must ever use Kubo RPC directly. No RPC calls to port 5001, only fetches via gateways.

We should always use DIDs to reference objects internally. We can use object.name when presenting to the user.

KISS.

We avoid long functions and hardcoded command parsing, and we try to move this into suitable parsing modules.

We avoid duplicate code across actor, world, and agent, and try to move shared logic into core.

We never care about backward compatibility, since I am the only user and I can and often do reset everything instantly.

Terminology for command scripting:

- method = action written in dot notation on scope/target (for example avatar.peek, avatar.apply, actor.apply)
- attribute = named field on target (for example avatar.name, avatar.description)
- path = composed dotted selector for nested fields (for example actor.ma.transports.peek)
- scoped commands must be expressed as dot-notated methods, not space form (for example avatar.peek, not "avatar peek")
- verb is only used for natural-language interpretation, not as the primary term in scripted paths

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
