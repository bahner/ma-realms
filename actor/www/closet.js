import {
  create_identity_with_ipns,
  closet_start,
  closet_command,
  closet_publish_did_document,
} from './pkg/ma_actor.js';
import { DID_MA_PREFIX, isMaDid } from './did.js';

export function createClosetFlow({
  state,
  byId,
  appendMessage,
  didRoot,
  isValidAliasName,
  saveIdentityRecord,
  updateIdentityLine,
  ensureBundleIrohSecret,
}) {
  function isClosetRequiredMessage(message) {
    const text = String(message || '').toLowerCase();
    return text.includes('closet') || text.includes('not registered in room');
  }

  function isClosetBootstrapFailureMessage(message) {
    const text = String(message || '').toLowerCase();
    return (
      text.includes('document signature is invalid')
      || text.includes('failed to decode did document')
      || text.includes('did document')
      || text.includes('verify') && text.includes('message')
      || text.includes('verify') && text.includes('signature')
      || text.includes('sender document')
      || text.includes('unknown did')
    );
  }

  function normalizeClosetInput(input) {
    const raw = String(input || '').trim();
    if (!raw) {
      return null;
    }

    const prefixed = raw.match(/^\/?closet\s+(.+)$/i);
    if (prefixed) {
      const command = String(prefixed[1] || '').trim();
      return command || null;
    }

    const first = raw.split(/\s+/, 1)[0].toLowerCase();
    const closetVerbs = new Set([
      'help',
      'show',
      'hear',
      'name',
      'description',
      'desc',
      'alias',
      'apply',
      'citizen',
      'enter',
      'recovery'
    ]);

    if (closetVerbs.has(first)) {
      return raw;
    }
    return null;
  }

  function parseClosetResponse(rawJson) {
    const parsed = JSON.parse(String(rawJson || '{}'));
    if (!parsed || typeof parsed !== 'object') {
      throw new Error('Invalid closet response.');
    }
    return parsed;
  }

  function adoptClosetAssignedDid(assignedDid, assignedFragment = '') {
    const did = String(assignedDid || '').trim();
    const root = didRoot(did);
    const ipns = isMaDid(root) ? root.slice(DID_MA_PREFIX.length) : '';
    if (!ipns || !state.passphrase) {
      return;
    }

    try {
      const created = JSON.parse(create_identity_with_ipns(state.passphrase, ipns));
      const localized = JSON.parse(
        ensureBundleIrohSecret(state.passphrase, created.encrypted_bundle)
      );
      const updated = localized;

      state.identity = updated;
      state.encryptedBundle = updated.encrypted_bundle;
      const bundleEl = byId('bundle-text');
      if (bundleEl) {
        bundleEl.value = updated.encrypted_bundle;
      }
      if (isValidAliasName(state.aliasName || '')) {
        saveIdentityRecord(state.aliasName, updated.encrypted_bundle);
      }
      appendMessage('system', `Identity rebound to ${root}.`);
      appendMessage('system', `Publishing DID document for ${root}...`);

      const publishWork = (async () => {
        const endpointId = String(state.closetEndpointId || '').trim();
        const sessionId = String(state.closetSessionId || '').trim();
        const ipnsPrivateKeyBase64 = String(state.closetPendingIpnsPrivateKeyB64 || '').trim();
        if (!endpointId || !sessionId) {
          throw new Error('No active closet session for DID publish.');
        }
        const published = parseClosetResponse(
          await closet_publish_did_document(
            endpointId,
            sessionId,
            updated.document_json,
            ipnsPrivateKeyBase64
          )
        );
        if (!published.ok) {
          throw new Error(published.message || 'Closet DID publish failed.');
        }
        state.closetPendingIpnsPrivateKeyB64 = '';
        state.didDocCache.delete(root);
        state.didPublishError = '';
        appendMessage('system', `Published DID document to /ipns/${ipns} (key: ${assignedFragment || '(session key)'}).`);
      })();

      state.didPublishPromise = publishWork
        .catch((error) => {
          const message = error instanceof Error ? error.message : String(error);
          state.didPublishError = message;
          appendMessage('system', `Warning: DID document not published yet (${message}).`);
          throw error;
        })
        .finally(() => {
          if (state.didPublishPromise === publishWork || state.didPublishPromise) {
            state.didPublishPromise = null;
          }
        });

      updateIdentityLine();
    } catch (error) {
      appendMessage('system', `Warning: could not rebind local identity (${error instanceof Error ? error.message : String(error)}).`);
    }
  }

  function renderClosetResponse(response) {
    const message = String(response?.message || '').trim();
    if (message) {
      appendMessage('system', message);
    }
    const prompt = String(response?.prompt || '').trim();
    if (prompt) {
      appendMessage('system', prompt);
    }
    const events = Array.isArray(response?.lobby_events) ? response.lobby_events : [];
    for (const event of events) {
      const body = String(event?.message || '').trim();
      const sender = String(event?.sender || 'lobby');
      if (body) {
        appendMessage('system', `[lobby/${sender}] ${body}`);
      }
    }
    if (response?.did) {
      appendMessage('system', `Assigned DID: ${response.did}`);
      adoptClosetAssignedDid(String(response.did), String(response?.fragment || ''));
    }
    if (response?.fragment) {
      appendMessage('system', `Assigned fragment: ${response.fragment}`);
    }
  }

  async function closetStartSessionForEndpoint(endpointId) {
    const normalizedEndpoint = String(endpointId || '').trim();
    if (!normalizedEndpoint) {
      throw new Error('Missing world endpoint for closet session.');
    }
    const response = parseClosetResponse(await closet_start(normalizedEndpoint));
    if (!response.ok) {
      throw new Error(response.message || 'Closet session failed to start.');
    }
    state.closetSessionId = String(response.session_id || '').trim();
    state.closetEndpointId = normalizedEndpoint;
    state.closetLobbySeq = Number(response.latest_lobby_sequence || 0);
    return response;
  }

  async function closetCommandForCurrentWorld(input) {
    if (!state.closetSessionId) {
      throw new Error('No active closet session.');
    }
    const rawInput = String(input || '').trim();
    const applyMatch = rawInput.match(/^(apply|citizen)(?:\s+(.+))?$/i);
    if (applyMatch) {
      state.closetPendingIpnsPrivateKeyB64 = String(applyMatch[2] || '').trim();
    }
    const endpointId = String(state.closetEndpointId || '').trim();
    if (!endpointId) {
      throw new Error('No world endpoint available for closet command.');
    }
    const response = parseClosetResponse(
      await closet_command(endpointId, state.closetSessionId, rawInput)
    );
    if (!response.ok) {
      throw new Error(response.message || 'Closet command failed.');
    }
    state.closetLobbySeq = Number(response.latest_lobby_sequence || state.closetLobbySeq || 0);
    return response;
  }

  return {
    isClosetRequiredMessage,
    isClosetBootstrapFailureMessage,
    normalizeClosetInput,
    renderClosetResponse,
    closetStartSessionForEndpoint,
    closetCommandForCurrentWorld,
  };
}
