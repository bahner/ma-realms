# ma-realms-core TODOs

## MaFields migration from did-ma

The `ma:` extension namespace is opaque in did-ma (`Option<serde_json::Value>`).
Typed `MaFields` struct and all associated validation should live here.

### Error variants to add (removed from did-ma)

- `EmptyPresenceHint` — presence hint is empty
- `EmptyLang` — lang is empty
- `EmptyLanguagePreference` — language preference list is empty
- `InvalidLanguagePreferenceFormat` — language preference must follow GNU LANGUAGE format
- `InvalidMaWorld(String)` — invalid ma.world DID
- `InvalidMaCurrentInbox(String)` — invalid ma.currentInbox value
- `InvalidMaServices` — invalid ma.services value (expected object or array)
- `InvalidMaStateCid(String)` — invalid ma.stateCid
- `InvalidMaDeactivated(String)` — invalid ma.deactivated timestamp
- `InvalidMaVersionId(String)` — invalid ma.version

### Validation helpers to add (removed from did-ma)

- `is_valid_gnu_language_token()`
- `is_valid_gnu_language_list()`
- `is_hex_64()`
- `is_valid_inbox_hint()`

### MaFields struct

The typed struct with all field accessors (set/clear/get) for:
presenceHint, currentInbox, lang, language, type, world, requestedTTL,
services, stateCid, deactivated, version, pingIntervalSecs
