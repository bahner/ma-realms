import { isMaDid } from './did.js';

export function createWhisperFlow({
  state,
  resolveAliasInput,
  findDidByEndpoint,
  fetchDidDocumentJsonByDid,
  sendWorldWhisper,
  sendWorldWhisperWithTtl,
  getMessageTtl,
}) {
  async function sendWhisperToDid(targetDidOrAlias, text, options = {}) {
    if (!state.identity || !state.currentHome) {
      throw new Error('Join a home before sending chat.');
    }

    const key = String(targetDidOrAlias || '').trim().replace(/^@+/, '');
    if (!key) {
      throw new Error("Usage: @target '<message>");
    }

    const resolved = resolveAliasInput(key);
    const mappedDid = state.handleDidMap[key]
      || state.handleDidMap[String(key).toLowerCase()]
      || state.handleDidMap[resolved]
      || state.handleDidMap[String(resolved).toLowerCase()]
      || '';
    const targetDid = isMaDid(key)
      ? key
      : (isMaDid(resolved) ? resolved : (mappedDid || findDidByEndpoint(resolved) || resolved));
    if (!isMaDid(String(targetDid))) {
      throw new Error(`Message target must be a did:ma DID, alias, or known handle mapped to a DID. Got: ${targetDid}`);
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
      throw new Error(result.message || 'direct message failed');
    }
  }

  return {
    sendWhisperToDid,
  };
}
