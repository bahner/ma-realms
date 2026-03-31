import { contentTypeToRouteKey } from './inbox-dispatcher.js';

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
