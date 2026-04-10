# ma-agent testing

Dette dokumentet dekker dagens `ma-agent`-modus etter opprydding av eldre onboarding-ruter.

## Scope

- Aktiv kjørbar modus: `ma-agent --daemon`
- Ingen onboarding/runtime-flyt i `ma-agent` uten `--daemon`

## Preconditions

1. Kubo API er tilgjengelig på `http://127.0.0.1:5001`
2. Rust-workspace bygger
3. Valgfritt: `ma-world` kjører hvis du tester daemon-endepunkter som krever world-kontekst

## Quick compile checks

Fra `ma-realms`-roten:

- `cargo check -p ma-core -p ma-world`
- `cargo check -p ma-agent`
- `make -C actor build`

Forventning: alle kommandoer lykkes.

## Runtime smoke tests

### Test A: help output

Kommando:

- `cargo run -p ma-agent -- --help`

Forventning:

1. Viser brukstekst
2. `--daemon` er dokumentert

### Test B: daemon startup

Kommando:

- `cargo run -p ma-agent -- --daemon`

Forventning:

1. Prosessen starter uten eldre onboarding-kall
2. Daemon lytter på konfigurert adresse (default eller config)

### Test C: non-daemon rejection

Kommando:

- `cargo run -p ma-agent --`

Forventning:

1. Prosessen avslutter med feilmelding som forklarer at non-daemon-modus er fjernet

## Exit criteria

1. `ma-agent --daemon` starter stabilt
2. Ingen referanser til den gamle onboarding-flyten i agent-runtime
3. Build/check for `ma-agent` er grønn
