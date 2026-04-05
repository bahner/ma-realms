
actor skal aldri bruke kubo rpc direkte. INgen RPC kall port 5001, utelukkende fetches via gateways
vi skal alltid bruke did's for å referere til objekter i bakkant. Men vi kan bruke object.name for vise til brukeren
KISS
Vi unngår lange funksjoner og hardkodet kommandor parsing, og forsøker å legge dette til velegnede parsing moduler.
vi unngår duplikat code, i actor, world, bot og forsøker å legge dette til core
vi bruker signerte did dokumenter til å tilby ipfs lagring fra world
vi bryr oss aldri om bakoverkompatibilitet, siden jeg er eneste bruker og kan og gjør resett av alt på ett blunk ofte

Terminologi for kommando-scripting:
- method = handling skrevet i dot-notasjon på scope/target (f.eks. avatar.peek, avatar.apply, actor.apply)
- attribute = navngitt felt på target (f.eks. avatar.name, avatar.description)
- path = sammensatt dotted selector for nestede felter (f.eks. actor.ma.transports.peek)
- scoped kommandoer skal uttrykkes som .noterte.metoder, ikke space-form (for eksempel avatar.peek, ikke "avatar peek")
- verb brukes kun for naturlig språk-tolkning, ikke som primær term i scripted paths
