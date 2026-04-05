
actor skal aldri bruke kubo rpc direkte. INgen RPC kall port 5001, utelukkende fetches via gateways
vi skal alltid bruke did's for å referere til objekter i bakkant. Men vi kan bruke object.name for vise til brukeren
KISS
Vi unngår lange funksjoner og hardkodet kommandor parsing, og forsøker å legge dette til velegnede parsing moduler.
vi unngår duplikat code, i actor, world, bot og forsøker å legge dette til core
vi bruker signerte did dokumenter til å tilby ipfs lagring fra world

