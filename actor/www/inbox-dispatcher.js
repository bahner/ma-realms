export function contentTypeToRouteKey(contentType) {
  const value = String(contentType || '').trim();
  if (value === 'application/x-ma-chat' || value === 'application/x.ma.chat') {
    return 'application/x-ma-chat';
  }
  if (value === 'application/x-ma-whisper' || value === 'application/x.ma.whisper') {
    return 'application/x-ma-whisper';
  }
  if (value === 'application/x-ma-presence' || value === 'application/x.ma.presence') {
    return 'application/x-ma-presence';
  }
  return '';
}

export function createInboundDispatcher(deps) {
  const {
    state,
    logger,
    appendMessage,
    displayActor,
    humanizeText,
    fetchDidDocumentJsonByDid,
    decodeChatEventMessage,
    decodeWhisperEventMessage,
    onPresenceEvent,
    didRoot
  } = deps;

  function inferInboundMimeType(kind) {
    if (kind === 'chat') return 'application/x-ma-chat';
    if (kind === 'whisper') return 'application/x-ma-whisper';
    return '';
  }

  function normalizeInboundEvent(event) {
    if (!event || typeof event !== 'object') {
      return null;
    }
    return {
      kind: String(event.kind || 'default'),
      mimeType: String(event.mime_type || '').trim() || inferInboundMimeType(String(event.kind || 'default')),
      senderHandle: event.sender || '',
      senderDid: event.sender_did || '',
      senderEndpoint: event.sender_endpoint || '',
      text: event.message || '',
      messageCborB64: event.message_cbor_b64 || ''
    };
  }

  function shouldSuppressInboundEcho(evt) {
    const currentDid = String(state.identity?.did || '').trim();
    const currentRootDid = didRoot(currentDid);
    const senderRootDid = didRoot(evt.senderDid || '');
    return Boolean(
      (evt.kind === 'speech' || evt.kind === 'chat') &&
        senderRootDid &&
        currentRootDid &&
        senderRootDid === currentRootDid
    );
  }

  function isBlockedSender(evt) {
    const senderRootDid = didRoot(evt.senderDid || '');
    if (!senderRootDid) {
      return false;
    }
    const blocked = state.blockedDidRoots;
    if (!blocked) {
      return false;
    }
    if (blocked instanceof Set) {
      return blocked.has(senderRootDid);
    }
    if (Array.isArray(blocked)) {
      return blocked.includes(senderRootDid);
    }
    return false;
  }

  function writeDialogChat(senderDid, senderHandle, text) {
    const actor = displayActor(senderDid, senderHandle);
    appendMessage('world', humanizeText(`${actor}: ${text}`));
  }

  function writeDialogWhisper(senderDid, senderHandle, text) {
    const actor = displayActor(senderDid, senderHandle);
    appendMessage('world', humanizeText(`${actor} whispers ${text}.`));
  }

  function writeDialogSystem(text) {
    appendMessage('system', humanizeText(text));
  }

  function writeDialogWorld(text) {
    appendMessage('world', humanizeText(text));
  }

  function logInboundDispatch(evt, routeName) {
    const incomingType = evt.mimeType || `event/${evt.kind}`;
    const sender = evt.senderDid || evt.senderHandle || 'unknown';
    logger.log('inbox.dispatch', `received ${incomingType} message from ${sender} sent to ${routeName} handler`);
  }

  async function handleInboundChat(evt) {
    if (!evt.messageCborB64) {
      return;
    }
    const senderDid = evt.senderDid;
    if (!senderDid) {
      throw new Error('missing sender DID for chat');
    }
    const senderDoc = await fetchDidDocumentJsonByDid(senderDid);
    const text = decodeChatEventMessage(senderDoc, evt.messageCborB64);
    writeDialogChat(evt.senderDid, evt.senderHandle, text);
  }

  async function handleInboundWhisper(evt) {
    if (!evt.messageCborB64) {
      return;
    }
    const senderDid = evt.senderDid;
    if (!senderDid) {
      throw new Error('missing sender DID for whisper');
    }
    const senderDoc = await fetchDidDocumentJsonByDid(senderDid);
    const text = decodeWhisperEventMessage(
      state.passphrase,
      state.encryptedBundle,
      senderDoc,
      evt.messageCborB64
    );
    writeDialogWhisper(senderDid, evt.senderHandle, text);
  }

  async function handleInboundSystem(evt) {
    if (!evt.text) return;
    writeDialogSystem(evt.text);
  }

  async function handleInboundSpeech(evt) {
    if (!evt.text) return;
    writeDialogChat(evt.senderDid, evt.senderHandle, evt.text);
  }

  async function handleInboundDefault(evt) {
    if (!evt.text) return;
    writeDialogWorld(evt.text);
  }

  async function handleInboundPresence(evt) {
    const expectedRoomDid = state.currentHome?.roomDid;
    if (!expectedRoomDid || evt.senderDid !== expectedRoomDid) {
      logger.log('inbox.presence', `dropped presence from unexpected sender ${evt.senderDid || '(none)'} (expected room did ${expectedRoomDid || 'none'})`);
      return;
    }
    if (!evt.text) return;
    let payload;
    try {
      payload = JSON.parse(evt.text);
    } catch (error) {
      logger.log('inbox.presence', `invalid presence payload json: ${error instanceof Error ? error.message : String(error)}`);
      return;
    }
    if (typeof onPresenceEvent === 'function') {
      await onPresenceEvent(payload, evt);
    }
  }

  const inboundRoutes = {
    'application/x-ma-chat': { name: 'chat', handler: handleInboundChat },
    'application/x-ma-whisper': { name: 'whisper', handler: handleInboundWhisper },
    'application/x-ma-presence': { name: 'presence', handler: handleInboundPresence },
    'event/system': { name: 'system', handler: handleInboundSystem },
    'event/speech': { name: 'speech', handler: handleInboundSpeech },
    'event/default': { name: 'default', handler: handleInboundDefault }
  };

  async function dispatchInboundEvent(event) {
    const evt = normalizeInboundEvent(event);
    if (!evt) {
      logger.log('inbox.dispatch', 'dropped malformed inbound event');
      return;
    }

    if (!evt.text && !evt.messageCborB64) {
      logger.log('inbox.dispatch', `dropped empty inbound event kind=${evt.kind}`);
      return;
    }

    if (evt.senderHandle && evt.senderDid) {
      state.handleDidMap[evt.senderHandle] = evt.senderDid;
    }
    if (evt.senderDid && evt.senderEndpoint) {
      state.didEndpointMap[didRoot(evt.senderDid)] = evt.senderEndpoint;
    }

    if (isBlockedSender(evt)) {
      logger.log('inbox.dispatch', `dropped blocked sender ${didRoot(evt.senderDid || '')}`);
      return;
    }

    if (shouldSuppressInboundEcho(evt)) {
      logger.log('inbox.dispatch', `dropped self-echo sender=${didRoot(evt.senderDid || '')}`);
      return;
    }

    const routeKey = evt.mimeType || `event/${evt.kind}`;
    const route = inboundRoutes[routeKey] || inboundRoutes['event/default'];
    logInboundDispatch(evt, route.name);

    try {
      await route.handler(evt);
    } catch (error) {
      writeDialogSystem(`Failed to process inbound message: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  return {
    dispatchInboundEvent
  };
}
