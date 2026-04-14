import { isMaDid } from './did.js';

export function createWhisperFlow({
  state,
  resolveAliasInput,
  findDidByEndpoint,
  fetchDidDocumentJsonByDid,
  sendWorldWhisper,
  sendWorldWhisperWithTtl,
  sendDirectMessage,
  sendDirectMessageWithTtl,
  getMessageTtl,
}) {
  function resolveTargetDid(targetDidOrAlias) {
    const key = String(targetDidOrAlias || '').trim().replace(/^@+/, '');
    if (!key) return null;
    const resolved = resolveAliasInput(key);
    const mappedDid = state.handleDidMap[key]
      || state.handleDidMap[String(key).toLowerCase()]
      || state.handleDidMap[resolved]
      || state.handleDidMap[String(resolved).toLowerCase()]
      || '';
    const targetDid = isMaDid(key)
      ? key
      : (isMaDid(resolved) ? resolved : (mappedDid || findDidByEndpoint(resolved) || resolved));
    return isMaDid(String(targetDid)) ? targetDid : null;
  }

  async function sendWhisperToDid(targetDidOrAlias, text, options = {}) {
    if (!state.identity || !state.currentHome) {
      throw new Error('Join a home before sending chat.');
    }

    const targetDid = resolveTargetDid(targetDidOrAlias);
    if (!targetDid) {
      throw new Error(`Message target must be a did:ma DID, alias, or known handle mapped to a DID. Got: ${targetDidOrAlias}`);
    }

    const recipientDocumentJson = await fetchDidDocumentJsonByDid(targetDid);
    const temporaryOverride = Number(state.temporaryMessageTtlOverride);
    const ttlOverride = options && Object.prototype.hasOwnProperty.call(options, 'ttlSeconds')
      ? Number(options.ttlSeconds)
      : null;
    const ttlSeconds = Number.isFinite(ttlOverride) && ttlOverride >= 0
      ? Math.floor(ttlOverride)
      : Number.isFinite(temporaryOverride) && temporaryOverride >= 0
        ? Math.floor(temporaryOverride)
      : Number(getMessageTtl('whisper'));
    const result = JSON.parse(
      await sendWorldWhisperWithTtl(
        state.currentHome.endpointId,
        state.passphrase,
        state.encryptedBundle,
        state.aliasName,
        recipientDocumentJson,
        text,
        BigInt(ttlSeconds)
      )
    );

    if (!result.ok) {
      throw new Error(result.message || 'whisper failed');
    }
  }

  /// Send a persistent encrypted message (application/x-ma-message) over the inbox lane.
  /// The recipient need not be online — the message is queued in their ma/inbox/1.
  async function sendMessageToDid(targetDidOrAlias, text) {
    if (!state.identity) {
      throw new Error('Load an identity before sending a message.');
    }

    const targetDid = resolveTargetDid(targetDidOrAlias);
    if (!targetDid) {
      throw new Error(`Message target must be a did:ma DID, alias, or known handle mapped to a DID. Got: ${targetDidOrAlias}`);
    }

    const recipientDocumentJson = await fetchDidDocumentJsonByDid(targetDid);
    const result = JSON.parse(
      await sendDirectMessage(
        state.passphrase,
        state.encryptedBundle,
        state.aliasName,
        recipientDocumentJson,
        text,
      )
    );

    if (!result.ok) {
      throw new Error(result.message || 'message send failed');
    }
  }

  return {
    sendWhisperToDid,
    sendMessageToDid,
  };
}
