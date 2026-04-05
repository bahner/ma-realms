world.help.commands = commands: help | list | show [did] | describe [did] | lang [show|set <cid>|clear]
world.list.empty = @world objects: (none)
world.private.status.on = @world private=on (new entrants must knock)
world.private.status.off = @world private=off (open entry)
world.owner.required = @world only the world owner can run that command.
world.private.on = private mode enabled; new entrants must knock
world.private.off = private mode disabled; entry is now open
world.private.usage = @world usage: @@private [on|off|status]
world.knock.empty = knock inbox is empty
world.lang.show = lang_cid={$cid}
world.lang.cleared = lang_cid cleared (save world to persist)
world.lang.set = lang_cid set to {$cid} (save world to persist)
world.lang.usage.set = @world usage: @@lang set <cid>
world.lang.usage = @world usage: @@lang [show|set <cid>|clear]

closet.empty = You are in the closet and have no avatar yet. Type 'help'.
closet.help = Closet commands: help | show | hear | apply [ipns_key_base64] | citizen [ipns_key_base64] | avatar.help | avatar.peek | avatar.apply [ipns_key_base64] | avatar.name: <text> | avatar.description: <text> | avatar.name peek | avatar.description peek | document.help | document.peek | document.id peek | document.ma.transports peek | document.publish [ipns_key_base64] | document.republish [ipns_key_base64] | document.apply [ipns_key_base64] | recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>
closet.help.prompt = If actor DID does not exist yet: run apply first. After actor is created, set avatar name/description. Then type 'go out' in the actor UI.
closet.actor.required = actor identity does not exist in this world yet; run apply first
closet.actor.required.prompt = run apply first; after actor creation you can set name/description
closet.apply.updated = changes applied. You can stay in the closet and keep editing; type 'go out' when ready.
closet.apply.accepted = application accepted. type 'go out' in the actor UI to enter the world.
closet.recovery.usage = usage: recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>
closet.recovery.passphrase.short = recovery passphrase must be at least 8 characters
closet.recovery.set.requires_did = recovery set requires a DID in this closet session (run apply first or open closet while logged in)
closet.recovery.set.stored = recovery checksum stored
closet.recovery.status.no_context = no DID context for this closet session
closet.recovery.status.configured = recovery is configured
closet.recovery.status.not_configured = recovery is not configured
closet.recovery.rekey.usage = usage: recovery rekey <@handle> <passphrase>
closet.recovery.rekey.requires_new_did = closet_recovery_rekey_requires_new_did: run apply first
closet.recovery.rekey.done = rekey complete for @{$handle} ({$old} -> {$new})
closet.hear.none = No new lobby events.
closet.hear.count = Heard {$count} lobby event(s).
closet.entered = entered {$room} as @{$handle}
closet.command.unknown = Unknown closet command '{$method}'. Type 'help'.
closet.session.ready = closet session ready
closet.session.ready.prompt = If actor is new: run apply first. Then set avatar profile fields; you can hear lobby events from here.
closet.session.active_since = closet session active since {$created_at}
closet.citizenship.imported = citizenship imported
closet.citizenship.prompt = Citizenship granted. Rebind your local identity to the returned DID and enter the world.
closet.did.published = did document published at /ipfs/{$cid}
