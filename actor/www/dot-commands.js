import { isMaDid } from './did.js';

export function createDotCommands({
  state,
  appendSystemUi,
  appendMessage,
  uiText,
  setMyHomeTarget,
  getMyHomeTarget,
  humanizeIdentifier,
  isPrintableAliasLabel,
  saveAliasBook,
  setDebugMode,
  setLogEnabled,
  setLogLevel,
  setDialogIdStyle,
  setAliasRewriteEnabled,
  setMessageTtl,
  getMessageTtl,
  setTemporaryMessageTtlOverride,
  clearTemporaryMessageTtlOverride,
  getTemporaryMessageTtlOverride,
  setBatchTimeoutSeconds,
  setBatchRetryCount,
  runBatchCommands,
  batchStatusLine,
  onAliasBookChanged,
  onDotEdit,
  onDotEval,
  onDotInspect,
  resolveAliasInput,
  resolveCommandTargetDidOrToken,
  lookupDidInCurrentRoom,
  sendWorldCommandQuery,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  refillCommandInputWithActiveTarget,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
  pollDirectInbox,
  pollCurrentHomeEvents,
  prepareIdentityDocumentForSend,
  publishIdentityToWorldDid,
  sendWhisperToDid,
  sendMessageToDid,
  broadcastEnabled,
  setBroadcastEnabled,
  broadcastSend,
}) {
  const DYNAMIC_SPECIAL_ALIASES = new Set(['@here', '@me', '@world', '@avatar']);

  function normalizeAliasNameToken(input) {
    const raw = String(input || '').trim();
    if (!raw) return '';
    return raw;
  }

  function normalizeAliasTargetToken(input) {
    const raw = String(input || '').trim();
    if (!raw) return '';
    if (/\s/u.test(raw)) {
      return '';
    }
    return raw;
  }

  function resolveAliasBookKey(input) {
    const raw = String(input || '').trim();
    if (!raw) return '';
    if (Object.prototype.hasOwnProperty.call(state.aliasBook, raw)) {
      return raw;
    }
    const alt = raw.startsWith('@') ? raw.slice(1) : `@${raw}`;
    if (alt && Object.prototype.hasOwnProperty.call(state.aliasBook, alt)) {
      return alt;
    }
    return raw;
  }

  function showIdentityDocument() {
    if (!state.identity) {
      appendSystemUi('No identity loaded. Create or unlock an identity first.', 'Ingen identitet lastet. Opprett eller lås opp en identitet først.');
      return true;
    }

    Promise.resolve()
      .then(async () => {
        if (typeof prepareIdentityDocumentForSend === 'function') {
          await prepareIdentityDocumentForSend();
        }

        const documentJson = String(state.identity?.document_json || '').trim();
        if (!documentJson) {
          appendMessage('system', 'No local DID document available in identity bundle yet.');
          return;
        }

        try {
          const parsed = JSON.parse(documentJson);
          appendMessage('system', JSON.stringify(parsed, null, 2));
        } catch {
          appendMessage('system', documentJson);
        }
      })
      .catch((error) => {
        appendMessage('system', `Identity prepare failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    return true;
  }

  function publishIdentity(args) {
    if (!state.identity) {
      appendSystemUi('No identity loaded. Create or unlock an identity first.', 'Ingen identitet lastet. Opprett eller lås opp en identitet først.');
      return true;
    }

    let worldDid = '';
    if (args.length === 0) {
      const home = String((typeof getMyHomeTarget === 'function' ? getMyHomeTarget() : state.myHome) || '').trim();
      if (home && isMaDid(home)) {
        worldDid = home;
      }
      if (!worldDid) {
        appendMessage('system', 'Usage: my.identity.publish [<did:ma:world>] (no home set — use my.home first)');
        return true;
      }
    } else {
      const raw = String(args[0] || '').trim();
      worldDid = isMaDid(raw) ? raw : String(resolveAliasInput(raw) || '').trim();
    }

    if (!isMaDid(worldDid)) {
      appendMessage('system', 'Usage: my.identity.publish [<did:ma:world>]');
      return true;
    }
    appendMessage('system', `Publishing identity to ${worldDid} via ma/ipfs/1...`);
    Promise.resolve()
      .then(async () => {
        const result = await publishIdentityToWorldDid(worldDid);
        if (result?.ok) {
          appendMessage('system', result.message || 'DID document published successfully.');
        } else {
          appendMessage('system', result?.message || 'Publish failed (world returned ok=false).');
        }
      })
      .catch((error) => {
        appendMessage('system', `Identity publish failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    return true;
  }

  function setHome(args) {
    if (args.length === 0) {
      const current = String((typeof getMyHomeTarget === 'function' ? getMyHomeTarget() : state.myHome) || '').trim();
      appendMessage('system', current || '(home not set)');
      return true;
    }

    if (args.length !== 1) {
      appendMessage('system', 'Usage: my.home [<did:ma:<world>#<room>>]');
      return true;
    }

    const target = String(args[0] || '').trim();
    if (!isMaDid(target) || !target.includes('#')) {
      appendMessage('system', 'Usage: my.home [<did:ma:<world>#<room>>]');
      return true;
    }

    if (typeof setMyHomeTarget === 'function') {
      setMyHomeTarget(target);
    } else {
      state.myHome = target;
    }
    if (typeof onAliasBookChanged === 'function') {
      onAliasBookChanged();
    }
    appendMessage('system', `Home set: ${target}`);

    if (!state.identity) {
      appendSystemUi(
        'No identity loaded yet. Create or unlock identity, then publish with @my.identity.publish.',
        'Ingen identitet lastet enda. Opprett eller lås opp identitet, og publiser med @my.identity.publish.'
      );
      return true;
    }

    appendSystemUi(
      'Birth: publishing identity to your home world now (first time can take a little while).',
      'Fødsel: publiserer identiteten din til hjemmeverden nå (første gang kan ta litt tid).'
    );
    publishIdentity([target]);
    return true;
  }

  function handleAliasRewrite(args) {
    if (args.length === 0) {
      appendMessage('system', `${state.aliasRewriteEnabled ? 'on' : 'off'}`);
      return true;
    }
    if (args.length !== 1) {
      appendMessage('system', 'Usage: @my.aliases.rewrite [on|off]');
      return true;
    }
    const mode = String(args[0] || '').trim().toLowerCase();
    if (mode !== 'on' && mode !== 'off') {
      appendMessage('system', 'Usage: @my.aliases.rewrite [on|off]');
      return true;
    }
    if (typeof setAliasRewriteEnabled === 'function') {
      setAliasRewriteEnabled(mode === 'on');
    }
    appendMessage('system', `Alias rewrite is now ${mode}.`);
    return true;
  }

  function handleAliases(subcommand, args, pathSuffix = '') {
    if (!subcommand && !pathSuffix && args.length === 0) {
      const entries = Object.entries(state.aliasBook);
      if (entries.length === 0) {
        appendMessage('system', 'No aliases saved yet.');
        return true;
      }
      for (const [name, address] of entries) {
        appendMessage('system', `@my.aliases.${name} ${address}`);
      }
      return true;
    }

    if (pathSuffix) {
      if (pathSuffix === 'rewrite') {
        return handleAliasRewrite(args);
      }

      if (pathSuffix === 'add' || pathSuffix === 'del') {
        return handleAliases(pathSuffix, args);
      }

      const inputAliasName = normalizeAliasNameToken(pathSuffix);
      const aliasName = resolveAliasBookKey(inputAliasName);
      if (!aliasName) {
        appendMessage('system', 'Usage: @my.aliases.<name>');
        return true;
      }
      if (!Object.prototype.hasOwnProperty.call(state.aliasBook, aliasName)) {
        appendMessage('system', `Alias not found: ${inputAliasName}`);
        return true;
      }
      appendMessage('system', `@my.aliases.${aliasName} ${state.aliasBook[aliasName]}`);
      return true;
    }

    const action = String(subcommand || '').trim().toLowerCase();
    if (!action) {
      appendMessage('system', 'Usage: @my.aliases | @my.aliases add <name> <target> | @my.aliases del <name> | @my.aliases.<name> | @my.aliases.rewrite [on|off]');
      return true;
    }

    if (action === 'add') {
      if (args.length !== 2) {
        appendMessage('system', 'Usage: @my.aliases add <name> <target>');
        return true;
      }

      const name = normalizeAliasNameToken(args[0]);
      const address = normalizeAliasTargetToken(args[1]);

      if (DYNAMIC_SPECIAL_ALIASES.has(name)) {
        appendMessage('system', `Alias ${name} is managed automatically.`);
        return true;
      }

      if (!isPrintableAliasLabel(name)) {
        appendMessage('system', 'Alias name must be printable UTF-8 (no spaces/control chars), up to 64 chars.');
        return true;
      }

      if (!address) {
        appendMessage('system', 'Alias value must be a non-empty target without spaces.');
        return true;
      }

      state.aliasBook[name] = address;
      saveAliasBook();
      if (typeof onAliasBookChanged === 'function') {
        onAliasBookChanged();
      }
      appendMessage('system', `Alias saved: ${name} => ${address}`);
      return true;
    }

    if (action === 'del') {
      if (args.length !== 1) {
        appendMessage('system', 'Usage: @my.aliases del <name>');
        return true;
      }
      const inputName = normalizeAliasNameToken(args[0]);
      const name = resolveAliasBookKey(inputName);
      if (DYNAMIC_SPECIAL_ALIASES.has(name)) {
        appendMessage('system', `Alias ${name} is managed automatically.`);
        return true;
      }
      if (!name || !Object.prototype.hasOwnProperty.call(state.aliasBook, name)) {
        appendMessage('system', `Alias not found: ${inputName || String(args[0] || '').trim()}`);
        return true;
      }
      delete state.aliasBook[name];
      saveAliasBook();
      if (typeof onAliasBookChanged === 'function') {
        onAliasBookChanged();
      }
      appendMessage('system', `Alias removed: ${name}`);
      return true;
    }

    if (action === 'rewrite') {
      return handleAliasRewrite(args);
    }

    appendMessage('system', 'Usage: @my.aliases | @my.aliases add <name> <target> | @my.aliases del <name> | @my.aliases.<name> | @my.aliases.rewrite [on|off]');
    return true;
  }

  function handleActorStateAliases(subcommand, args, pathSuffix = '') {
    const runtime = (state.systemAliases && typeof state.systemAliases === 'object')
      ? state.systemAliases
      : {};

    if (!subcommand && !pathSuffix && args.length === 0) {
      const entries = Object.entries(runtime);
      if (entries.length === 0) {
        appendMessage('system', 'No actor runtime aliases set.');
        return true;
      }
      for (const [name, address] of entries) {
        appendMessage('system', `@actor.state.aliases.${name} ${address}`);
      }
      return true;
    }

    if (pathSuffix) {
      const raw = normalizeAliasNameToken(pathSuffix);
      const key = raw.startsWith('@') ? raw : `@${raw}`;
      if (!Object.prototype.hasOwnProperty.call(runtime, key)) {
        appendMessage('system', `Runtime alias not found: ${raw}`);
        return true;
      }
      appendMessage('system', `@actor.state.aliases.${key} ${runtime[key]}`);
      return true;
    }

    const action = String(subcommand || '').trim().toLowerCase();
    if (!action || action === 'list' || action === 'show') {
      return handleActorStateAliases('', [], '');
    }

    appendMessage('system', 'Usage: @actor.state.aliases | @actor.state.aliases.<name> (read-only runtime aliases)');
    return true;
  }

  function handleMail(subcommand, args) {
    const action = String(subcommand || 'list').toLowerCase();
    const list = Array.isArray(state.mailbox) ? state.mailbox : [];

    if (action === 'list') {
      if (!list.length) {
        appendSystemUi('Mailbox is empty.', 'Mailbox er tom.');
        return true;
      }
      appendMessage('system', `Mailbox (${list.length}):`);
      for (const entry of list) {
        const preview = String(entry.content_text || '').replace(/\s+/g, ' ').slice(0, 80) || '(binary)';
        appendMessage(
          'system',
          `  #${entry.id} from=${humanizeIdentifier(entry.from_did || '(unknown)')} type=${entry.content_type || '(unknown)'} text=${preview}`
        );
      }
      appendSystemUi('Use @my.mail pick <id>, @my.mail reply <id> <text>, or @my.mail delete <id>.', 'Bruk @my.mail pick <id>, @my.mail reply <id> <tekst>, eller @my.mail delete <id>.');
      return true;
    }

    if (action === 'pick' || action === 'show') {
      const idRaw = String(args[0] || '').trim();
      const id = Number(idRaw);
      if (!Number.isFinite(id) || id <= 0) {
        appendMessage('system', 'Usage: my.mail pick <id>');
        return true;
      }
      const entry = list.find((item) => Number(item.id) === id);
      if (!entry) {
        appendMessage('system', `Mailbox entry not found: ${id}`);
        return true;
      }
      appendMessage('system', `my.mail pick ${id}`);
      appendMessage('system', `  from: ${humanizeIdentifier(entry.from_did || '(unknown)')}`);
      appendMessage('system', `  endpoint: ${humanizeIdentifier(entry.from_endpoint || '(unknown)')}`);
      appendMessage('system', `  type: ${entry.content_type || '(unknown)'}`);
      appendMessage('system', `  text: ${entry.content_text || '(binary)'}`);
      appendMessage('system', `  cbor: ${entry.message_cbor_b64 || '(missing)'}`);
      return true;
    }

    if (action === 'delete' || action === 'del' || action === 'rm') {
      const idRaw = String(args[0] || '').trim();
      const id = Number(idRaw);
      if (!Number.isFinite(id) || id <= 0) {
        appendMessage('system', 'Usage: my.mail delete <id>');
        return true;
      }
      const before = list.length;
      state.mailbox = list.filter((item) => Number(item.id) !== id);
      if (state.mailbox.length === before) {
        appendMessage('system', `Mailbox entry not found: ${id}`);
        return true;
      }
      appendMessage('system', `Deleted mailbox entry #${id}.`);
      return true;
    }

    if (action === 'reply') {
      const idRaw = String(args[0] || '').trim();
      const id = Number(idRaw);
      const replyText = args.slice(1).join(' ').trim();
      if (!Number.isFinite(id) || id <= 0 || !replyText) {
        appendMessage('system', 'Usage: my.mail reply <id> <text>');
        return true;
      }
      const entry = list.find((item) => Number(item.id) === id);
      if (!entry) {
        appendMessage('system', `Mailbox entry not found: ${id}`);
        return true;
      }
      const targetDid = String(entry.from_did || '').trim();
      if (!isMaDid(targetDid)) {
        appendMessage('system', `Mailbox entry #${id} has no valid sender DID.`);
        return true;
      }
      sendMessageToDid(targetDid, replyText)
        .then(() => {
          appendMessage('system', `Reply sent to ${humanizeIdentifier(targetDid)} from mailbox #${id}.`);
        })
        .catch((error) => {
          appendMessage('system', `Reply failed: ${error instanceof Error ? error.message : String(error)}`);
        });
      return true;
    }

    if (action === 'clear') {
      const cleared = list.length;
      state.mailbox = [];
      appendMessage('system', `Mailbox cleared (${cleared} entries).`);
      return true;
    }

    appendMessage('system', 'Usage: @my.mail [list|pick <id>|reply <id> <text>|delete <id>|clear]');
    return true;
  }

  function parseLocalCommand(input) {
    const trimmed = String(input || '').trim();
    if (!trimmed) {
      return false;
    }
    if (trimmed.startsWith('.')) {
      return parseDot(trimmed);
    }

    const [verbRaw, ...args] = trimmed.split(/\s+/);
    const verbToken = String(verbRaw || '').trim();
    const rawCommand = verbToken.toLowerCase();
    const actorCommandMode = rawCommand.startsWith('@actor.');
    let command = rawCommand;
    if (rawCommand.startsWith('@actor.')) {
      command = `my.${rawCommand.slice('@actor.'.length)}`;
    } else if (rawCommand.startsWith('@my.')) {
      command = `my.${rawCommand.slice('@my.'.length)}`;
    }
    if (!command.startsWith('my.')) {
      return false;
    }

    if (actorCommandMode) {
      const firstArg = String(args[0] || '').trim().toLowerCase();
      const isAliasMutation = command === 'my.aliases'
        && (firstArg === 'add' || firstArg === 'del' || firstArg === 'rewrite');
      const isIdentityMutation = command === 'my.identity.publish' || command === 'my.home';
      const isMailMutation = (command === 'my.mail' || command.startsWith('my.mail.'))
        && (firstArg === 'reply' || firstArg === 'delete' || firstArg === 'del' || firstArg === 'rm' || firstArg === 'clear'
          || command === 'my.mail.reply' || command === 'my.mail.delete' || command === 'my.mail.del' || command === 'my.mail.rm' || command === 'my.mail.clear');
      const aliasPath = command.startsWith('my.aliases.') ? command.slice('my.aliases.'.length) : '';
      const isAliasPathMutation = aliasPath === 'add' || aliasPath === 'del' || aliasPath === 'rewrite';

      if (isAliasMutation || isIdentityMutation || isMailMutation || isAliasPathMutation) {
        appendMessage('system', '@actor.* is read-only for now. Use @my.* for mutating commands.');
        return true;
      }
    }

    if (command === 'my.did') {
      if (!state.identity) {
        appendSystemUi('No identity loaded. Create or unlock an identity first.', 'Ingen identitet lastet. Opprett eller lås opp en identitet først.');
        return true;
      }
      appendMessage('system', String(state.identity?.did || '').trim() || '(missing did)');
      return true;
    }

    if (command === 'my.identity') {
      return showIdentityDocument();
    }

    if (command === 'my.identity.publish') {
      return publishIdentity(args);
    }

    if (command === 'my.home') {
      return setHome(args);
    }

    if (command === 'my.aliases') {
      return handleAliases(args[0] || '', args.slice(1));
    }

    if (command.startsWith('my.aliases.')) {
      return handleAliases('', args, command.slice('my.aliases.'.length));
    }

    if (command === 'my.state.aliases') {
      return handleActorStateAliases(args[0] || '', args.slice(1));
    }

    if (command.startsWith('my.state.aliases.')) {
      return handleActorStateAliases('', args, command.slice('my.state.aliases.'.length));
    }

    if (command === 'my.mail') {
      return handleMail(args[0] || 'list', args.slice(1));
    }

    if (command.startsWith('my.mail.')) {
      return handleMail(command.slice('my.mail.'.length), args);
    }

    appendMessage('system', uiText(
      `Unknown command: ${verbToken}. Try .help.`,
      `Ukjent kommando: ${verbToken}. Prøv .help.`
    ));
    return true;
  }

  function parseDot(input) {
    const trimmed = String(input || '').trim();
    if (!trimmed.startsWith('.')) {
      return false;
    }

    const rest = trimmed.slice(1).trim();
    if (!rest) {
      appendSystemUi('Usage: .<command> (try .help)', 'Bruk: .<kommando> (prøv .help)');
      return true;
    }

    const [verbRaw, ...args] = rest.split(/\s+/);
    const verbToken = String(verbRaw || '').trim();
    const dotCommand = verbToken.toLowerCase();
    const tail = args.join(' ').trim();

    if (dotCommand === 'help') {
      appendSystemUi('Local commands:', 'Lokale kommandoer:');
      appendSystemUi('  .help                      - this message', '  .help                      - denne meldingen');
      appendSystemUi('My namespace (self/config):', 'My-navnerom (selv/konfig):');
      appendSystemUi('  @my.did               - show your identity DID', '  @my.did               - vis identitets-DID-en din');
      appendSystemUi('  @my.identity          - show local pre-publish DID document as raw JSON', '  @my.identity          - vis lokalt DID-dokument (før publisering) som rå JSON');
      appendSystemUi('  @my.identity.publish [<did:ma:world>] - publish DID document to world via ma/ipfs/1 (defaults to home world)', '  @my.identity.publish [<did:ma:world>] - publiser DID-dokument til verden via ma/ipfs/1 (standard: hjemmeverden)');
      appendSystemUi('  @my.home [<did:ma:...#room>] - show/set home target', '  @my.home [<did:ma:...#room>] - vis/sett home-mål');
      appendSystemUi('    birth flow: @my.home also auto-publishes your identity; first publish can take a little while', '    fødselsflyt: @my.home publiserer også identiteten automatisk; første publisering kan ta litt tid');
      appendSystemUi('  @my.aliases add <name> <target> - add/update alias (no spaces in target)', '  @my.aliases add <navn> <mål> - legg til/oppdater alias (ingen mellomrom i mål)');
      appendSystemUi('    note: @here/@me/@world/@avatar are updated automatically', '    merk: @here/@me/@world/@avatar oppdateres automatisk');
      appendSystemUi('  @my.aliases del <name>      - remove alias', '  @my.aliases del <navn>      - fjern alias');
      appendSystemUi('  @my.aliases                 - list aliases', '  @my.aliases                 - list alias');
      appendSystemUi('  @my.aliases.<name>          - show address for one alias', '  @my.aliases.<navn>          - vis adresse for ett alias');
      appendSystemUi('  @my.aliases.rewrite [on|off]- rewrite aliases to DID before parsing', '  @my.aliases.rewrite [on|off]- skriv alias om til DID før parsing');
      appendSystemUi('  @actor.state.aliases        - list runtime symbolic aliases (read-only)', '  @actor.state.aliases        - vis symbolske runtime-alias (kun lesing)');
      appendSystemUi('  @actor.state.aliases.<name> - show one runtime alias value', '  @actor.state.aliases.<navn> - vis verdi for ett runtime-alias');
      appendSystemUi('  @my.mail [list|pick|reply|delete|clear] - inspect mailbox queue', '  @my.mail [list|pick|reply|delete|clear] - inspiser mailbox-kø');
      appendSystemUi('  @actor.<command>          - read-only actor namespace (use @my.* to mutate)', '  @actor.<kommando>          - skrivebeskyttet actor-navnerom (bruk @my.* for endringer)');
      appendSystemUi('Dot commands (local tools):', 'Punktkommandoer (lokale verktøy):');
      appendSystemUi('  .inspect @here|@me|@exit <name>|<object>- inspect room/me/exit/object and discover DID/CIDs', '  .inspect @here|@me|@exit <navn>|<objekt>- inspiser rom/meg/utgang/objekt og finn DID/CID');
      appendSystemUi('  .use <object|did> [as alias] - set local default target', '  .use <objekt|did> [as alias] - sett lokal standardtarget');
      appendSystemUi('  .unuse @alias              - clear local default target', '  .unuse @alias              - fjern lokal standardtarget');
      appendSystemUi('  .edit [@here|@me|@exit <name>|did:ma:<world>#<room>] - open editor', '  .edit [@here|@me|@exit <navn>|did:ma:<world>#<room>] - åpne editor');
      appendSystemUi('  .eval <cid|alias>          - run script from IPFS CID or alias', '  .eval <cid|alias>          - kjør script fra IPFS CID eller alias');
      appendSystemUi('  .refresh                   - fetch latest room state and events now', '  .refresh                   - hent siste romtilstand og hendelser nå');
      appendSystemUi('  .ping [@world|@here|@avatar]- local RTT ping via command path', '  .ping [@world|@here|@avatar]- lokal RTT-ping via kommandoløypa');
      appendSystemUi('  .smoke [alias]             - run connectivity smoke test', '  .smoke [alias]             - kjør enkel tilkoblingstest');
      appendSystemUi('  .debug [on|off]            - toggle debug logs', '  .debug [on|off]            - slå debuglogger av/på');
      appendSystemUi('  .broadcast                 - show broadcast status', '  .broadcast                 - vis kringkastingsstatus');
      appendSystemUi('  .broadcast.enabled [on|off] - toggle broadcast reception', '  .broadcast.enabled [på|av] - slå kringkasting av/på');
      appendSystemUi('  .broadcast.send <message>  - send broadcast to all ma nodes', '  .broadcast.send <melding>  - send kringkasting til alle ma-noder');
      appendSystemUi('  .log                       - show log settings', '  .log                       - vis logginnstillinger');
      appendSystemUi('  .log.enabled [true|false]  - get/set console logging enabled', '  .log.enabled [true|false]  - hent/sett om konsoll-logging er aktiv');
      appendSystemUi('  .log.level [warn|info|debug|error] - get/set console log level', '  .log.level [warn|info|debug|error] - hent/sett loggnivå i konsoll');
      appendSystemUi('  .dialog.id [alias|fragment|did] - get/set DID rendering in dialog', '  .dialog.id [alias|fragment|did] - hent/sett DID-visning i dialog');
      appendSystemUi('  .msg.ttl                    - show actor message TTL defaults', '  .msg.ttl                    - vis standard TTL for actor-meldinger');
      appendSystemUi('  .msg.ttl <chat|cmd|whisper> <seconds> - set default TTL per message type', '  .msg.ttl <chat|cmd|whisper> <sekunder> - sett standard TTL per meldingstype');
      appendSystemUi('  .ttl [seconds]              - show/set temporary TTL override for outgoing messages', '  .ttl [sekunder]             - vis/sett midlertidig TTL-override for utgående meldinger');
      appendSystemUi('  .ttl.unset                  - clear temporary TTL override and use defaults', '  .ttl.unset                  - fjern midlertidig TTL-override og bruk standardverdier');
      appendSystemUi('  .batch.start <seconds>      - start collecting a local batch with per-command timeout', '  .batch.start <sekunder>     - start innsamling av lokal batch med timeout per kommando');
      appendSystemUi('  .batch.retry <count>        - set retries per batch command', '  .batch.retry <antall>       - sett retry per batch-kommando');
      appendSystemUi('  .batch                      - run collected batch (fail-fast)', '  .batch                      - kjør innsamlet batch (stopp ved feil)');
      appendSystemUi('Gameplay (bare, no prefix):', 'Gameplay (bart, uten prefiks):');
      appendSystemUi('  go did:ma:<world>#<room>   - connect when currently disconnected', '  go did:ma:<world>#<room>   - koble til når du er frakoblet');
      appendMessage('system', '  pick up <object>           - pick up object before open/list/accept actions');
      appendSystemUi('  go north                   - navigate (server resolves exit)', '  go north                   - naviger (server løser utgang)');
      appendSystemUi('  look                       - describe current room', '  look                       - beskriv naverende rom');
      appendSystemUi('  attack goblin              - gameplay command sent to world', '  attack goblin              - gameplay-kommando sendt til world');
      appendSystemUi('  @did:ma:<world>#<room> poll - refresh room metadata on demand', '  @did:ma:<world>#<room> poll - oppdater rommetadata ved behov');
      appendMessage('system', "  'Hello world               - shorthand for @me say Hello world");
      appendSystemUi('  @target command args       - send command to actor', '  @target command args       - send kommando til actor');
      appendMessage('system', "  @target 'message           - whisper to actor (E2E)");
      appendSystemUi('  @world.<command>           - world-admin command', '  @world.<kommando>          - world-admin-kommando');
      return true;
    }

    if (dotCommand === 'debug') {
      if (args.length === 0) {
        setDebugMode(!state.debug);
      } else {
        const mode = String(args[0] || '').trim().toLowerCase();
        if (mode === 'on' || mode === '1' || mode === 'true') {
          setDebugMode(true);
        } else if (mode === 'off' || mode === '0' || mode === 'false') {
          setDebugMode(false);
        } else {
          appendMessage('system', 'Usage: .debug [on|off]');
          return true;
        }
      }
      return true;
    }

    if (dotCommand === 'my.aliases') {
      return handleAliases(args[0] || '', args.slice(1));
    }

    if (dotCommand.startsWith('my.aliases.')) {
      return handleAliases('', args, dotCommand.slice('my.aliases.'.length));
    }

    if (dotCommand === 'log') {
      if (args.length !== 0) {
        appendMessage('system', 'Usage: .log');
        return true;
      }
      appendMessage('system', `.log.enabled ${state.logEnabled ? 'true' : 'false'}`);
      appendMessage('system', `.log.level ${state.logLevel}`);
      return true;
    }

    if (dotCommand === 'log.enabled') {
      if (args.length === 0) {
        appendMessage('system', `${state.logEnabled ? 'true' : 'false'}`);
        return true;
      }
      if (args.length !== 1) {
        appendMessage('system', 'Usage: .log.enabled [true|false]');
        return true;
      }
      const mode = String(args[0] || '').trim().toLowerCase();
      if (mode === 'true' || mode === '1' || mode === 'on') {
        setLogEnabled(true);
        return true;
      }
      if (mode === 'false' || mode === '0' || mode === 'off') {
        setLogEnabled(false);
        return true;
      }
      appendMessage('system', 'Usage: .log.enabled [true|false]');
      return true;
    }

    if (dotCommand === 'log.level') {
      if (args.length === 0) {
        appendMessage('system', `${state.logLevel}`);
        return true;
      }
      if (args.length !== 1) {
        appendMessage('system', 'Usage: .log.level [warn|info|debug|error]');
        return true;
      }
      const level = String(args[0] || '').trim().toLowerCase();
      if (level !== 'warn' && level !== 'info' && level !== 'debug' && level !== 'error') {
        appendMessage('system', 'Usage: .log.level [warn|info|debug|error]');
        return true;
      }
      setLogLevel(level);
      return true;
    }

    if (dotCommand === 'msg.ttl') {
      if (args.length === 0) {
        appendMessage('system', `actor.msg.chat.ttl ${getMessageTtl('chat')}`);
        appendMessage('system', `actor.msg.cmd.ttl ${getMessageTtl('cmd')}`);
        appendMessage('system', `actor.msg.whisper.ttl ${getMessageTtl('whisper')}`);
        return true;
      }

      if (args.length !== 2) {
        appendMessage('system', 'Usage: .msg.ttl <chat|cmd|whisper> <seconds>');
        return true;
      }

      const kind = String(args[0] || '').trim().toLowerCase();
      const ttlRaw = String(args[1] || '').trim();
      if (!/^\d+$/.test(ttlRaw)) {
        appendMessage('system', 'Usage: .msg.ttl <chat|cmd|whisper> <seconds>');
        return true;
      }

      const ok = setMessageTtl(kind, Number(ttlRaw));
      if (!ok) {
        appendMessage('system', 'Usage: .msg.ttl <chat|cmd|whisper> <seconds>');
      }
      return true;
    }

    if (dotCommand === 'ttl') {
      if (args.length === 0) {
        const current = getTemporaryMessageTtlOverride();
        appendMessage('system', current === null ? '.ttl unset' : `.ttl ${current}`);
        return true;
      }

      if (args.length !== 1) {
        appendMessage('system', 'Usage: .ttl [seconds] | .ttl.unset');
        return true;
      }

      const ttlRaw = String(args[0] || '').trim();
      if (!/^\d+$/.test(ttlRaw)) {
        appendMessage('system', 'Usage: .ttl [seconds] | .ttl.unset');
        return true;
      }

      const ok = setTemporaryMessageTtlOverride(Number(ttlRaw));
      if (!ok) {
        appendMessage('system', 'Usage: .ttl [seconds] | .ttl.unset');
      }
      return true;
    }

    if (dotCommand === 'ttl.unset') {
      clearTemporaryMessageTtlOverride();
      return true;
    }

    if (dotCommand === 'batch.start') {
      if (args.length !== 1 || !/^\d+$/.test(String(args[0] || '').trim())) {
        appendMessage('system', 'Usage: .batch.start <seconds>');
        return true;
      }
      const ok = setBatchTimeoutSeconds(Number(args[0]));
      if (!ok) {
        appendMessage('system', 'Usage: .batch.start <seconds>');
      }
      return true;
    }

    if (dotCommand === 'batch.retry') {
      if (args.length !== 1 || !/^\d+$/.test(String(args[0] || '').trim())) {
        appendMessage('system', 'Usage: .batch.retry <count>');
        return true;
      }
      const ok = setBatchRetryCount(Number(args[0]));
      if (!ok) {
        appendMessage('system', 'Usage: .batch.retry <count>');
      }
      return true;
    }

    if (dotCommand === 'batch') {
      if (args.length !== 0) {
        appendMessage('system', 'Usage: .batch');
        return true;
      }
      appendMessage('system', batchStatusLine());
      Promise.resolve(runBatchCommands()).catch((error) => {
        appendMessage('system', error instanceof Error ? error.message : String(error));
      });
      return true;
    }

    if (dotCommand === 'dialog.id') {
      if (args.length === 0) {
        appendMessage('system', `${state.dialogIdStyle || 'alias'}`);
        return true;
      }
      if (args.length !== 1) {
        appendMessage('system', 'Usage: .dialog.id [alias|fragment|did]');
        return true;
      }
      const mode = String(args[0] || '').trim().toLowerCase();
      if (mode !== 'alias' && mode !== 'fragment' && mode !== 'did') {
        appendMessage('system', 'Usage: .dialog.id [alias|fragment|did]');
        return true;
      }
      if (typeof setDialogIdStyle === 'function' && !setDialogIdStyle(mode)) {
        appendMessage('system', 'Could not update dialog ID style.');
        return true;
      }
      appendMessage('system', `Dialog DID style set to: ${mode}`);
      return true;
    }

    if (dotCommand === 'edit') {
      onDotEdit(tail);
      return true;
    }

    if (dotCommand === 'eval') {
      onDotEval(tail);
      return true;
    }

    if (dotCommand === 'inspect') {
      onDotInspect(tail);
      return true;
    }

    if (dotCommand === 'use') {
      const requirement = 'none';
      const useTail = String(tail || '').trim();
      const didMatch = useTail.match(/^(\S+)(?:\s+as\s+(@?[A-Za-z0-9_-]+))?$/i);
      if (!didMatch) {
        appendMessage('system', uiText('Usage: .use <object|did:ma:...#fragment> [as alias]', 'Bruk: .use <objekt|did:ma:...#fragment> [as alias]'));
        return true;
      }

      const rawTarget = String(didMatch[1] || '').trim();
      const requestedAliasRaw = String(didMatch[2] || '').trim();
      const requestedAlias = requestedAliasRaw
        ? (requestedAliasRaw.startsWith('@') ? requestedAliasRaw : `@${requestedAliasRaw}`)
        : '';
      Promise.resolve()
        .then(async () => {
          const objectDid = isMaDid(rawTarget)
            ? rawTarget
            : (rawTarget.toLowerCase().startsWith('@my.')
              ? await resolveCommandTargetDidOrToken(rawTarget)
              : await lookupDidInCurrentRoom(rawTarget));
          const fragment = objectDid.includes('#') ? objectDid.split('#')[1] : '';
          const autoAlias = fragment ? `@${fragment.replace(/[^A-Za-z0-9_-]/g, '').toLowerCase()}` : '@obj';
          const alias = requestedAlias || autoAlias;
          if (!/^@[A-Za-z0-9_-]+$/.test(alias)) {
            appendMessage('system', uiText('Usage: .use <object|did:ma:...#fragment> [as alias]', 'Bruk: .use <objekt|did:ma:...#fragment> [as alias]'));
            return;
          }
          await sendWorldCommandQuery(`@${objectDid} id`);
          cacheRoomDidLookup(rawTarget, objectDid);
          cacheRoomDidLookup(alias, objectDid);
          setActiveObjectTarget(alias, objectDid, requirement);
          appendMessage('system', `using ${alias} -> ${objectDid}`);
          refillCommandInputWithActiveTarget();
        })
        .catch((error) => {
          appendMessage('system', uiText(
            `Use failed: ${error instanceof Error ? error.message : String(error)}`,
            `Use feilet: ${error instanceof Error ? error.message : String(error)}`
          ));
        });
      return true;
    }

    if (dotCommand === 'unuse') {
      const alias = String(args[0] || '').trim();
      if (!alias || !alias.startsWith('@')) {
        appendMessage('system', uiText('Usage: .unuse @alias', 'Bruk: .unuse @alias'));
        return true;
      }
      dropCachedRoomDidLookup(alias);
      clearActiveObjectTarget(alias);
      appendMessage('system', uiText(`stopped using ${alias}`, `sluttet å bruke ${alias}`));
      refillCommandInputWithActiveTarget();
      return true;
    }

    if (dotCommand === 'refresh') {
      if (!state.currentHome) {
        appendSystemUi('Not connected to a world.', 'Ikke koblet til en world.');
        return true;
      }
      Promise.resolve()
        .then(() => pollDirectInbox())
        .then(() => pollCurrentHomeEvents())
        .then(() => appendSystemUi('Refreshed room state.', 'Oppdatert romtilstand.'))
        .catch((err) => {
          appendMessage('system', uiText(
            `Refresh failed: ${err instanceof Error ? err.message : String(err)}`,
            `Oppdatering feilet: ${err instanceof Error ? err.message : String(err)}`
          ));
        });
      return true;
    }

    if (dotCommand === 'ping') {
      const target = String(args[0] || '@world').trim().toLowerCase();
      if (target !== '@world' && target !== '@here' && target !== '@avatar') {
        appendMessage('system', 'Usage: .ping [@world|@here|@avatar]');
        return true;
      }

      const command = `${target} ping`;
      const started = Date.now();
      sendWorldCommandQuery(command)
        .then((message) => {
          const elapsed = Date.now() - started;
          const payload = String(message || '').trim() || '(empty response)';
          appendMessage('system', `.ping ${target} -> ${elapsed}ms | ${payload}`);
        })
        .catch((error) => {
          const elapsed = Date.now() - started;
          appendMessage('system', `.ping ${target} failed after ${elapsed}ms: ${error instanceof Error ? error.message : String(error)}`);
        });
      return true;
    }

    if (dotCommand === 'smoke') {
      if (args.length > 1) {
        appendMessage('system', 'Usage: .smoke [alias]');
        return true;
      }
      runSmokeTest(args[0]).catch((err) => {
        appendMessage('system', `Smoke failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      return true;
    }

    if (dotCommand === 'broadcast' || dotCommand === 'broadcast.enabled' || dotCommand === 'broadcast.send') {
      if (dotCommand === 'broadcast') {
        appendMessage('system', `.broadcast.enabled ${broadcastEnabled() ? 'on' : 'off'}`);
        return true;
      }
      if (dotCommand === 'broadcast.enabled') {
        if (args.length === 0) {
          appendMessage('system', `.broadcast.enabled ${broadcastEnabled() ? 'on' : 'off'}`);
          return true;
        }
        const val = String(args[0] || '').trim().toLowerCase();
        if (val === 'on' || val === '1' || val === 'true') {
          setBroadcastEnabled(true);
        } else if (val === 'off' || val === '0' || val === 'false') {
          setBroadcastEnabled(false);
        } else {
          appendMessage('system', 'Usage: .broadcast.enabled [on|off]');
        }
        return true;
      }
      if (dotCommand === 'broadcast.send') {
        const text = args.join(' ').trim();
        if (!text) {
          appendMessage('system', 'Usage: .broadcast.send <message>');
          return true;
        }
        broadcastSend(text).catch((err) => {
          appendMessage('system', `Broadcast send failed: ${err instanceof Error ? err.message : String(err)}`);
        });
        return true;
      }
    }

    appendMessage('system', uiText(
      `Unknown command: .${dotCommand}. Try .help.`,
      `Ukjent kommando: .${dotCommand}. Prøv .help.`
    ));
    return true;
  }

  return {
    parseDot,
    parseLocalCommand,
  };
}
