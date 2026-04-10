import { isMaDid } from './did.js';

export function createDotCommands({
  state,
  appendSystemUi,
  appendMessage,
  uiText,
  humanizeIdentifier,
  isPrintableAliasLabel,
  saveAliasBook,
  resolveCurrentPositionTarget,
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
  didRoot,
  resolveTargetDidRoot,
  saveBlockedDidRoots,
  onDotEdit,
  onDotEval,
  onDotInspect,
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
  sendWhisperToDid,
  runSmokeTest,
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
      appendSystemUi('Dot commands:', 'Punktkommandoer:');
      appendSystemUi('  .help                      - this message', '  .help                      - denne meldingen');
      appendSystemUi('  .identity                  - show local pre-publish DID document as raw JSON', '  .identity                  - vis lokalt DID-dokument (før publisering) som rå JSON');
      appendSystemUi('  .aliases add <name> <target> - add/update alias (no spaces in target)', '  .aliases add <navn> <mål> - legg til/oppdater alias (ingen mellomrom i mål)');
      appendSystemUi('    note: @here/@me/@world/@avatar are updated automatically', '    merk: @here/@me/@world/@avatar oppdateres automatisk');
      appendSystemUi('  .set home [did:ma:...#room]- set home target (or current position)', '  .set home [did:ma:...#room]- sett home-mål (eller nåværende posisjon)');
      appendSystemUi('  .aliases del <name>        - remove alias', '  .aliases del <navn>        - fjern alias');
      appendSystemUi('  .aliases                   - list aliases', '  .aliases                   - list alias');
      appendSystemUi('  .aliases.<name>            - show address for one alias', '  .aliases.<navn>            - vis adresse for ett alias');
      appendSystemUi('  .aliases.rewrite [on|off]  - rewrite aliases to DID before parsing', '  .aliases.rewrite [on|off]  - skriv alias om til DID før parsing');
      appendSystemUi('  .inspect @here|@me|@exit <name>|<object>- inspect room/me/exit/object and discover DID/CIDs', '  .inspect @here|@me|@exit <navn>|<objekt>- inspiser rom/meg/utgang/objekt og finn DID/CID');
      appendSystemUi('  .use <object|did> [as alias] - set local default target', '  .use <objekt|did> [as alias] - sett lokal standardtarget');
      appendSystemUi('  .unuse @alias              - clear local default target', '  .unuse @alias              - fjern lokal standardtarget');
      appendSystemUi('  .edit [@here|@me|@exit <name>|did:ma:<world>#<room>] - open editor', '  .edit [@here|@me|@exit <navn>|did:ma:<world>#<room>] - åpne editor');
      appendSystemUi('  .eval <cid|alias>          - run script from IPFS CID or alias', '  .eval <cid|alias>          - kjør script fra IPFS CID eller alias');
      appendSystemUi('  .refresh                   - fetch latest room state and events now', '  .refresh                   - hent siste romtilstand og hendelser nå');
      appendSystemUi('  .mail [list|pick|reply|delete|clear] - inspect mailbox queue', '  .mail [list|pick|reply|delete|clear] - inspiser mailbox-kø');
      appendSystemUi('  .invite <did|alias> [note] - allow DID and send invite notice', '  .invite <did|alias> [note] - tillat DID og send invitasjonsmelding');
      appendSystemUi('  .smoke [alias]             - run connectivity smoke test', '  .smoke [alias]             - kjør enkel tilkoblingstest');
      appendSystemUi('  .block <did|alias|handle>  - block sender DID root', '  .block <did|alias|handle>  - blokker avsenders DID-root');
      appendSystemUi('  .unblock <did|alias|handle>- remove sender from block list', '  .unblock <did|alias|handle>- fjern avsender fra blokkeringslisten');
      appendSystemUi('  .blocks                    - list blocked sender DID roots', '  .blocks                    - list blokkerte avsender-DID-rooter');
      appendSystemUi('  .debug [on|off]            - toggle debug logs', '  .debug [on|off]            - slå debuglogger av/på');
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

    if (dotCommand === 'identity') {
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

    if (dotCommand === 'aliases') {
      if (args.length === 0) {
        const entries = Object.entries(state.aliasBook);
        if (entries.length === 0) {
          appendMessage('system', 'No aliases saved yet.');
          return true;
        }
        for (const [name, address] of entries) {
          appendMessage('system', `.aliases.${name} ${address}`);
        }
        return true;
      }

      const sub = String(args[0] || '').trim().toLowerCase();
      if (sub !== 'add' && sub !== 'del') {
        appendMessage('system', 'Usage: .aliases | .aliases add <name> <target> | .aliases del <name> | .aliases.<name> | .aliases.rewrite [on|off]');
        return true;
      }

      if (sub === 'del') {
        if (args.length !== 2) {
          appendMessage('system', 'Usage: .aliases del <name>');
          return true;
        }
        const inputName = normalizeAliasNameToken(args[1]);
        const name = resolveAliasBookKey(inputName);
        if (DYNAMIC_SPECIAL_ALIASES.has(name)) {
          appendMessage('system', `Alias ${name} is managed automatically.`);
          return true;
        }
        if (!name || !Object.prototype.hasOwnProperty.call(state.aliasBook, name)) {
          appendMessage('system', `Alias not found: ${inputName || String(args[1] || '').trim()}`);
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

      if (args.length !== 3) {
        appendMessage('system', 'Usage: .aliases add <name> <target>');
        return true;
      }

      let name = normalizeAliasNameToken(args[1]);
      const address = normalizeAliasTargetToken(args[2]);

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

    if (dotCommand === 'aliases.rewrite') {
      if (args.length === 0) {
        appendMessage('system', `${state.aliasRewriteEnabled ? 'on' : 'off'}`);
        return true;
      }
      if (args.length !== 1) {
        appendMessage('system', 'Usage: .aliases.rewrite [on|off]');
        return true;
      }
      const mode = String(args[0] || '').trim().toLowerCase();
      if (mode !== 'on' && mode !== 'off') {
        appendMessage('system', 'Usage: .aliases.rewrite [on|off]');
        return true;
      }
      if (typeof setAliasRewriteEnabled === 'function') {
        setAliasRewriteEnabled(mode === 'on');
      }
      appendMessage('system', `Alias rewrite is now ${mode}.`);
      return true;
    }

    if (dotCommand.startsWith('aliases.')) {
      const inputAliasName = normalizeAliasNameToken(verbToken.slice('aliases.'.length).trim());
      const aliasName = resolveAliasBookKey(inputAliasName);
      if (!aliasName) {
        appendMessage('system', 'Usage: .aliases.<name>');
        return true;
      }
      if (!Object.prototype.hasOwnProperty.call(state.aliasBook, aliasName)) {
        appendMessage('system', `Alias not found: ${inputAliasName}`);
        return true;
      }
      appendMessage('system', `.aliases.${aliasName} ${state.aliasBook[aliasName]}`);
      return true;
    }

    if (dotCommand === 'set') {
      const key = String(args[0] || '').toLowerCase();
      if (key !== 'home') {
        appendMessage('system', 'Usage: .set home [did:ma:<world>#<room>]');
        return true;
      }

      let target = args.slice(1).join(' ').trim();
      if (!target) {
        target = resolveCurrentPositionTarget();
        if (!target) {
          appendMessage('system', 'Could not resolve current position as did:ma target. Use .set home did:ma:<world>#<room>.');
          return true;
        }
      }

      if (!isMaDid(target)) {
        appendMessage('system', 'Usage: .set home [did:ma:<world>#<room>]');
        return true;
      }

      state.aliasBook.home = target;
      saveAliasBook();
      appendMessage('system', `Home set: home => ${target}`);
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

    if (dotCommand === 'blocks') {
      const blocked = Array.from(state.blockedDidRoots || []).sort();
      if (!blocked.length) {
        appendMessage('system', 'No blocked senders.');
        return true;
      }
      appendMessage('system', `Blocked senders (${blocked.length}):`);
      for (const did of blocked) {
        appendMessage('system', `  ${did}`);
      }
      return true;
    }

    if (dotCommand === 'block') {
      if (args.length !== 1) {
        appendMessage('system', 'Usage: .block <did|alias|handle>');
        return true;
      }
      try {
        const root = resolveTargetDidRoot(args[0]);
        if (state.identity && didRoot(state.identity.did) === root) {
          appendMessage('system', 'Refusing to block your own DID root.');
          return true;
        }
        const before = state.blockedDidRoots.size;
        state.blockedDidRoots.add(root);
        if (state.blockedDidRoots.size !== before) {
          saveBlockedDidRoots();
        }
        appendMessage('system', `Blocked sender: ${root}`);
      } catch (error) {
        appendMessage('system', error instanceof Error ? error.message : String(error));
      }
      return true;
    }

    if (dotCommand === 'unblock') {
      if (args.length !== 1) {
        appendMessage('system', 'Usage: .unblock <did|alias|handle>');
        return true;
      }
      try {
        const root = resolveTargetDidRoot(args[0]);
        const removed = state.blockedDidRoots.delete(root);
        if (removed) {
          saveBlockedDidRoots();
          appendMessage('system', `Unblocked sender: ${root}`);
        } else {
          appendMessage('system', `Sender not blocked: ${root}`);
        }
      } catch (error) {
        appendMessage('system', error instanceof Error ? error.message : String(error));
      }
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
            : await lookupDidInCurrentRoom(rawTarget);
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

    if (dotCommand === 'mail' || dotCommand === 'mailbox') {
      const sub = String(args[0] || 'list').toLowerCase();
      const list = Array.isArray(state.mailbox) ? state.mailbox : [];

      if (sub === 'list') {
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
        appendSystemUi('Use .mail pick <id>, .mail reply <id> <text>, or .mail delete <id>.', 'Bruk .mail pick <id>, .mail reply <id> <tekst>, eller .mail delete <id>.');
        return true;
      }

      if (sub === 'pick' || sub === 'show') {
        const idRaw = String(args[1] || '').trim();
        const id = Number(idRaw);
        if (!Number.isFinite(id) || id <= 0) {
          appendMessage('system', 'Usage: .mail pick <id>');
          return true;
        }
        const entry = list.find((item) => Number(item.id) === id);
        if (!entry) {
          appendMessage('system', `Mailbox entry not found: ${id}`);
          return true;
        }
        appendMessage('system', `.mail pick ${id}`);
        appendMessage('system', `  from: ${humanizeIdentifier(entry.from_did || '(unknown)')}`);
        appendMessage('system', `  endpoint: ${humanizeIdentifier(entry.from_endpoint || '(unknown)')}`);
        appendMessage('system', `  type: ${entry.content_type || '(unknown)'}`);
        appendMessage('system', `  text: ${entry.content_text || '(binary)'}`);
        appendMessage('system', `  cbor: ${entry.message_cbor_b64 || '(missing)'}`);
        return true;
      }

      if (sub === 'delete' || sub === 'del' || sub === 'rm') {
        const idRaw = String(args[1] || '').trim();
        const id = Number(idRaw);
        if (!Number.isFinite(id) || id <= 0) {
          appendMessage('system', 'Usage: .mail delete <id>');
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

      if (sub === 'reply') {
        const idRaw = String(args[1] || '').trim();
        const id = Number(idRaw);
        const replyText = args.slice(2).join(' ').trim();
        if (!Number.isFinite(id) || id <= 0 || !replyText) {
          appendMessage('system', 'Usage: .mail reply <id> <text>');
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
        sendWhisperToDid(targetDid, replyText)
          .then(() => {
            appendMessage('system', `Reply sent to ${humanizeIdentifier(targetDid)} from mailbox #${id}.`);
          })
          .catch((error) => {
            appendMessage('system', `Reply failed: ${error instanceof Error ? error.message : String(error)}`);
          });
        return true;
      }

      if (sub === 'clear') {
        const cleared = list.length;
        state.mailbox = [];
        appendMessage('system', `Mailbox cleared (${cleared} entries).`);
        return true;
      }

      appendMessage('system', 'Usage: .mail [list|pick <id>|reply <id> <text>|delete <id>|clear]');
      return true;
    }

    if (dotCommand === 'invite') {
      if (args.length < 1) {
        appendMessage('system', 'Usage: .invite <did|alias|handle> [note]');
        return true;
      }
      let targetRoot = '';
      try {
        targetRoot = resolveTargetDidRoot(args[0]);
      } catch (error) {
        appendMessage('system', error instanceof Error ? error.message : String(error));
        return true;
      }
      const note = args.slice(1).join(' ').trim();
      const inviteText = note || 'Your knock request was accepted. You may enter now.';
      const command = `@world invite ${targetRoot} ${inviteText}`;
      sendWorldCommandQuery(command)
        .then((message) => {
          appendMessage('system', message || `Invited ${targetRoot}.`);
          return sendWhisperToDid(targetRoot, `invite accepted: ${inviteText}`);
        })
        .then(() => {
          appendMessage('system', `Invite notice sent to ${humanizeIdentifier(targetRoot)}.`);
        })
        .catch((error) => {
          appendMessage('system', `Invite failed: ${error instanceof Error ? error.message : String(error)}`);
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

    appendMessage('system', uiText(
      `Unknown command: .${dotCommand}. Try .help.`,
      `Ukjent kommando: .${dotCommand}. Prøv .help.`
    ));
    return true;
  }

  return {
    parseDot,
  };
}
