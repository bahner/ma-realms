# ma-realms Specifications

Realm-specific specifications for the ma-realms world runtime. These documents
build on the foundational [did:ma spec](https://github.com/bahner/ma-spec) and
define realm runtime behavior.

## Documents

- [did:ma Extension Fields — Realm Profile](did-ma-fields-format.md) — Fragment
  requirement (DID vs DID URL), type profiles
  (world/agent/avatar/room/object), transport ALPN registry, and validation
  rules. Builds on the foundational
  [did-ma-fields-format.md](https://github.com/bahner/ma-spec/blob/main/did-ma-fields-format.md).

- [Access Control v1](access-control-v1.md) — Capability-based ACL model with
  wildcard patterns, avatar binding gate, owner semantics, and evaluation
  algorithm for realm objects.

- [Realm Notation, ACL & Requirements](ma-realm-notation-acl-requirements-spec-v1.md) —
  Dot-notation mapping, alias symlink behavior, capability derivation, ACL
  subject patterns, requirements syntax, and method resolution pipeline.

- [Avatar Presence Protocol v1](avatar-presence-v1.yaml) — Heartbeat-based
  avatar presence with TTL, ping/prune lifecycle.

- [Realm Capabilities Tree v1](ma-realm-capabilities-tree-v1.yaml) — Capability
  model, dot-tree method listing, ACL resolution order, routing rules, and
  state/restore overview.

- [Schema Examples](schema-examples/) — Example YAML schemas for world config,
  world root, actor registry, and world manifest.
