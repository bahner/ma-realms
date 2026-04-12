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
    fetchDidDocumentJsonByDid,
    decodeChatEventMessage,
    decodeWhisperEventMessage,
    onPresenceEvent,
    onPresenceRefreshRequest,
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

  function writeDialogChat(senderDid, senderHandle, text) {
    const actor = displayActor(senderDid, senderHandle);
    appendMessage('world', `${actor}: ${text}`);
  }

  function writeDialogWhisper(senderDid, senderHandle, text) {
    const actor = displayActor(senderDid, senderHandle);
    appendMessage('world', `${actor} whispers ${text}.`);
  }

  function writeDialogSystem(text) {
    appendMessage('system', String(text || ''));
  }

  function writeDialogWorld(text) {
    appendMessage('world', String(text || ''));
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
    const kind = String(payload?.kind || '').trim();
    if (kind === 'presence.refresh.request' && typeof onPresenceRefreshRequest === 'function') {
      await onPresenceRefreshRequest(payload, evt);
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
      state.didEndpointMap[evt.senderDid] = evt.senderEndpoint;
    }

    if (shouldSuppressInboundEcho(evt)) {
      logger.log('inbox.dispatch', `dropped self-echo sender=${evt.senderDid || ''}`);
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

export function createInboxTransport({
  state,
  logger,
  startInboxListener,
  pollInboxMessages,
  inspectSignedMessage,
  dispatchInboundEvent,
  notifyMailboxMessage
}) {
  function messageRequiresResponse(meta) {
    const contentType = String(meta?.content_type || '').trim().toLowerCase();
    const contentText = String(meta?.content_text || '').trim().toLowerCase();
    if (!contentType && !contentText) {
      return false;
    }

    if (
      contentType === 'application/x-ma-whisper' ||
      contentType === 'application/x.ma.whisper' ||
      contentType === 'application/x-ma-cmd' ||
      contentType === 'application/x-ma-command' ||
      contentType === 'application/x-ma-world'
    ) {
      return true;
    }

    return /\bknock\b/.test(contentText);
  }

  function queueMailboxItem(item, meta) {
    if (!Array.isArray(state.mailbox)) {
      state.mailbox = [];
    }
    const nextId = Number(state.mailboxSeq || 0) + 1;
    state.mailboxSeq = nextId;

    const entry = {
      id: nextId,
      from_did: String(meta?.from || '').trim(),
      from_endpoint: String(item?.from_endpoint || '').trim(),
      content_type: String(meta?.content_type || '').trim(),
      content_text: String(meta?.content_text || '').trim(),
      message_cbor_b64: String(item?.message_cbor_b64 || '').trim(),
      received_at: Number(item?.received_at || 0)
    };

    state.mailbox.push(entry);
    const maxItems = 256;
    if (state.mailbox.length > maxItems) {
      state.mailbox.splice(0, state.mailbox.length - maxItems);
    }

    if (typeof notifyMailboxMessage === 'function') {
      notifyMailboxMessage(entry);
    }
  }

  async function ensureInboxListener() {
    if (state.inboxEndpointId) {
      return state.inboxEndpointId;
    }
    const endpointId = await startInboxListener(state.passphrase, state.encryptedBundle);
    state.inboxEndpointId = endpointId;
    logger.log('inbox.listener', `listening on ${endpointId}`);
    return endpointId;
  }

  async function pollDirectInbox() {
    if (!state.identity || state.inboxPollInFlight) {
      return;
    }
    await ensureInboxListener();
    state.inboxPollInFlight = true;

    try {
      const result = JSON.parse(await pollInboxMessages());
      if (!result || !Array.isArray(result.messages) || result.messages.length === 0) {
        return;
      }

      for (const item of result.messages) {
        const meta = JSON.parse(inspectSignedMessage(item.message_cbor_b64));
        if (messageRequiresResponse(meta)) {
          queueMailboxItem(item, meta);
          continue;
        }

        const routeKey = contentTypeToRouteKey(meta.content_type);
        if (!routeKey) {
          logger.log('inbox.dispatch', `ignoring unsupported inbound content_type=${meta.content_type}`);
          continue;
        }

        const kind = routeKey === 'application/x-ma-whisper'
          ? 'whisper'
          : routeKey === 'application/x-ma-presence'
            ? 'presence'
            : 'chat';

        await dispatchInboundEvent({
          kind,
          mime_type: routeKey,
          sender: '',
          sender_did: meta.from,
          sender_endpoint: item.from_endpoint || '',
          message: kind === 'presence' ? (meta.content_text || '') : '',
          message_cbor_b64: item.message_cbor_b64,
          sequence: 0,
          occurred_at: ''
        });
      }
    } finally {
      state.inboxPollInFlight = false;
    }
  }

  return {
    ensureInboxListener,
    pollDirectInbox
  };
}
