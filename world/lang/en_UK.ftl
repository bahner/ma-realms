world.help.commands = commands: help | list | show [did] | describe [did] | lang [show|set <cid>|clear]
world.list.empty = @world objects: (none)
world.private.status.on = @world private=on (new entrants must knock)
world.private.status.off = @world private=off (open entry)
world.owner.required = @world only the world owner can run that command.
world.private.on = private mode enabled; new entrants must knock
world.private.off = private mode disabled; entry is now open
world.private.usage = @world usage: @@private [on|off|status]
world.knock.empty = knock inbox is empty
world.lang.show = lang_cid={ $cid }
world.lang.cleared = lang_cid cleared (save world to persist)
world.lang.set = lang_cid set to { $cid } (save world to persist)
world.lang.usage.set = @world usage: @@lang set <cid>
world.lang.usage = @world usage: @@lang [show|set <cid>|clear]

closet.empty = You are in the closet and have no avatar yet. Type 'help'.
closet.help = Closet commands: help | show | hear | name <text> | description <text> | alias <text> | apply [ipns_key_base64] | citizen [ipns_key_base64] | recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>
closet.help.prompt = You are in the closet with no avatar yet. Required: name + description + alias. Then run apply. When done, type 'go lobby' in the actor UI.
closet.required_fields = required fields are: name, description, alias
closet.required_fields.prompt = set name/description/alias, then run apply
closet.apply.updated = changes applied. You can stay in the closet and keep editing; type 'go lobby' when ready.
closet.apply.accepted = application accepted. type 'go lobby' in the actor UI to enter the world.
closet.recovery.usage = usage: recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>
closet.recovery.passphrase.short = recovery passphrase must be at least 8 characters
closet.recovery.set.requires_did = recovery set requires a DID in this closet session (run apply first or open closet while logged in)
closet.recovery.set.stored = recovery checksum stored
closet.recovery.status.no_context = no DID context for this closet session
closet.recovery.status.configured = recovery is configured
closet.recovery.status.not_configured = recovery is not configured
closet.recovery.rekey.usage = usage: recovery rekey <@handle> <passphrase>
closet.recovery.rekey.requires_new_did = recovery rekey requires a new DID in this session (run citizen first)
closet.recovery.rekey.done = rekey complete for @{ $handle } ({ $old } -> { $new })
closet.hear.none = No new lobby events.
closet.hear.count = Heard { $count } lobby event(s).
closet.entered = entered { $room } as @{ $handle }
closet.command.unknown = Unknown closet command '{ $verb }'. Type 'help'.
closet.session.ready = closet session ready
closet.session.ready.prompt = Answer profile questions while waiting; you can hear lobby events from here.
closet.session.active_since = closet session active since { $created_at }
closet.citizenship.imported = citizenship imported
closet.citizenship.prompt = Citizenship granted. Rebind your local identity to the returned DID and enter the world.
closet.did.published = did document published
