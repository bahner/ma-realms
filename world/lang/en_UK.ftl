world.help.commands = commands: help | list | show [did] | describe [did] | lang [show|set <cid>|clear] | save | publish
world.list.empty = @world objects: (none)
world.private.status.on = @world private=on (new entrants must knock)
world.private.status.off = @world private=off (open entry)
world.owner.required = @world only the world owner can run that command.
world.private.on = private mode enabled; new entrants must knock
world.private.off = private mode disabled; entry is now open
world.private.usage = @world usage: @world.private [on|off|status]
world.knock.empty = knock inbox is empty
world.lang.show = lang_cid={$cid}
world.lang.cleared = lang_cid cleared (save world to persist)
world.lang.set = lang_cid set to {$cid} (save world to persist)
world.lang.usage.set = @world usage: @world.lang set <cid>
world.lang.usage = @world usage: @world.lang [show|set <cid>|clear]

# ─── avatar commands ─────────────────────────────────────────────────────────
avatar.look.no-description = Nothing special here.
avatar.look.no-exits = There are no obvious exits.
avatar.look.exits-label = Exits:
avatar.look.present-label = Present:
avatar.look.things-label = You see:
avatar.go.usage = Go where?
avatar.go.no-exit = No exit
avatar.go.locked = The way is locked:
avatar.go.denied = You cannot use exit
avatar.look.exit-locked = It is locked.
avatar.look.exit-open = It is open.
avatar.look.nothing = You see nothing called
avatar.inspect.usage = Inspect what?
avatar.inspect.not-found = Nothing to inspect:
