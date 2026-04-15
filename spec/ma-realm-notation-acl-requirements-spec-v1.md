# MA Realm Notation, ACL, and Requirements Profile v1

Status: Draft

Audience: Runtime developers, actor-client developers, policy authors

This document is a realm profile and intentionally avoids restating base DID/ma specifications.

## 1. Normative References

Authoritative base specifications live in:

- https://github.com/bahner/ma-spec

This profile MUST be interpreted as an overlay on the base specs above. If this document conflicts with base DID/ma semantics, the base specs win.

## 2. Profile Scope

This profile only defines:

- realm-level dot notation mapping
- alias symlink behavior in actor clients
- capability key derivation from canonical paths
- ACL subject/pattern policy profile
- requirements syntax profile and evaluation order
- method resolution pipeline used by realm runtimes

Out of scope (defined by base specs or other docs):

- DID method semantics
- ma envelope structure/signature/timestamps
- transport internals
- generic content-type registry

## 3. Realm Canonical Tree

All realm address resolution MUST normalize into this conceptual tree:

- world
- world.rooms.<room_id>
- world.avatars.<avatar_id>
- world.objects.<object_id>

This tree is the source for:

- notation mapping
- capability key generation
- requirement context generation
- policy checks

## 4. External Notation Profile

### 4.1 Target Form

Realm target form (profile constraint):

  @did:ma:<world>[#<fragment>].<method_or_attribute> [args]

Profile rules:

- Fragment namespace is flat for realm object ids.
- Fragment MUST NOT be interpreted as path/index/selector.

### 4.2 Required Alias Equivalence

Actor-local aliases are convenience only and MUST resolve before send.

Required equivalence:

- @avatar.name
- @did:ma:<world>#<avatar_id>.name

Both MUST map to canonical:

  world.avatars.<avatar_id>.name

### 4.3 Mapping Rules

Minimum mapping set:

- @world.<x> => world.<x>
- @here.<x> => world.rooms.<room_id>.<x>
- @did:ma:<world>#<avatar_id>.<x> => world.avatars.<avatar_id>.<x>
- @did:ma:<world>#<object_id>.<x> => world.objects.<object_id>.<x>

## 5. Alias Symlink Profile

Alias binding is a local symlink in actor command context.

Example:

  use mailbox as @dings

Rules:

- Alias rewrite MUST be deterministic.
- Alias rewrite SHOULD be idempotent.
- Alias rewrite MUST NOT change envelope from.
- Alias rewrite MUST only change target expression.
- Wire MUST NOT contain actor-local labels such as @world/@here/@me/@avatar.

## 6. Avatar Execution Context Profile

Realm execution context is avatar-based.

Rules:

- Realm command content that executes in world context MUST carry content.avatar.
- Runtime MUST validate avatar.owner == message.from before executing methods.
- Policy/ownership checks for in-world actions MUST execute as avatar principal.

## 7. Capability ACL Profile

### 7.1 ACL Shapes

Accepted ACL shapes:

- YAML/JSON map subject => [patterns]
- YAML/JSON object with acl => map subject => [patterns]

Subject profile:

- explicit DID
- *
- owner

Pattern profile:

- wildcard * is allowed
- empty pattern is invalid

### 7.2 Capability Derivation

Given canonical node <domain>.<id>:

- read capability: <domain>.<id>.read
- method capability: <domain>.<id>.method.<method>.invoke

Domain normalization profile:

- world => world
- rooms => room
- avatars => avatar
- objects => object

### 7.3 Capability Reference Table

| Canonical Path | Operation | Capability Key |
|---|---|---|
| world | method ping | world.method.ping.invoke |
| world.avatars.<avatar_id> | read attribute | avatar.<avatar_id>.read |
| world.avatars.<avatar_id> | method describe | avatar.<avatar_id>.method.describe.invoke |
| world.rooms.<room_id> | read metadata | room.<room_id>.read |
| world.rooms.<room_id> | method content-b64 | room.<room_id>.method.content-b64.invoke |
| world.objects.<object_id> | read | object.<object_id>.read |
| world.objects.<object_id> | method peek | object.<object_id>.method.peek.invoke |
| world.objects.<object_id> | method apply | object.<object_id>.method.apply.invoke |

### 7.4 Evaluation Order

Object capability evaluation MUST apply in order:

1. global ACL (if configured)
2. local ACL by ACL CID (if configured)
3. local inline ACL (if configured)
4. owner subject semantics

If any required layer denies, access MUST be denied.

## 8. Requirements Syntax Profile

Requirements are per-verb preconditions.

### 8.1 Legacy Requirement Signatures

Supported signatures:

- object.exists
- object.held
- object.not_held
- object.held_by_self
- object.held_by_other
- object.open
- object.closed
- object.opened_by_self
- object.opened_by_other
- world.owned
- room.in(<optional_arg>)

### 8.2 Expression Syntax

Operators:

- ==
- !=
- &&
- ||
- !
- ( )

Literals:

- string
- true
- false
- null

Allowed symbols in current profile:

- user
- owner
- location
- opened_by
- world.owner
- world.slug
- inbox
- room.<token>.inbox
- state.<field>

Validation rules:

- Unknown symbol MUST fail validation.
- Invalid argument arity MUST fail validation.
- Contradictory legacy requirements MUST fail validation.
- Duplicate requirements SHOULD be rejected.

## 9. Method Resolution Pipeline

Runtime MUST execute in this order:

1. Resolve canonical target node from DID target.
2. Resolve method/attribute under node.
3. Validate avatar-owner binding.
4. Evaluate capability ACL for operation.
5. Validate and evaluate requirements.
6. Execute node handler/parser.

Routing profile:

- World parser is router and policy gate.
- Object parser/handler SHOULD remain authoritative for object-specific semantics.

## 10. Ownership Profile

Conventions:

- avatar.owner = actor DID
- room.owner = avatar DID
- object.owner = avatar DID

Constraints:

- Avatars MUST NOT be owned by world objects.
- World objects MAY be owned by avatars.

## 11. Worked Mapping Examples

Example A:

Input:

  @avatar.name

Resolution:

- @avatar => did:ma:<world>#<avatar_id>
- canonical => world.avatars.<avatar_id>.name

Example B:

Input:

  @did:ma:worldipns#mailbox.peek

Resolution:

- canonical => world.objects.mailbox.peek
- capability => object.mailbox.method.peek.invoke

Example C:

Input:

  use mailbox as @dings
  @dings.peek

Resolution:

- @dings => did:ma:worldipns#mailbox
- canonical => world.objects.mailbox.peek

## 12. Authoring Checklist

- Define canonical node and operation first.
- Derive capability keys from canonical path mapping.
- Keep alias behavior local to actor clients.
- Validate requirements grammar and symbol usage.
- Enforce avatar-owner binding before operation execution.

## 13. Migration Note

No backward compatibility is required by project policy.

Migration guidance:

- Replace actor-root in-world execution checks with avatar-principal checks.
- Keep capability naming pattern stable where possible.
