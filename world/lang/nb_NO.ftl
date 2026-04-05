world.help.commands = kommandoer: help | list | show [did] | describe [did] | lang [show|set <cid>|clear]
world.list.empty = @world objekter: (ingen)
world.private.status.on = @world private=on (nye brukere må banke på)
world.private.status.off = @world private=off (åpen inngang)
world.owner.required = @world bare world-eier kan kjøre den kommandoen.
world.private.on = privat modus aktivert; nye brukere må banke på
world.private.off = privat modus deaktivert; inngang er nå åpen
world.private.usage = @world bruk: @@private [on|off|status]
world.knock.empty = knock-innboksen er tom
world.lang.show = lang_cid={$cid}
world.lang.cleared = lang_cid tømt (lagre verden for å persistere)
world.lang.set = lang_cid satt til {$cid} (lagre verden for å persistere)
world.lang.usage.set = @world bruk: @@lang set <cid>
world.lang.usage = @world bruk: @@lang [show|set <cid>|clear]

closet.empty = Du er i skapet og har ingen avatar ennå. Skriv 'help'.
closet.help = Skap-kommandoer: help | show | hear | apply [ipns_key_base64] | citizen [ipns_key_base64] | avatar.help | avatar.peek | avatar.apply [ipns_key_base64] | avatar.name: <tekst> | avatar.description: <tekst> | avatar.name peek | avatar.description peek | document.help | document.peek | document.id peek | document.ma.transports peek | document.publish [ipns_key_base64] | document.republish [ipns_key_base64] | document.apply [ipns_key_base64] | recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>
closet.help.prompt = Hvis actor-DID ikke finnes ennå: kjør apply først. Etter at actor er opprettet, sett avatar name/description. Skriv så 'go out' i actor-UI.
closet.actor.required = actor-identitet finnes ikke i denne verdenen ennå; kjør apply først
closet.actor.required.prompt = kjør apply først; etter actor-opprettelse kan du sette name/description
closet.apply.updated = endringer brukt. Du kan bli i skapet og fortsette å redigere; skriv 'go out' når du er klar.
closet.apply.accepted = søknad godkjent. skriv 'go out' i actor-UI for å gå inn i verden.
closet.recovery.usage = bruk: recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>
closet.recovery.passphrase.short = recovery-passphrase må være minst 8 tegn
closet.recovery.set.requires_did = recovery set krever en DID i denne skap-sesjonen (kjør apply først eller åpne skapet mens du er innlogget)
closet.recovery.set.stored = recovery-sjekksum lagret
closet.recovery.status.no_context = ingen DID-kontekst for denne skap-sesjonen
closet.recovery.status.configured = recovery er konfigurert
closet.recovery.status.not_configured = recovery er ikke konfigurert
closet.recovery.rekey.usage = bruk: recovery rekey <@handle> <passphrase>
closet.recovery.rekey.requires_new_did = closet_recovery_rekey_requires_new_did: run apply first
closet.recovery.rekey.done = rekey fullført for @{$handle} ({$old} -> {$new})
closet.hear.none = Ingen nye lobby-hendelser.
closet.hear.count = Hørte {$count} lobby-hendelse(r).
closet.entered = gikk inn i {$room} som @{$handle}
closet.command.unknown = Ukjent skap-kommando '{$method}'. Skriv 'help'.
closet.session.ready = skap-sesjon klar
closet.session.ready.prompt = Hvis actor er ny: kjør apply først. Sett deretter avatar-felter; du kan høre lobby-hendelser herfra.
closet.session.active_since = skap-sesjon aktiv siden {$created_at}
closet.citizenship.imported = borgerskap importert
closet.citizenship.prompt = Borgerskap innvilget. Knytt lokal identitet til returnert DID og gå inn i verden.
closet.did.published = DID-dokument publisert på /ipfs/{$cid}
