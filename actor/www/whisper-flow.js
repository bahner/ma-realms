import { isMaDid } from './did.js';

export function createWhisperFlow({
  state,
  resolveAliasInput,
  findDidByEndpoint,
  fetchDidDocumentJsonByDid,
  sendWorldWhisper,
}) {
  async function sendWhisperToDid(targetDidOrAlias, text) {
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
      throw new Error(`Chat target must be a did:ma: DID, alias, or known handle mapped to a DID. Got: ${targetDid}`);
    }

    const recipientDocumentJson = await fetchDidDocumentJsonByDid(targetDid);
    const result = JSON.parse(
      await sendWorldWhisper(
        state.currentHome.endpointId,
        state.passphrase,
        state.encryptedBundle,
        state.aliasName,
        recipientDocumentJson,
        text
      )
    );

    if (!result.ok) {
      throw new Error(result.message || 'whisper failed');
    }
  }

  return {
    sendWhisperToDid,
  };
}
