# TODO (actor)

## Editor / CodeMirror
- [ ] Undersok hvorfor editor fortsatt kan falle tilbake til textarea selv om status viser: "CodeMirror (esm.sh) aktiv med YAML-syntax".
- [ ] Legg inn tydelig runtime-telemetri for init-flyt i editoren (core load, YAML extension, EditorView mount, fallback-arsak).
- [ ] Bekreft om fallback trigges av race condition ved modal-open eller av en separat feil etter init.
- [ ] Nar stabilt: behold CodeMirror som default og bruk textarea kun ved reell init-feil.

### Repro notat
- Observasjon: "Editor: CodeMirror (esm.sh) aktiv med YAML-syntax", men bruker opplever fortsatt fallback.
- Kontekst: `ma-realms/actor/www/editor.js`, `.edit`-flyten.
- Dato: 2026-04-04

## BUG
ma.deactivated is supposed to be a timestamp, not a boolean
