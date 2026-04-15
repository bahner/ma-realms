# did:ma Extension Fields (ma namespace) — Realm Profile

Version: 0.0.5
Status: Draft

## Abstract

This document defines the ma-realms implementation profile for method-specific
fields under the top-level `ma` key in `did:ma` documents. It specifies type
profiles, transport format, validation rules, the inbox addressing model, and
the fragment requirement for realm DID URLs.

For the generic `ma` namespace structure and reserved field names, see the
foundational [did-ma-fields-format.md](https://github.com/bahner/ma-spec/blob/main/did-ma-fields-format.md).

## 0. DID vs DID URL — Fragment Requirement

Per W3C DID Core §3.1, a DID is the bare identifier with no fragment:

```text
did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr
```

Per W3C DID Core §3.2, a DID URL MAY include a fragment:

```text
did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr#bahner
```

In the ma-realms runtime, the bare DID identifies the IPNS namespace (the
document identity). The DID URL with fragment identifies an addressable object
within that namespace — an avatar, a room, a mailbox, etc.

### 0.1 Fragment Rules (per W3C DID Core §8.1 rule 11)

1. All addressable realm objects MUST be referenced by DID URL (with fragment).
1. DID document `id` and `controller` fields MUST be bare DIDs (no fragment),
   per W3C §5.1.1 and §5.1.2.
1. Verification method `id` fields MUST be DID URLs (with fragment), per W3C
   §5.2.
1. The fragment is the local identifier within the world's namespace. The world
   passes fragments internally; DID URLs are the external messaging addresses.
1. When no specific fragment is warranted (e.g. a standalone agent identity),
   the RECOMMENDED default fragment is `#ma`.

### 0.2 Where DIDs and DID URLs Appear

| Context | Form | Example |
| --- | --- | --- |
| Document `id` | bare DID | `did:ma:k51...` |
| Document `controller` | bare DID | `did:ma:k51...` |
| Verification method `id` | DID URL | `did:ma:k51...#signing` |
| Verification method `controller` | bare DID | `did:ma:k51...` |
| `ma.inbox` | DID URL | `did:ma:k51...#bahner` |
| `ma.world` | bare DID | `did:ma:k51...` |
| `assertionMethod` / `keyAgreement` refs | DID URL | `did:ma:k51...#signing` |
| Message `from` / `to` | DID URL | `did:ma:k51...#bahner` |

## 1. Namespace Rules

1. All did:ma method-specific fields MUST be inside top-level ma.
1. `ma` MUST be a map when present.
1. Unknown ma fields SHOULD be ignored unless a stricter profile says otherwise.

## 2. Common Field Definitions

| Property | Key | Type | Description |
| --- | --- | --- | --- |
| Type | type | string | Subject type label. Allowed: world, agent, avatar, room, object. |
| Inbox | inbox | string | Canonical message address for this subject. |
| World | world | string | World DID root that this subject belongs to or targets. |
| Language | language | string | Preferred GNU LANGUAGE list, for example nb_NO:en_UK:en. |
| Requested TTL | requestedTTL | integer | Preferred retention/caching window in seconds. |
| Transports | transports | array of strings | Addressable transport endpoints for this DID subject. |
| Version | version | integer | Optional schema/profile compatibility marker. |

Notes:

1. inbox is canonical and SHOULD be present for all types in this profile.
1. currentInbox MUST NOT be used.
1. presenceHint MUST NOT be used.

## 3. Transport Format and ALPN Registry

### 3.1 Transport String Format

Each transports entry MUST use this form:

  /ma-iroh/<endpoint-id>/<alpn>

Where:

1. endpoint-id is the iroh endpoint identifier.
1. alpn is the lane identifier.

Example:

  /ma-iroh/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/ma/inbox/1

### 3.2 Minimal ALPN Set (Current Profile)

Required now:

1. ma/inbox/1

Optional runtime lanes (if supported by deployment):

1. ma/presence/1
1. ma/broadcast/1
1. ma/whisper/1
1. ma/ipfs/1

ma/ipfs/1 note:

1. MAY be offered as a public service for DID publish requests when clients lack
  direct Kubo access.
1. Availability is deployment-policy controlled (ACL/rate limits/quota).

Guidance:

1. New documents SHOULD prefer only ma/inbox/1 unless extra lanes are needed.
1. Consumers SHOULD iterate transports and use the first ALPN they support.

## 4. Type Profiles

| ma.type | Required | Recommended | Optional | Forbidden/Notes |
| --- | --- | --- | --- | --- |
| world | `type=world`, `inbox`, `transports` (must include at least one `ma/inbox/1`) | `language`, `version` | `requestedTTL` | `world` SHOULD NOT be set (self-reference redundant). |
| agent | `type=agent`, `inbox`, `transports` (must include at least one `ma/inbox/1`) | `language`, `version` | `requestedTTL` | `world` MUST NOT be set. Agent publishes transports like a world node. |
| avatar | `type=avatar`, `inbox`, `world` | `transports`, `version` | `language`, `requestedTTL` | If `transports` omitted, routing resolves via `world` transports. |
| room | `type=room`, `inbox`, `world` | `transports`, `version` | `language`, `requestedTTL` | If `transports` omitted, routing resolves via `world` transports. |
| object | `type=object`, `inbox`, `world` | `transports`, `version` | `language`, `requestedTTL` | If `transports` omitted, routing resolves via `world` transports. |

## 5. Validation Rules

1. type MUST be one of: world, agent, avatar, room, object.
1. inbox MUST be non-empty for all types in this profile.
1. If world is present, it MUST be a valid did:ma root DID.
1. If type is agent, world MUST NOT be present.
1. transports MUST be an array of non-empty strings when present.
1. Each transport entry SHOULD follow /ma-iroh/<endpoint-id>/<alpn>.
1. requestedTTL MUST be a non-negative integer when present.

## 6. DID Document Requirement by Type

This section defines whether a subject type must be represented by its own DID
document.

| Type | Own DID Document |
| --- | --- |
| world | REQUIRED |
| agent | REQUIRED |
| avatar | OPTIONAL |
| room | OPTIONAL |
| object | OPTIONAL |

Rules for optional types:

1. If transports is omitted, world MUST be present.
1. Consumers MUST use world DID document transports to deliver messages.
1. inbox remains required as canonical address even when transport is inherited.

## 7. Examples (ma section only)

### 7.1 world

```json
{
  "ma": {
    "type": "world",
    "inbox": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr",
    "transports": [
      "/ma-iroh/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/ma/inbox/1",
      "/ma-iroh/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/ma/ipfs/1"
    ],
    "version": 1
  }
}
```

### 7.2 agent

```json
{
  "ma": {
    "type": "agent",
    "inbox": "did:ma:k51agentroot#self",
    "transports": [
      "/ma-iroh/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/ma/inbox/1"
    ],
    "language": "nb_NO:en_UK:en",
    "requestedTTL": 3600,
    "version": 1
  }
}
```

### 7.3 avatar

```json
{
  "ma": {
    "type": "avatar",
    "inbox": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr#bahner",
    "world": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr",
    "version": 1
  }
}
```

### 7.4 room

```json
{
  "ma": {
    "type": "room",
    "inbox": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr#garden",
    "world": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr",
    "version": 1
  }
}
```

### 7.5 object

```json
{
  "ma": {
    "type": "object",
    "inbox": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr#mailbox",
    "world": "did:ma:k51qzi5uqu5dj9807pbuod1pplf0vxh8m4lfy3ewl9qbm2s8dsf9ugdf9gedhr",
    "version": 1
  }
}
```

## 8. Correlation With Base DID Format

Base DID format remains authoritative for:

1. @context, id, controller
1. verificationMethod, assertionMethod, keyAgreement
1. proof
1. serialization rules
