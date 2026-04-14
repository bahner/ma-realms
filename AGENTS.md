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

WE NEVER EVER GENERATE SHORTENED DIDs THEY MUST BE FULL INCLUDING FRAGMENTS

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

## CRITICAL: Aliases are ALWAYS local to the actor. All messages to world use full DIDs.

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
- For DID path routing:
	- `@did:ma:<world>.<method>` routes as world method.
	- `@did:ma:<world>.<object>.<method>` routes as object method (`did root + #object` target).
- DID/object commands should work statelessly (without requiring active room presence state). Avatar/room presence commands may still require presence.
- Dynamic special aliases are actor-local conveniences only: `@world`, `@here`, `@me`, `@avatar`.
	- They are system-managed and may be auto-refreshed in actor `.aliases`.
	- World runtime must not parse, dispatch, or depend on these alias names.
	- Actor must resolve them to DID targets before send.
	- Actor must never send `@world/@here/@me/@avatar` (or any other alias label) over the wire; outbound targets must always be full `did:ma:...` values.
- Error text for target resolution should prefer raw DID visibility over alias-humanized display when debugging unknown-target failures.

DIDs are always fully qualified. You might need to get the .ipns for ipfs feature or to veridy the ipns key or things like that, but that is backoffice stuff. Plumbing. The ma-realms never uses root_cid ir base_id's. They are not a thing. There is no real "identity" behind anything. Each did is discreet, in the greater scheme of things. The fact that we share ipns is incidental and just an implementation detail for world, where we don't want so many keys. DIDs should still be treated as full identities nontheless.
