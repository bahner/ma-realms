import { isMaDid } from './did.js';

export function normalizeIrohAddress(address) {
  const value = String(address || '').trim();
  if (!value) return '';
  if (value.startsWith('/iroh-ma/')) {
    return value.slice('/iroh-ma/'.length).split('/')[0];
  }
  if (value.startsWith('/ma-iroh/')) {
    return value.slice('/ma-iroh/'.length).split('/')[0];
  }
  if (value.startsWith('/iroh+ma/')) {
    return value.slice('/iroh+ma/'.length).split('/')[0];
  }
  if (value.startsWith('/iroh/')) {
    return value.slice('/iroh/'.length).split('/')[0];
  }
  return value;
}

export function isLikelyIrohAddress(address) {
  return /^[a-f0-9]{64}$/i.test(normalizeIrohAddress(address));
}

export function extractEndpointFromTransportEntry(entry) {
  if (!entry) return '';
  if (typeof entry === 'string') {
    const endpoint = normalizeIrohAddress(entry);
    return isLikelyIrohAddress(endpoint) ? endpoint : '';
  }
  if (typeof entry !== 'object') {
    return '';
  }

  const candidates = [
    entry.endpoint_id,
    entry.endpointId,
    entry.iroh,
    entry.address,
    entry.currentInbox,
    entry.current_inbox,
    entry.presence_hint,
    entry.presenceHint
  ];
  for (const candidate of candidates) {
    const endpoint = normalizeIrohAddress(candidate || '');
    if (isLikelyIrohAddress(endpoint)) {
      return endpoint;
    }
  }

  return '';
}

export function extractWorldEndpointFromDidDoc(document) {
  if (!document || typeof document !== 'object') {
    return '';
  }

  const ma = document.ma && typeof document.ma === 'object' ? document.ma : null;

  const transports = ma?.transports;
  if (Array.isArray(transports)) {
    for (const entry of transports) {
      const endpoint = extractEndpointFromTransportEntry(entry);
      if (endpoint) {
        return endpoint;
      }
    }
  } else {
    const endpoint = extractEndpointFromTransportEntry(transports);
    if (endpoint) {
      return endpoint;
    }
  }

  const inbox = normalizeIrohAddress(ma?.currentInbox || ma?.current_inbox || '');
  if (isLikelyIrohAddress(inbox)) {
    return inbox;
  }

  const fallback = normalizeIrohAddress(ma?.presenceHint || '');
  if (isLikelyIrohAddress(fallback)) {
    return fallback;
  }

  return '';
}

export function parseEnterDirective(message) {
  const text = String(message || '');
  const match = text.match(/(?:^|\s)go\s+(did:ma:[^\s]+)/i);
  if (!match) {
    return null;
  }
  const rawDid = String(match[1] || '').replace(/[),.;]+$/, '');
  if (!isMaDid(rawDid)) {
    return null;
  }
  return rawDid;
}

export function resolveAliasTargetToken(token, aliases, depth = 0) {
  const raw = String(token || '').trim();
  if (!raw) {
    return '';
  }
  const normalized = raw.toLowerCase();
  if (depth > 6) {
    return '';
  }
  const mapped = aliases.get(normalized)
    || (normalized.startsWith('@') ? aliases.get(normalized.slice(1)) : aliases.get(`@${normalized}`));
  if (!mapped) {
    return '';
  }
  if (isMaDid(mapped)) {
    return mapped;
  }
  const mappedToken = String(mapped || '').trim();
  if (!mappedToken.startsWith('@')) {
    return '';
  }
  const mappedKey = mappedToken.toLowerCase();
  // Treat self-referential mappings as terminal targets (e.g. panteia => @panteia).
  if (mappedKey === normalized || mappedKey === `@${normalized}` || `@${mappedKey}` === normalized) {
    return mappedToken;
  }
  if (aliases.has(mappedKey)
    || (mappedKey.startsWith('@') && aliases.has(mappedKey.slice(1)))
    || (!mappedKey.startsWith('@') && aliases.has(`@${mappedKey}`))) {
    return resolveAliasTargetToken(mappedKey, aliases, depth + 1);
  }
  return mappedToken;
}

export function createWorldTitleFlow({ state, properName, documentRef = document }) {
  function currentWorldName() {
    const alias = String(state.currentHome?.alias || '').trim();
    if (alias) return alias;
    const endpoint = String(state.currentHome?.endpointId || '').trim();
    if (endpoint) return endpoint.slice(0, 10);
    return '';
  }

  function updateDocumentTitle() {
    const world = currentWorldName();
    const activeTarget = String(state.activeObjectTargetAlias || '').trim();
    const context = world ? `${world}${activeTarget}` : activeTarget;
    documentRef.title = context ? `${properName} - ${context}` : properName;
  }

  function updateLocationContext() {
    updateDocumentTitle();
  }

  return {
    updateDocumentTitle,
    updateLocationContext,
  };
}

export function createWorldFlow({
  state,
  appendMessage,
  sendWorldCommandQuery,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  refillCommandInputWithActiveTarget,
  logger,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
  buildCurrentHomeResumeTarget,
  enterHome,
  requestReentry,
}) {
  function isNotRegisteredInRoomMessage(message) {
    const text = String(message || '').toLowerCase();
    return (
      text.includes('not registered in room')
      || (text.includes('unknown avatar @') && text.includes(' in room '))
    );
  }

  function isActiveTargetGoneMessage(message) {
    const text = String(message || '').toLowerCase();
    return (
      text.includes('unknown actor or object')
      || text.includes('object alias') && text.includes('stale')
      || text.includes('shortcut') && text.includes('not found')
      || text.includes('object') && text.includes('not found')
    );
  }

  function reportActiveTargetVanished(alias) {
    const normalizedAlias = String(alias || '').trim().replace(/^@+/, '') || 'dings';
    appendMessage('system', `${normalizedAlias} vanished in a puff of logic.`);
  }

  async function restoreActiveObjectTargetAfterReentry(alias, did) {
    const normalizedAlias = String(alias || '').trim().replace(/^@+/, '');
    const normalizedDid = String(did || '').trim();
    if (!normalizedAlias || !isMaDid(normalizedDid)) {
      return;
    }

    try {
      await sendWorldCommandQuery(`@${normalizedDid} id`);
      cacheRoomDidLookup(normalizedAlias, normalizedDid);
      setActiveObjectTarget(normalizedAlias, normalizedDid);
      refillCommandInputWithActiveTarget();
    } catch (error) {
      logger.log(
        'reconnect',
        `could not restore active target ${normalizedAlias}: ${error instanceof Error ? error.message : String(error)}`
      );
      dropCachedRoomDidLookup(normalizedAlias);
      clearActiveObjectTarget(normalizedAlias);
      refillCommandInputWithActiveTarget();
      reportActiveTargetVanished(normalizedAlias);
    }
  }

  async function performTransparentReentry(reason) {
    const activeAlias = String(state.activeObjectTargetAlias || '').trim();
    const activeDid = String(state.activeObjectTargetDid || '').trim();

    await requestReentry(reason);
    await restoreActiveObjectTargetAfterReentry(activeAlias, activeDid);
  }

  return {
    isNotRegisteredInRoomMessage,
    isActiveTargetGoneMessage,
    reportActiveTargetVanished,
    performTransparentReentry,
  };
}

export function createWorldDispatchFlow({
  state,
  appendMessage,
  enterHome,
  resolveWorldEndpointForDid,
  isLikelyIrohAddress,
  normalizeIrohAddress,
  parseDot,
  resolveCommandTargetDidOrToken,
  logger,
  sendWorldChat,
  sendWorldChatWithTtl,
  sendWorldCmd,
  sendWorldCmdWithTtl,
  getMessageTtl,
  pollCurrentHomeEvents,
  appendAmbientProseAfterSpeech,
  renderLocalBroadcastMessage,
  applyWorldResponse,
  tryHandleDidTargetMetaPoll,
  sendWhisperToDid,
  isNotRegisteredInRoomMessage,
  performTransparentReentry,
}) {
  function resolveTtlSeconds(kind) {
    const temporary = Number(state.temporaryMessageTtlOverride);
    if (Number.isFinite(temporary) && temporary >= 0) {
      return Math.floor(temporary);
    }
    const configured = Number(getMessageTtl(kind));
    if (Number.isFinite(configured) && configured >= 0) {
      return Math.floor(configured);
    }
    return 60;
  }

  function actorDidFromState() {
    const did = String(state.identity?.did || '').trim();
    if (!isMaDid(did) || !did.includes('#')) {
      return '';
    }
    return did;
  }

  function roomHintFromDidTarget(targetDid) {
    const raw = String(targetDid || '').trim();
    const hashIdx = raw.indexOf('#');
    if (hashIdx === -1 || hashIdx >= raw.length - 1) {
      return 'lobby';
    }
    const fragment = String(raw.slice(hashIdx + 1) || '').trim();
    if (!fragment || fragment.includes('.')) {
      return 'lobby';
    }
    const safe = fragment.toLowerCase();
    if (safe === 'world' || safe === 'room' || safe === 'here' || safe === 'avatar' || safe === 'me' || safe === 'self') {
      return 'lobby';
    }
    return 'lobby';
  }

  function normalizeDidTargetPath(baseDid, pathRaw) {
    const did = String(baseDid || '').trim();
    const path = String(pathRaw || '').trim();
    if (!isMaDid(did)) {
      return { targetDid: did, targetPath: path, routeWorld: false };
    }
    if (!path || did.includes('#')) {
      return { targetDid: did, targetPath: path, routeWorld: false };
    }

    const segments = path
      .split('.')
      .map((segment) => String(segment || '').trim())
      .filter(Boolean);
    if (segments.length === 1) {
      return {
        targetDid: did,
        targetPath: segments[0],
        routeWorld: true,
      };
    }
    if (segments.length < 2) {
      return { targetDid: did, targetPath: path, routeWorld: false };
    }

    const fragment = segments[0];
    const targetPath = segments.slice(1).join('.');
    return {
      targetDid: `${did}#${fragment}`,
      targetPath,
      routeWorld: false,
    };
  }

  async function sendStatelessDidCommand(inputText) {
    const trimmed = String(inputText || '').trim();
    if (!trimmed.startsWith('@')) {
      return false;
    }

    const spaceIdx = trimmed.indexOf(' ');
    const rawTarget = (spaceIdx === -1
      ? trimmed.slice(1)
      : trimmed.slice(1, spaceIdx)).trim();
    const remainder = (spaceIdx === -1
      ? ''
      : trimmed.slice(spaceIdx + 1)).trim();

    if (!rawTarget) {
      return false;
    }

    const dotIdx = rawTarget.indexOf('.');
    const baseRaw = String(dotIdx === -1 ? rawTarget : rawTarget.slice(0, dotIdx)).trim();
    const pathRaw = String(dotIdx === -1 ? '' : rawTarget.slice(dotIdx + 1)).trim();
    if (!baseRaw) {
      return false;
    }

    const resolvedBase = isMaDid(baseRaw)
      ? baseRaw
      : await resolveCommandTargetDidOrToken(baseRaw);
    const normalizedDid = String(resolvedBase || '').trim().replace(/^@+/, '');
    if (!isMaDid(normalizedDid)) {
      return false;
    }

    const { targetDid, targetPath, routeWorld } = normalizeDidTargetPath(normalizedDid, pathRaw);

    const ipnsKey = String(targetDid.split('#')[0] || '').trim();
    if (!ipnsKey) {
      return false;
    }

    const endpointId = await resolveWorldEndpointForDid(ipnsKey);
    if (!isLikelyIrohAddress(endpointId)) {
      throw new Error(`DID ${targetDid} did not resolve to a valid iroh endpoint.`);
    }

    const normalizedTarget = routeWorld
      ? `world.${targetPath}`
      : (targetPath ? `${targetDid}.${targetPath}` : targetDid);
    const normalizedInput = remainder
      ? `@${normalizedTarget} ${remainder}`
      : `@${normalizedTarget}`;
    const roomHint = roomHintFromDidTarget(targetDid);
    const sender = actorDidFromState() || activeActorName();

    const sendStart = Date.now();
    logger.log('send.command.stateless', `room=${roomHint} sender=${sender} msg_len=${normalizedInput.length}`);

    const result = JSON.parse(
      await sendWorldCmdWithTtl(
        endpointId,
        state.passphrase,
        state.encryptedBundle,
        sender,
        roomHint,
        normalizedInput,
        BigInt(resolveTtlSeconds('cmd'))
      )
    );

    const elapsed = Date.now() - sendStart;
    logger.log('send.command.stateless', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

    if (!result.ok) {
      throw new Error(result.message || 'send failed');
    }

    appendMessage('world', String(result.message || '(no response)'));
    return true;
  }

  function resolveWorldConnectTarget(rawTarget) {
    const target = String(rawTarget || '').trim();
    if (!target) {
      return '';
    }
    if (isMaDid(target)) {
      return target;
    }

    const endpoint = normalizeIrohAddress(target);
    if (isLikelyIrohAddress(endpoint) || target.startsWith('/ma-iroh/')) {
      return target;
    }

    const plain = target.startsWith('@') ? target.slice(1) : target;
    const keys = [target, plain, `@${plain}`];
    for (const key of keys) {
      const resolved = String(state.aliasBook?.[key] || '').trim();
      if (!resolved) {
        continue;
      }
      if (isMaDid(resolved)) {
        return resolved;
      }
      const resolvedEndpoint = normalizeIrohAddress(resolved);
      if (isLikelyIrohAddress(resolvedEndpoint) || resolved.startsWith('/ma-iroh/')) {
        return resolved;
      }
    }

    return '';
  }

  function actorFragmentFromDid(did) {
    const value = String(did || '').trim();
    const idx = value.indexOf('#');
    if (idx === -1 || idx >= value.length - 1) {
      return '';
    }
    return value.slice(idx + 1).trim();
  }

  function normalizeActorFragmentCandidate(value) {
    const raw = String(value || '').trim().replace(/^@+/, '');
    if (!raw) {
      return '';
    }
    if (!/^[A-Za-z0-9_-]+$/.test(raw)) {
      return '';
    }
    return raw;
  }

  function activeActorName() {
    const fragment = normalizeActorFragmentCandidate(actorFragmentFromDid(state.identity?.did));
    if (fragment) {
      return fragment;
    }

    const aliasFragment = normalizeActorFragmentCandidate(state.aliasName);
    if (aliasFragment) {
      return aliasFragment;
    }

    return 'actor';
  }

  function activeAvatarDid() {
    const aliasAvatar = String(state.aliasBook?.['@avatar'] || state.aliasBook?.avatar || '').trim();
    if (isMaDid(aliasAvatar) && aliasAvatar.includes('#')) {
      return aliasAvatar;
    }

    const worldRoot = String(
      state.currentHome?.roomDid
      || state.aliasBook?.['@world']
      || state.aliasBook?.world
      || ''
    ).trim().split('#')[0];
    const handle = String(state.currentHome?.handle || '').trim().replace(/^@+/, '');
    if (isMaDid(worldRoot) && handle && !/\s/u.test(handle)) {
      return `${worldRoot}#${handle}`;
    }
    return '';
  }

  function aliasTargetMap() {
    const result = new Map();
    for (const [alias, value] of Object.entries(state.aliasBook || {})) {
      const rawKey = String(alias || '').trim();
      if (!rawKey) {
        continue;
      }
      const target = String(value || '').trim();
      if (!target || /\s/u.test(target)) {
        continue;
      }
      const lowered = rawKey.toLowerCase();
      result.set(lowered, target);
      if (lowered.startsWith('@')) {
        result.set(lowered.slice(1), target);
      } else {
        result.set(`@${lowered}`, target);
      }
    }
    return result;
  }

  function resolveAliasTarget(token, aliases) {
    return resolveAliasTargetToken(token, aliases);
  }

  function rewriteAliasesToDid(input) {
    const text = String(input || '');
    if (!text.trim() || text.trim().startsWith("'")) {
      return text;
    }

    const aliases = aliasTargetMap();
    if (!aliases.size) {
      return text;
    }

    let out = text;
    let ownerOverrideRewriteInfo = null;

    out = out.replace(/^(\s*)@@here\.owner\s+(\S+)(.*)$/i, (all, prefix, value, suffix) => {
      const room = String(state.currentHome?.room || '').trim();
      if (!room) {
        return all;
      }

      const rawValue = String(value || '').trim();
      if (!rawValue) {
        return all;
      }

      let resolvedValue = rawValue;
      // Keep value syntax strict: resolve bare alias tokens only.
      if (!rawValue.startsWith('@')) {
        const resolved = resolveAliasTarget(rawValue, aliases);
        const normalized = String(resolved || '').trim();
        if (normalized) {
          resolvedValue = normalized;
        }
      }

      ownerOverrideRewriteInfo = {
        room,
        resolvedValue,
      };

      return `${prefix}@world.room ${room} owner ${resolvedValue}${String(suffix || '')}`;
    });

    if (ownerOverrideRewriteInfo) {
      appendMessage(
        'system',
        `admin override: @@here.owner -> @world.room ${ownerOverrideRewriteInfo.room} owner ${ownerOverrideRewriteInfo.resolvedValue}`
      );
    }

    out = out.replace(/^(\s*go\s+)(\S+)(.*)$/i, (all, prefix, target, suffix) => {
      const rawTarget = String(target || '').trim();
      if (!rawTarget) {
        return all;
      }
      const resolved = resolveAliasTarget(rawTarget, aliases);
      if (!resolved) {
        return all;
      }
      if (isMaDid(resolved)) {
        return `${prefix}${resolved}${suffix}`;
      }
      const normalizedEndpoint = normalizeIrohAddress(resolved);
      if (isLikelyIrohAddress(normalizedEndpoint)) {
        return `${prefix}${normalizedEndpoint}${suffix}`;
      }
      if (resolved.startsWith('/ma-iroh/')) {
        return `${prefix}${resolved}${suffix}`;
      }
      return all;
    });

    out = out.replace(/^(\s*@(?:here|world)\.owner\s+)(\S+)(.*)$/i, (all, prefix, value, suffix) => {
      const rawValue = String(value || '').trim();
      if (!rawValue) {
        return all;
      }

      // Keep value syntax strict: resolve bare alias tokens only.
      if (rawValue.startsWith('@')) {
        return all;
      }

      const resolved = resolveAliasTarget(rawValue, aliases);
      const normalized = String(resolved || '').trim();
      if (!normalized) {
        return all;
      }
      return `${prefix}${normalized}${String(suffix || '')}`;
    });

    out = out.replace(/^(\s*@\S+)(\s+.+)$/u, (all, head, tail) => {
      const parts = String(tail || '').trim().split(/\s+/u);
      const rewritten = parts.map((token) => {
        const raw = String(token || '').trim();
        if (!raw) {
          return raw;
        }
        if (raw.includes('"') || raw.includes("'")) {
          return raw;
        }

        const direct = resolveAliasTarget(raw, aliases);
        const stripped = resolveAliasTarget(raw.replace(/^@+/, ''), aliases);
        const resolved = String(direct || stripped || '').trim();
        if (!resolved || !isMaDid(resolved)) {
          return raw;
        }
        return resolved;
      });

      return `${head} ${rewritten.join(' ')}`;
    });

    return out;
  }

  async function normalizeOutgoingAtTarget(commandText) {
    const trimmed = String(commandText || '').trim();
    if (!trimmed.startsWith('@')) {
      return trimmed;
    }

    const spaceIdx = trimmed.indexOf(' ');
    const rawTarget = (spaceIdx === -1
      ? trimmed.substring(1)
      : trimmed.substring(1, spaceIdx)).trim();
    const remainder = (spaceIdx === -1
      ? ''
      : trimmed.substring(spaceIdx + 1)).trim();

    if (!rawTarget) {
      return trimmed;
    }

    const dotIdx = rawTarget.indexOf('.');
    const baseRaw = String(dotIdx === -1 ? rawTarget : rawTarget.slice(0, dotIdx)).trim();
    const pathRaw = String(dotIdx === -1 ? '' : rawTarget.slice(dotIdx + 1)).trim();
    if (!baseRaw) {
      return trimmed;
    }

    const resolvedBase = isMaDid(baseRaw)
      ? baseRaw
      : await resolveCommandTargetDidOrToken(baseRaw);
    const normalizedDid = String(resolvedBase || '').trim().replace(/^@+/, '');
    if (!isMaDid(normalizedDid)) {
      throw new Error('Target after @ must resolve to did:ma.');
    }

    const { targetDid, targetPath, routeWorld } = normalizeDidTargetPath(normalizedDid, pathRaw);
    const normalizedTarget = routeWorld
      ? `world.${targetPath}`
      : (targetPath ? `${targetDid}.${targetPath}` : targetDid);

    return remainder
      ? `@${normalizedTarget} ${remainder}`
      : `@${normalizedTarget}`;
  }

  function routeBareCommandToSelfDid(commandText) {
    const trimmed = String(commandText || '').trim();
    if (!trimmed || trimmed.startsWith('@') || trimmed.startsWith("'")) {
      return trimmed;
    }
    const selfDid = String(state.identity?.did || '').trim();
    if (!isMaDid(selfDid)) {
      return trimmed;
    }
    return `@${selfDid} ${trimmed}`;
  }

  async function sendWorldCommandQuery(commandText) {
    if (!state.identity || !state.currentHome) {
      throw new Error('Join a home before sending commands.');
    }

    const rewritten = rewriteAliasesToDid(commandText);
    const routed = routeBareCommandToSelfDid(rewritten);
    const normalizedCommand = routed.startsWith('@')
      ? await normalizeOutgoingAtTarget(routed)
      : routed;

    const result = JSON.parse(
      await sendWorldCmdWithTtl(
        state.currentHome.endpointId,
        state.passphrase,
        state.encryptedBundle,
        activeActorName(),
        state.currentHome.room,
        normalizedCommand,
        BigInt(resolveTtlSeconds('cmd'))
      )
    );

    if (!result.ok) {
      throw new Error(result.message || 'command failed');
    }
    if (result.broadcasted) {
      pollCurrentHomeEvents().catch((error) => {
        logger.log('poll.events', `non-fatal fallback poll after broadcast failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    }
    return String(result.message || '');
  }

  async function sendCurrentWorldMessage(text, options = {}) {
    const attempt = (options && typeof options === 'object' && Number.isFinite(options.attempt))
      ? Number(options.attempt)
      : 0;
    const bootstrapAttempt = (options && typeof options === 'object' && Number.isFinite(options.bootstrapAttempt))
      ? Number(options.bootstrapAttempt)
      : 0;

    try {
      const rewrittenInput = rewriteAliasesToDid(text);
      const trimmedText = rewrittenInput.trim();

      if (!state.identity) {
        appendMessage('system', 'Create or unlock an identity first.');
        return;
      }

      if (!state.currentHome) {
        const shortcutConnectTarget = String(trimmedText || '').trim();
        const atTarget = shortcutConnectTarget.startsWith('@')
          ? shortcutConnectTarget.slice(1).trim()
          : '';
        const looksLikeAtCommandTarget = atTarget
          && (atTarget.startsWith('did:ma:') || atTarget.includes('#') || atTarget.includes('.'));

        if (shortcutConnectTarget.startsWith('@')
          && !/\s/u.test(shortcutConnectTarget)
          && !looksLikeAtCommandTarget) {
          await enterHome(shortcutConnectTarget);
          return;
        }

        if (shortcutConnectTarget.startsWith('@') && atTarget.startsWith('did:ma:')) {
          const sent = await sendStatelessDidCommand(trimmedText);
          if (!sent) {
            appendMessage('system', 'Invalid DID target command. Use @did:ma:<world>[#object].<method> [args].');
          }
          return;
        }

        if (shortcutConnectTarget.startsWith('@') && looksLikeAtCommandTarget) {
          const sent = await sendStatelessDidCommand(trimmedText);
          if (sent) {
            return;
          }
        }

        // Stateless DID-object commands should not require an explicit `go ...` first.
        if (bootstrapAttempt < 1 && atTarget && atTarget.startsWith('did:ma:')) {
          const dotIdx = atTarget.indexOf('.');
          const didToken = String(dotIdx === -1 ? atTarget : atTarget.slice(0, dotIdx)).trim();
          if (didToken) {
            await enterHome(didToken, null, { silent: true });
            return await sendCurrentWorldMessage(text, {
              attempt,
              bootstrapAttempt: bootstrapAttempt + 1,
            });
          }
        }

        const bootstrapMatch = trimmedText.match(/^go\s+(.+)$/i);
        if (bootstrapMatch) {
          const target = String(bootstrapMatch[1] || '').trim();
          const connectTarget = resolveWorldConnectTarget(target);
          const looksLikeDid = isMaDid(target);
          const looksLikeAlias = Boolean(connectTarget);
          const looksLikeEndpoint = isLikelyIrohAddress(normalizeIrohAddress(target));

          if (looksLikeDid || looksLikeAlias || looksLikeEndpoint) {
            await enterHome(target);
            return;
          }
        }

        appendMessage('system', 'Not connected. Use go did:ma:<world>#<room> or go home (after .set home).');
        return;
      }

      if (/^use\s+/i.test(trimmedText) || /^unuse\s+/i.test(trimmedText)) {
        parseDot(`.${trimmedText}`);
        return;
      }

      const pickUpMatch = trimmedText.match(/^(?:pick\s+up|pickup)\s+(.+)$/i);
      if (pickUpMatch) {
        const targetToken = String(pickUpMatch[1] || '').trim();
        if (!targetToken) {
          appendMessage('system', 'Usage: pick up <object>');
          return;
        }
        const targetDid = await resolveCommandTargetDidOrToken(targetToken);
        const result = await sendWorldCommandQuery(`@${targetDid} take`);
        appendMessage('system', result || `Picked up ${targetToken}.`);
        return;
      }

      const fragmentWhisperMatch = trimmedText.match(/^#([^\s:]+)\s*:\s*(.+)$/);
      if (fragmentWhisperMatch) {
        const fragment = String(fragmentWhisperMatch[1] || '').trim();
        const payload = String(fragmentWhisperMatch[2] || '').trim();
        if (!fragment || !payload) {
          appendMessage('system', 'Usage: #fragment: message');
          return;
        }
        try {
          const targetDid = await resolveCommandTargetDidOrToken(`#${fragment}`);
          if (!isMaDid(String(targetDid))) {
            throw new Error(`Fragment target must resolve to did:ma, got: ${targetDid}`);
          }
          await sendWhisperToDid(targetDid, payload, { ttlSeconds: resolveTtlSeconds('whisper') });
          appendMessage('system', `Chat sent to ${targetDid}.`);
          return;
        } catch (err) {
          appendMessage('system', `Error sending chat to #${fragment}: ${err.message}`);
          return;
        }
      }

      if (trimmedText.startsWith("'")) {
        const payload = trimmedText.substring(1).trim();
        if (!payload) return;
        const sendStart = Date.now();
        logger.log('send.say', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${payload.length}`);

        const result = JSON.parse(
          await sendWorldCmdWithTtl(
            state.currentHome.endpointId,
            state.passphrase,
            state.encryptedBundle,
            activeActorName(),
            state.currentHome.room,
            `say ${payload}`,
            BigInt(resolveTtlSeconds('cmd'))
          )
        );
        const elapsed = Date.now() - sendStart;
        logger.log('send.say', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

        if (!result.ok) {
          throw new Error(result.message || 'say failed');
        }

        pollCurrentHomeEvents().catch((error) => {
          logger.log('poll.events', `non-fatal fallback poll after broadcast failed: ${error instanceof Error ? error.message : String(error)}`);
        });
        appendAmbientProseAfterSpeech().catch((err) => {
          logger.log('ambient.prose', `failed: ${err instanceof Error ? err.message : String(err)}`);
        });
        return;
      }

      if (trimmedText.startsWith(':')) {
        const payload = trimmedText.substring(1).trim();
        if (!payload) return;
        const sendStart = Date.now();
        logger.log('send.emote', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${payload.length}`);

        const result = JSON.parse(
          await sendWorldCmdWithTtl(
            state.currentHome.endpointId,
            state.passphrase,
            state.encryptedBundle,
            activeActorName(),
            state.currentHome.room,
            `emote ${payload}`,
            BigInt(resolveTtlSeconds('cmd'))
          )
        );
        const elapsed = Date.now() - sendStart;
        logger.log('send.emote', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

        if (!result.ok) {
          throw new Error(result.message || 'emote failed');
        }

        pollCurrentHomeEvents().catch((error) => {
          logger.log('poll.events', `non-fatal fallback poll after broadcast failed: ${error instanceof Error ? error.message : String(error)}`);
        });
        return;
      }

      if (trimmedText.startsWith('@')) {
        const trimmed = trimmedText.replace(/^@+/, '@');
        const whisperSep = trimmed.indexOf(" '");
        if (whisperSep > 1) {
          const target = trimmed.substring(1, whisperSep).trim();
          const payload = trimmed.substring(whisperSep + 2);
          const dotIdx = target.indexOf('.');
          const baseRaw = String(dotIdx === -1 ? target : target.slice(0, dotIdx)).trim();
          const pathRaw = String(dotIdx === -1 ? '' : target.slice(dotIdx + 1)).trim();

          try {
            const resolvedBase = isMaDid(baseRaw)
              ? baseRaw
              : await resolveCommandTargetDidOrToken(baseRaw);
            const normalizedDid = String(resolvedBase || '').trim().replace(/^@+/, '');
            const { targetDid, targetPath, routeWorld } = normalizeDidTargetPath(normalizedDid, pathRaw);
            const isDirectDidTarget = isMaDid(String(targetDid)) && String(targetDid).includes('#') && !routeWorld && !targetPath;

            if (isDirectDidTarget) {
              await sendWhisperToDid(targetDid, payload, { ttlSeconds: resolveTtlSeconds('whisper') });
              appendMessage('system', `Message sent to ${targetDid}.`);
              return;
            }
          } catch (err) {
            appendMessage('system', `Error sending message to ${target}: ${err.message}`);
            return;
          }
        }

        const spaceIdx = trimmed.indexOf(' ');
        const rawTarget = (spaceIdx === -1
          ? trimmed.substring(1)
          : trimmed.substring(1, spaceIdx)).trim();
        const remainder = (spaceIdx === -1
          ? ''
          : trimmed.substring(spaceIdx + 1)).trim();

        if (!rawTarget) {
          appendMessage('system', '?');
          return;
        }

        const dotIdx = rawTarget.indexOf('.');
        const baseRaw = String(dotIdx === -1 ? rawTarget : rawTarget.slice(0, dotIdx)).trim();
        const pathRaw = String(dotIdx === -1 ? '' : rawTarget.slice(dotIdx + 1)).trim();
        if (!baseRaw) {
          appendMessage('system', '?');
          return;
        }

        const resolvedBase = isMaDid(baseRaw)
          ? baseRaw
          : await resolveCommandTargetDidOrToken(baseRaw);
        const normalizedDid = String(resolvedBase || '').trim().replace(/^@+/, '');
        if (!isMaDid(normalizedDid)) {
          throw new Error('Target after @ must resolve to did:ma.');
        }

        const { targetDid, targetPath, routeWorld } = normalizeDidTargetPath(normalizedDid, pathRaw);
        const normalizedTarget = routeWorld
          ? `world.${targetPath}`
          : (targetPath ? `${targetDid}.${targetPath}` : targetDid);

        if (remainder && await tryHandleDidTargetMetaPoll(normalizedTarget, remainder)) {
          return;
        }

        const normalized = remainder
          ? `@${normalizedTarget} ${remainder}`
          : `@${normalizedTarget}`;

        const sendStart = Date.now();
        logger.log('send.command', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${trimmed.length}`);
        const result = JSON.parse(
          await sendWorldCmdWithTtl(
            state.currentHome.endpointId,
            state.passphrase,
            state.encryptedBundle,
            activeActorName(),
            state.currentHome.room,
            normalized,
            BigInt(resolveTtlSeconds('cmd'))
          )
        );
        const elapsed = Date.now() - sendStart;
        logger.log('send.command', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

        if (!result.ok) {
          throw new Error(result.message || 'send failed');
        }

        if (!result.broadcasted) {
          applyWorldResponse(result);
          return;
        }

        pollCurrentHomeEvents().catch((error) => {
          logger.log('poll.events', `non-fatal fallback poll after broadcast failed: ${error instanceof Error ? error.message : String(error)}`);
        });
        return;
      }

      const sendStart = Date.now();
      const routedText = routeBareCommandToSelfDid(trimmedText);
      logger.log('send.command', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${routedText.length}`);

      const result = JSON.parse(
        await sendWorldCmdWithTtl(
          state.currentHome.endpointId,
          state.passphrase,
          state.encryptedBundle,
          activeActorName(),
          state.currentHome.room,
          routedText,
          BigInt(resolveTtlSeconds('cmd'))
        )
      );
      const elapsed = Date.now() - sendStart;
      logger.log('send.command', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

      if (!result.ok) {
        throw new Error(result.message || 'send failed');
      }

      if (!result.broadcasted) {
        applyWorldResponse(result);
        return;
      }

      pollCurrentHomeEvents().catch((error) => {
        logger.log('poll.events', `non-fatal fallback poll after broadcast failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (attempt >= 1 || !isNotRegisteredInRoomMessage(message)) {
        throw error;
      }
      await performTransparentReentry(message);
      return await sendCurrentWorldMessage(text, { attempt: attempt + 1 });
    }
  }

  return {
    sendWorldCommandQuery,
    sendCurrentWorldMessage,
  };
}

export function createWorldResponseFlow({
  state,
  saveLastRoom,
  updateIdentityLine,
  updateRoomHeading,
  syncSpecialAliases,
  clearActiveObjectTarget,
  clearRoomPresence,
  trackRoomPresence,
  saveActiveHomeSnapshot,
  toSequenceNumber,
  primeDidLookupCacheFromRoomObjectDids,
  primeDidLookupCacheFromWorldMessage,
  appendMessage,
  autoFollowEnterDirective,
  refillCommandInputWithActiveTarget,
}) {
  function applyWorldResponse(result) {
    if (!state.currentHome) {
      return;
    }

    function syncRoomHeading() {
      updateRoomHeading(state.currentHome.roomTitle || '', state.currentHome.roomDescription || '');
    }

    function seedPresenceFromResultAvatars(avatars) {
      if (Array.isArray(avatars) && avatars.length > 0) {
        for (const avatar of avatars) {
          const handle = String(avatar?.handle || '').trim();
          if (handle) trackRoomPresence(handle, String(avatar?.did || ''));
        }
        return;
      }
      // Fallback: seed with self only (snapshot push will fill the rest).
      trackRoomPresence(state.currentHome.handle || state.aliasName, state.identity?.did || '');
    }

    function applyRoomChange(nextRoom) {
      const previousRoom = state.currentHome.room;
      state.currentHome.room = nextRoom;
      if (result.room_did) state.currentHome.roomDid = result.room_did;
      if (result.world_did) state.currentHome.worldDid = result.world_did;
      if (result.room_title) state.currentHome.roomTitle = result.room_title;
      if (typeof result.room_description === 'string') state.currentHome.roomDescription = result.room_description;
      saveLastRoom(state.currentHome.endpointId, nextRoom);
      if (typeof syncSpecialAliases === 'function') {
        syncSpecialAliases();
      }
      updateIdentityLine();
      syncRoomHeading();

      if (nextRoom === previousRoom) {
        return;
      }

      clearActiveObjectTarget();
      state.roomDidLookupCache.clear();
      state.roomDidLookupInFlight.clear();
      clearRoomPresence();
      seedPresenceFromResultAvatars(result.avatars);
    }

    function applyRoomMetadataPatch() {
      if (typeof result.room_title === 'string' && result.room_title) {
        state.currentHome.roomTitle = result.room_title;
      }
      if (typeof result.room_description === 'string') {
        state.currentHome.roomDescription = result.room_description;
      }
      syncRoomHeading();
    }

    function applyDirectMessageResponse() {
      primeDidLookupCacheFromWorldMessage(result.message);
      appendMessage('world', result.message || '(no response)');
      autoFollowEnterDirective(result.message).catch((err) => {
        appendMessage('system', `Auto-enter failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      if (state.activeObjectTargetAlias) {
        refillCommandInputWithActiveTarget();
      }
    }

    if (result.room) {
      applyRoomChange(result.room);
      saveActiveHomeSnapshot();
    } else if (result.room_description !== undefined || result.room_title !== undefined) {
      applyRoomMetadataPatch();
    }

    state.currentHome.lastEventSequence = toSequenceNumber(
      result.latest_event_sequence || state.currentHome.lastEventSequence || 0
    );

    primeDidLookupCacheFromRoomObjectDids(result.room_object_dids);

    if (!result.broadcasted) {
      applyDirectMessageResponse();
    }
  }

  return {
    applyWorldResponse,
  };
}
