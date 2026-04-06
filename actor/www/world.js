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
}) {
  function isNotRegisteredInRoomMessage(message) {
    const text = String(message || '').toLowerCase();
    return text.includes('not registered in room');
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
    const normalizedAlias = String(alias || '').trim() || '@dings';
    appendMessage('system', `${normalizedAlias} vanished in a puff of logic.`);
  }

  async function restoreActiveObjectTargetAfterReentry(alias, did) {
    const normalizedAlias = String(alias || '').trim();
    const normalizedDid = String(did || '').trim();
    if (!normalizedAlias.startsWith('@') || !isMaDid(normalizedDid)) {
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
    if (state.transparentReentryPromise) {
      return await state.transparentReentryPromise;
    }

    if (!state.currentHome) {
      throw new Error('Not connected to a world.');
    }

    const home = state.currentHome;
    const endpointId = String(home.endpointId || '').trim();
    const room = String(home.room || '').trim() || 'lobby';
    const activeAlias = String(state.activeObjectTargetAlias || '').trim();
    const activeDid = String(state.activeObjectTargetDid || '').trim();
    const resumeTarget = buildCurrentHomeResumeTarget() || endpointId;

    const work = (async () => {
      logger.log(
        'reconnect',
        `transparent re-entry triggered (${reason || 'unknown reason'}) endpoint=${endpointId.slice(0, 8)}... room=${room}`
      );
      await enterHome(resumeTarget, room, { silent: true });
      await restoreActiveObjectTargetAfterReentry(activeAlias, activeDid);
    })();

    state.transparentReentryPromise = work;
    try {
      await work;
    } finally {
      if (state.transparentReentryPromise === work) {
        state.transparentReentryPromise = null;
      }
    }
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
  normalizeClosetInput,
  closetCommandForCurrentWorld,
  renderClosetResponse,
  enterHome,
  isLikelyIrohAddress,
  normalizeIrohAddress,
  parseDot,
  resolveCommandTargetDidOrToken,
  logger,
  sendWorldChat,
  sendWorldCmd,
  pollCurrentHomeEvents,
  appendAmbientProseAfterSpeech,
  renderLocalBroadcastMessage,
  applyWorldResponse,
  tryHandleDidTargetMetaPoll,
  sendWhisperToDid,
  isNotRegisteredInRoomMessage,
  performTransparentReentry,
}) {
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

  function activeActorName() {
    const fragment = actorFragmentFromDid(state.identity?.did);
    if (fragment) {
      return fragment;
    }
    return String(state.aliasName || 'actor').trim();
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
    if (!state.aliasRewriteEnabled) {
      return text;
    }
    if (!text.trim() || text.trim().startsWith("'")) {
      return text;
    }

    const aliases = aliasTargetMap();
    if (!aliases.size) {
      return text;
    }

    let out = text;

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

    out = out.replace(/@([^\s]+)/g, (all, token) => {
      const raw = String(token || '').trim();
      if (!raw) {
        return all;
      }
      const dotIdx = raw.indexOf('.');
      const base = dotIdx === -1 ? raw : raw.slice(0, dotIdx);
      const suffix = dotIdx === -1 ? '' : raw.slice(dotIdx);
      const resolved = resolveAliasTarget(base, aliases);
      if (!resolved) {
        return all;
      }
      const targetToken = isMaDid(resolved) ? `@${resolved}` : resolved;
      return `${targetToken}${suffix}`;
    });

    return out;
  }

  async function sendWorldCommandQuery(commandText) {
    if (!state.identity || !state.currentHome) {
      throw new Error('Join a home before sending commands.');
    }

    const result = JSON.parse(
      await sendWorldCmd(
        state.currentHome.endpointId,
        state.passphrase,
        state.encryptedBundle,
        activeActorName(),
        state.currentHome.room,
        commandText
      )
    );

    if (!result.ok) {
      throw new Error(result.message || 'command failed');
    }
    if (result.broadcasted) {
      await pollCurrentHomeEvents();
    }
    return String(result.message || '');
  }

  async function sendCurrentWorldMessage(text, options = {}) {
    const attempt = (options && typeof options === 'object' && Number.isFinite(options.attempt))
      ? Number(options.attempt)
      : 0;

    try {
      const rewrittenInput = rewriteAliasesToDid(text);
      const trimmedText = rewrittenInput.trim();
      const closetInput = normalizeClosetInput(trimmedText);
      const hasActiveClosetSession = Boolean(
        String(state.closetSessionId || '').trim()
        && String(state.closetEndpointId || '').trim()
      );
      const isClosetEndpointActive = hasActiveClosetSession && (
        !state.currentHome
        || String(state.currentHome?.endpointId || '').trim() === String(state.closetEndpointId || '').trim()
      );

      if (!state.identity) {
        appendMessage('system', 'Create or unlock an identity first.');
        return;
      }

      if (isClosetEndpointActive) {
        const shortcutConnectTarget = String(trimmedText || '').trim();
        if (shortcutConnectTarget.startsWith('@') && !/\s/u.test(shortcutConnectTarget)) {
          await enterHome(shortcutConnectTarget);
          return;
        }

        const goMatch = trimmedText.match(/^go\s+(.+)$/i);
        if (goMatch) {
          const targetOrRoom = String(goMatch[1] || '').trim();
          if (targetOrRoom) {
            const connectTarget = resolveWorldConnectTarget(targetOrRoom);
            if (connectTarget) {
              await enterHome(targetOrRoom);
              return;
            }

            if (!String(state.closetSessionDid || '').trim()) {
              const applyResponse = await closetCommandForCurrentWorld('apply');
              renderClosetResponse(applyResponse);
              if (!String(state.closetSessionDid || '').trim()) {
                appendMessage('system', 'Closet session still has no DID. Complete required profile fields, then try go out again.');
                return;
              }
            }

            const response = await closetCommandForCurrentWorld(`enter ${targetOrRoom}`);
            renderClosetResponse(response);
            const reconnectRoom = targetOrRoom.toLowerCase() === 'out' ? 'lobby' : targetOrRoom;
            await enterHome(state.closetEndpointId, reconnectRoom, {
              silent: true,
              skipLocalDidProbe: true,
            });
            return;
          }
        }

        if (closetInput) {
          const response = await closetCommandForCurrentWorld(closetInput);
          renderClosetResponse(response);
        } else {
          appendMessage('system', 'Active closet session. Use dot commands (for example: avatar.peek, avatar.name bahner, actor.peek, actor.apply).');
        }
        return;
      }

      if (!state.currentHome) {
        const shortcutConnectTarget = String(trimmedText || '').trim();
        if (shortcutConnectTarget.startsWith('@') && !/\s/u.test(shortcutConnectTarget)) {
          await enterHome(shortcutConnectTarget);
          return;
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

      if (state.closetSessionId && state.closetEndpointId
        && state.currentHome.endpointId === state.closetEndpointId
        && closetInput) {
        const response = await closetCommandForCurrentWorld(closetInput);
        renderClosetResponse(response);
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
          await sendWhisperToDid(targetDid, payload);
          appendMessage('system', `Chat sent to ${targetDid}.`);
          return;
        } catch (err) {
          appendMessage('system', `Error sending chat to #${fragment}: ${err.message}`);
          return;
        }
      }

      if (trimmedText.startsWith("'")) {
        const payload = trimmedText.substring(1);
        const sendStart = Date.now();
        logger.log('send.chat', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${payload.length}`);

        const result = JSON.parse(
          await sendWorldChat(
            state.currentHome.endpointId,
            state.passphrase,
            state.encryptedBundle,
            activeActorName(),
            state.currentHome.room,
            payload
          )
        );
        const elapsed = Date.now() - sendStart;
        logger.log('send.chat', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

        if (!result.ok) {
          throw new Error(result.message || 'chat failed');
        }

        renderLocalBroadcastMessage(payload);
        await pollCurrentHomeEvents();
        appendAmbientProseAfterSpeech().catch((err) => {
          logger.log('ambient.prose', `failed: ${err instanceof Error ? err.message : String(err)}`);
        });
        return;
      }

      if (trimmedText.startsWith('@')) {
        const trimmed = trimmedText;
        const whisperSep = trimmed.indexOf(" '");
        if (whisperSep > 1) {
          const target = trimmed.substring(1, whisperSep).trim();
          const payload = trimmed.substring(whisperSep + 2);
          try {
            const targetDid = await resolveCommandTargetDidOrToken(target);
            if (!isMaDid(String(targetDid))) {
              throw new Error(`Whisper target must resolve to did:ma, got: ${targetDid}`);
            }
            await sendWhisperToDid(targetDid, payload);
            appendMessage('system', `Chat sent to ${targetDid}.`);
            return;
          } catch (err) {
            appendMessage('system', `Error sending chat to ${target}: ${err.message}`);
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

        const baseTarget = String(rawTarget.split('.')[0] || '').trim().toLowerCase();
        const isBuiltinPathTarget = baseTarget === 'world'
          || baseTarget === 'avatar'
          || baseTarget === 'me'
          || baseTarget === 'self'
          || baseTarget === 'here'
          || baseTarget === 'room';

        const hasDottedPath = rawTarget.includes('.');
        const canSendAsIs = isBuiltinPathTarget && (hasDottedPath || !remainder);

        if (canSendAsIs) {
          const normalizedInput = remainder
            ? `@${rawTarget} ${remainder}`
            : `@${rawTarget}`;
          const sendStart = Date.now();
          logger.log('send.command', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${normalizedInput.length}`);
          const result = JSON.parse(
            await sendWorldCmd(
              state.currentHome.endpointId,
              state.passphrase,
              state.encryptedBundle,
              activeActorName(),
              state.currentHome.room,
              normalizedInput
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

          await pollCurrentHomeEvents();
          return;
        }

        if (!remainder) {
          appendMessage('system', '?');
          return;
        }

        const target = rawTarget;

        if (await tryHandleDidTargetMetaPoll(target, remainder)) {
          return;
        }

        const resolvedTarget = await resolveCommandTargetDidOrToken(target);
        const normalized = `@${resolvedTarget} ${remainder}`;

        const sendStart = Date.now();
        logger.log('send.command', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${trimmed.length}`);
        const result = JSON.parse(
          await sendWorldCmd(
            state.currentHome.endpointId,
            state.passphrase,
            state.encryptedBundle,
            activeActorName(),
            state.currentHome.room,
            normalized
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

        await pollCurrentHomeEvents();
        return;
      }

      const sendStart = Date.now();
      logger.log('send.command', `room=${state.currentHome.room} actor=${activeActorName()} msg_len=${trimmedText.length}`);

      const result = JSON.parse(
        await sendWorldCmd(
          state.currentHome.endpointId,
          state.passphrase,
          state.encryptedBundle,
          activeActorName(),
          state.currentHome.room,
          trimmedText
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

      if (trimmedText.startsWith("'")) {
        renderLocalBroadcastMessage(trimmedText.substring(1));
      }
      await pollCurrentHomeEvents();
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
      if (result.room_title) state.currentHome.roomTitle = result.room_title;
      if (typeof result.room_description === 'string') state.currentHome.roomDescription = result.room_description;
      saveLastRoom(state.currentHome.endpointId, nextRoom);
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
