import {
  create_identity_with_ipns,
  set_bundle_language,
  set_bundle_transports,
  start_inbox_listener,
  closet_start,
  closet_command,
  closet_submit_citizenship,
  closet_publish_did_document,
} from './pkg/ma_actor.js';
import { DID_MA_PREFIX, isMaDid } from './did.js';

export function createClosetFlow({
  state,
  byId,
  appendMessage,
  didRoot,
  didPublishPendingTtlMs = 5 * 60 * 1000,
  isValidAliasName,
  saveIdentityRecord,
  updateIdentityLine,
  ensureBundleIrohSecret,
}) {
  function pendingDidRoot(didLike = '') {
    const root = didRoot(String(didLike || '').trim());
    return String(root || '').trim();
  }

  function clearDidPublishPending(didLike = '') {
    const root = pendingDidRoot(didLike);
    if (!root) return;
    state.didPublishPendingCache.delete(root);
  }

  function markDidPublishPending(didLike = '') {
    const root = pendingDidRoot(didLike);
    if (!root) return 0;
    const ttlMs = Math.max(1_000, Number(didPublishPendingTtlMs || 0));
    const expiresAt = Date.now() + ttlMs;
    state.didPublishPendingCache.set(root, expiresAt);
    return expiresAt;
  }

  function pendingDidPublishExpiresAt(didLike = '') {
    const root = pendingDidRoot(didLike);
    if (!root) return 0;
    const expiresAt = Number(state.didPublishPendingCache.get(root) || 0);
    if (!Number.isFinite(expiresAt) || expiresAt <= Date.now()) {
      state.didPublishPendingCache.delete(root);
      return 0;
    }
    return expiresAt;
  }
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
      || text.includes('failed to fetch did document')
      || text.includes('name/resolve failed')
      || text.includes('/ipns/')
      || text.includes('enter request timed out')
      || text.includes('did document is not published yet')
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
      return normalizeStructuredClosetCommand(command);
    }

    const structured = normalizeStructuredClosetCommand(raw);
    if (structured && structured !== raw) {
      return structured;
    }

    const first = raw.split(/\s+/, 1)[0].toLowerCase();
    const closetVerbs = new Set([
      'help',
      'show',
      'hear',
      'reset',
      'restart',
      'kill',
      'name',
      'description',
      'desc',
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

  function normalizeStructuredClosetCommand(input) {
    const raw = String(input || '').trim();
    if (!raw) {
      return null;
    }

    const scoped = raw.match(/^([A-Za-z][A-Za-z0-9_-]*)(?:\.|\s+)(.+)$/);
    if (!scoped) {
      return raw;
    }

    const scope = String(scoped[1] || '').trim().toLowerCase();
    const rest = String(scoped[2] || '').trim();
    if (!scope || !rest) {
      return raw;
    }

    const pathPeek = rest.match(/^([A-Za-z0-9_.-]+)\s+(show|peek)$/i);
    if (pathPeek) {
      const path = String(pathPeek[1] || '').trim();
      const mode = String(pathPeek[2] || 'peek').toLowerCase();
      return `__resource_show_path__ ${scope} ${mode} ${path}`;
    }

    const opMatch = rest.match(/^([A-Za-z][A-Za-z0-9_-]*)(?:\s*:?\s*(.*))?$/);
    if (!opMatch) {
      return raw;
    }
    const op = String(opMatch[1] || '').trim().toLowerCase();
    const args = String(opMatch[2] || '').trim();

    const table = {
      avatar: {
        help: () => 'help',
        show: () => 'show',
        peek: () => '__resource_show__ avatar peek',
        apply: () => (args ? `apply ${args}` : 'apply'),
        name: () => (args ? `name ${args}` : 'name'),
        description: () => (args ? `description ${args}` : 'description'),
      },
      document: {
        help: () => `__resource_help__ ${scope}`,
        show: () => `__resource_show__ ${scope} show`,
        peek: () => `__resource_show__ ${scope} peek`,
        apply: () => (args ? `__resource_apply__ ${scope} ${args}` : `__resource_apply__ ${scope}`),
        publish: () => (args ? `__resource_apply__ ${scope} ${args}` : `__resource_apply__ ${scope}`),
        republish: () => (args ? `__resource_apply__ ${scope} ${args}` : `__resource_apply__ ${scope}`),
      },
    };

    const resolver = table?.[scope]?.[op];
    if (typeof resolver === 'function') {
      return resolver();
    }

    return raw;
  }

  function readPathValue(root, pathParts) {
    let cursor = root;
    for (const part of pathParts) {
      if (!cursor || typeof cursor !== 'object') {
        return { found: false, value: null };
      }

      if (part in cursor) {
        cursor = cursor[part];
        continue;
      }

      const camel = String(part || '').replace(/_([a-z])/g, (_, ch) => ch.toUpperCase());
      if (camel && camel in cursor) {
        cursor = cursor[camel];
        continue;
      }

      return { found: false, value: null };
    }
    return { found: true, value: cursor };
  }

  function assertSnakeCasePath(path) {
    const parts = String(path || '').split('.').map((entry) => entry.trim()).filter(Boolean);
    for (const part of parts) {
      if (!/^[a-z0-9_]+$/.test(part)) {
        throw new Error(`invalid path '${path}': use snake_case segments only`);
      }
    }
  }

  function renderDocumentShowMessage(path = '') {
    const docJson = String(state.identity?.document_json || '').trim();
    if (!docJson) {
      throw new Error('No local DID document is available. Unlock identity first.');
    }
    const document = JSON.parse(docJson);
    const normalizedPath = String(path || '').trim();
    if (!normalizedPath) {
      const summary = {
        id: document?.id || null,
        assertionMethod: document?.assertionMethod || null,
        keyAgreement: document?.keyAgreement || null,
        ma: document?.ma || null,
      };
      return `document.peek\n${JSON.stringify(summary, null, 2)}`;
    }

    assertSnakeCasePath(normalizedPath);

    const parts = normalizedPath.split('.').map((entry) => entry.trim()).filter(Boolean);
    const resolved = readPathValue(document, parts);
    if (!resolved.found) {
      throw new Error(`document path not found: ${normalizedPath}`);
    }
    return `document.${normalizedPath} peek\n${JSON.stringify(resolved.value, null, 2)}`;
  }

  function parseClosetProfileMessage(message) {
    const text = String(message || '').trim();
    const match = text.match(/^closet profile:\s+did=(.*?)\s+name=(.*?)\s+description=(.*?)\s+recovery=(\S+)$/i);
    if (!match) {
      return null;
    }
    return {
      did: String(match[1] || '').trim(),
      name: String(match[2] || '').trim(),
      description: String(match[3] || '').trim(),
      recovery: String(match[4] || '').trim(),
      fragment: String(state.closetSessionFragment || '').trim(),
    };
  }

  async function fetchClosetProfileSnapshot(endpointId) {
    const response = parseClosetResponse(
      await closet_command(endpointId, state.closetSessionId, 'show')
    );
    if (!response.ok) {
      throw new Error(response.message || 'Closet show failed.');
    }
    const snapshot = parseClosetProfileMessage(response.message);
    if (snapshot) {
      state.closetProfile = snapshot;
    }
    return snapshot;
  }

  async function renderAvatarPeekMessage(path = '', endpointId = '') {
    let snapshot = state.closetProfile || null;
    if (!snapshot && endpointId) {
      snapshot = await fetchClosetProfileSnapshot(endpointId);
    }
    if (!snapshot) {
      throw new Error('No closet avatar profile is available. Run avatar.show first.');
    }

    const normalizedPath = String(path || '').trim();
    if (!normalizedPath) {
      return `avatar.peek\n${JSON.stringify(snapshot, null, 2)}`;
    }

    const key = normalizedPath.toLowerCase();
    const map = {
      did: snapshot.did,
      name: snapshot.name,
      description: snapshot.description,
      recovery: snapshot.recovery,
      fragment: snapshot.fragment,
    };
    if (!(key in map)) {
      throw new Error(`avatar path not found: ${normalizedPath}`);
    }
    return `avatar.${normalizedPath} peek\n${JSON.stringify(map[key], null, 2)}`;
  }

  function parseClosetResponse(rawJson) {
    const parsed = JSON.parse(String(rawJson || '{}'));
    if (!parsed || typeof parsed !== 'object') {
      throw new Error('Invalid closet response.');
    }
    return parsed;
  }

  function normalizeDesiredFragment(raw) {
    const fragment = String(raw || '').trim().replace(/^#/, '');
    return isValidAliasName(fragment) ? fragment : '';
  }

  function desiredFragmentFromIdentityDid() {
    const did = String(state.identity?.did || '').trim();
    if (!did) {
      return '';
    }
    const idx = did.indexOf('#');
    if (idx < 0 || idx >= did.length - 1) {
      return '';
    }
    return normalizeDesiredFragment(did.slice(idx + 1));
  }

  function desiredFragmentForCloset() {
    const fromAlias = normalizeDesiredFragment(state.aliasName || '');
    if (fromAlias) {
      return fromAlias;
    }
    return desiredFragmentFromIdentityDid();
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
      let updated = localized;

      try {
        const preferredLanguageOrder = String(state.languageOrder || '').trim();
        if (preferredLanguageOrder) {
          updated = JSON.parse(
            set_bundle_language(state.passphrase, updated.encrypted_bundle, preferredLanguageOrder)
          );
        }
      } catch (languageError) {
        appendMessage(
          'system',
          `Warning: could not set DID language before publish (${languageError instanceof Error ? languageError.message : String(languageError)}).`
        );
      }

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

        if (!state.inboxEndpointId) {
          state.inboxEndpointId = await start_inbox_listener(state.passphrase, updated.encrypted_bundle);
        }

        if (state.inboxEndpointId) {
          try {
            updated = JSON.parse(
              set_bundle_transports(state.passphrase, updated.encrypted_bundle, state.inboxEndpointId)
            );
            state.identity = updated;
            state.encryptedBundle = updated.encrypted_bundle;
            const bundleEl = byId('bundle-text');
            if (bundleEl) {
              bundleEl.value = updated.encrypted_bundle;
            }
            if (isValidAliasName(state.aliasName || '')) {
              saveIdentityRecord(state.aliasName, updated.encrypted_bundle);
            }
          } catch (transportError) {
            appendMessage(
              'system',
              `Warning: could not set DID transport hints before publish (${transportError instanceof Error ? transportError.message : String(transportError)}).`
            );
          }
        }

        const published = parseClosetResponse(
          await closet_publish_did_document(
            endpointId,
            sessionId,
            updated.document_json,
            ipnsPrivateKeyBase64,
            desiredFragmentForCloset()
          )
        );
        if (!published.ok) {
          throw new Error(published.message || 'Closet DID publish failed.');
        }
        state.closetPendingIpnsPrivateKeyB64 = '';
        state.didDocCache.delete(root);
        state.didPublishError = '';
        clearDidPublishPending(root);
        appendMessage('system', `Published DID document to /ipns/${ipns} (key: ${assignedFragment || '(session key)'}).`);
      })();

      state.didPublishPromise = publishWork
        .catch((error) => {
          const message = error instanceof Error ? error.message : String(error);
          state.didPublishError = message;
          appendMessage('system', `Warning: DID document not published yet (${message}).`);
          return null;
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
      const assignedDid = String(response.did || '').trim();
      const previousSessionDid = String(state.closetSessionDid || '').trim();
      state.closetSessionDid = assignedDid;
      appendMessage('system', `Assigned DID: ${assignedDid}`);
      markDidPublishPending(assignedDid);

      if (assignedDid && assignedDid !== previousSessionDid) {
        adoptClosetAssignedDid(assignedDid, String(response?.fragment || ''));
      }
    }
    if (response?.fragment) {
      state.closetSessionFragment = String(response.fragment || '').trim();
      appendMessage('system', `Assigned fragment: ${response.fragment}`);
    }
    const profileSnapshot = parseClosetProfileMessage(message);
    if (profileSnapshot) {
      state.closetProfile = profileSnapshot;
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
    state.closetSessionDid = '';
    state.closetEndpointId = normalizedEndpoint;
    state.closetLobbySeq = Number(response.latest_lobby_sequence || 0);
    return response;
  }

  async function closetCommandForCurrentWorld(input) {
    if (!state.closetSessionId) {
      throw new Error('No active closet session.');
    }
    const rawInput = String(input || '').trim();
    const normalizedInput = normalizeStructuredClosetCommand(rawInput) || rawInput;

    if (/^(reset|restart|kill)$/i.test(normalizedInput)) {
      const endpointId = String(state.closetEndpointId || '').trim();
      if (!endpointId) {
        throw new Error('No world endpoint available for closet reset.');
      }
      const restarted = await closetStartSessionForEndpoint(endpointId);
      return {
        ok: true,
        session_id: restarted.session_id,
        latest_lobby_sequence: restarted.latest_lobby_sequence,
        message: `Closet session reset. New session: ${restarted.session_id || state.closetSessionId}`,
      };
    }

    const resourceHelpMatch = normalizedInput.match(/^__resource_help__\s+([A-Za-z][A-Za-z0-9_-]*)$/);
    if (resourceHelpMatch) {
      const resource = String(resourceHelpMatch[1] || '').trim().toLowerCase();
      if (resource === 'document') {
        return {
          ok: true,
          message: 'document commands: document.help | document.show | document.ma.<field> show | document.publish [ipns_private_key_base64] | document.republish [ipns_private_key_base64] | document.apply [ipns_private_key_base64]'
        };
      }
      return {
        ok: false,
        message: `unsupported resource help: ${resource}`
      };
    }

    const resourceShowMatch = normalizedInput.match(/^__resource_show__\s+([A-Za-z][A-Za-z0-9_-]*)(?:\s+(show|peek))?$/);
    if (resourceShowMatch) {
      const resource = String(resourceShowMatch[1] || '').trim().toLowerCase();
      if (resource === 'document') {
        return {
          ok: true,
          message: renderDocumentShowMessage('')
        };
      }
      if (resource === 'avatar') {
        return {
          ok: true,
          message: await renderAvatarPeekMessage('', String(state.closetEndpointId || '').trim())
        };
      }
      throw new Error(`unsupported show resource: ${resource}`);
    }

    const resourcePathShowMatch = normalizedInput.match(/^__resource_show_path__\s+([A-Za-z][A-Za-z0-9_-]*)\s+(show|peek)\s+(.+)$/);
    if (resourcePathShowMatch) {
      const resource = String(resourcePathShowMatch[1] || '').trim().toLowerCase();
      const path = String(resourcePathShowMatch[3] || '').trim();
      if (resource === 'document') {
        return {
          ok: true,
          message: renderDocumentShowMessage(path)
        };
      }
      if (resource === 'avatar') {
        return {
          ok: true,
          message: await renderAvatarPeekMessage(path, String(state.closetEndpointId || '').trim())
        };
      }
      throw new Error(`unsupported show resource path: ${resource}`);
    }

    const applyMatch = normalizedInput.match(/^(apply|citizen)(?:\s+(.+))?$/i);
    if (applyMatch) {
      state.closetPendingIpnsPrivateKeyB64 = String(applyMatch[2] || '').trim();
    }
    const endpointId = String(state.closetEndpointId || '').trim();
    if (!endpointId) {
      throw new Error('No world endpoint available for closet command.');
    }
    if (applyMatch && !String(state.closetSessionDid || '').trim()) {
      const response = parseClosetResponse(
        await closet_submit_citizenship(
          endpointId,
          state.closetSessionId,
          state.closetPendingIpnsPrivateKeyB64,
          desiredFragmentForCloset()
        )
      );
      if (!response.ok) {
        throw new Error(response.message || 'Closet citizenship request failed.');
      }
      state.closetLobbySeq = Number(response.latest_lobby_sequence || state.closetLobbySeq || 0);
      return response;
    }

    if (applyMatch && String(state.closetSessionDid || '').trim()) {
      const response = parseClosetResponse(
        await closet_command(endpointId, state.closetSessionId, normalizedInput)
      );
      if (!response.ok) {
        const message = String(response.message || '').trim();
        if (/ipns publish failed|did document is not published yet/i.test(message)) {
          state.didPublishError = message;
          const expiresAt = pendingDidPublishExpiresAt(state.closetSessionDid) || markDidPublishPending(state.closetSessionDid);
          const secondsLeft = Math.max(1, Math.ceil((expiresAt - Date.now()) / 1000));
          return {
            ok: true,
            latest_lobby_sequence: state.closetLobbySeq,
            message: `${message}\nBackground publish is still pending (TTL ${secondsLeft}s). You can continue and use go out.`
          };
        }
        throw new Error(message || 'Closet apply failed.');
      }
      state.closetLobbySeq = Number(response.latest_lobby_sequence || state.closetLobbySeq || 0);
      return response;
    }

    const resourceApplyMatch = normalizedInput.match(/^__resource_apply__\s+([A-Za-z][A-Za-z0-9_-]*)(?:\s+(.+))?$/);
    if (resourceApplyMatch) {
      const resource = String(resourceApplyMatch[1] || '').trim().toLowerCase();
      if (resource !== 'document') {
        throw new Error(`unsupported apply resource: ${resource}`);
      }
      if (!String(state.closetSessionDid || '').trim()) {
        throw new Error('actor identity does not exist in this world yet; run apply first');
      }
      if (!state.passphrase || !state.encryptedBundle || !state.identity?.document_json) {
        throw new Error('No local DID document is available to publish. Unlock identity first.');
      }

      let updated = state.identity;
      if (!state.inboxEndpointId) {
        state.inboxEndpointId = await start_inbox_listener(state.passphrase, state.encryptedBundle);
      }
      if (state.inboxEndpointId) {
        updated = JSON.parse(
          set_bundle_transports(state.passphrase, state.encryptedBundle, state.inboxEndpointId)
        );
        const preferredLanguageOrder = String(state.languageOrder || '').trim();
        if (preferredLanguageOrder) {
          updated = JSON.parse(
            set_bundle_language(state.passphrase, updated.encrypted_bundle, preferredLanguageOrder)
          );
        }
        state.identity = updated;
        state.encryptedBundle = updated.encrypted_bundle;
        const bundleEl = byId('bundle-text');
        if (bundleEl) {
          bundleEl.value = updated.encrypted_bundle;
        }
        if (isValidAliasName(state.aliasName || '')) {
          saveIdentityRecord(state.aliasName, updated.encrypted_bundle);
        }
      }

      const providedKey = String(resourceApplyMatch[2] || '').trim();
      const response = parseClosetResponse(
        await closet_publish_did_document(
          endpointId,
          state.closetSessionId,
          updated.document_json,
          providedKey,
          desiredFragmentForCloset()
        )
      );
      if (!response.ok) {
        const message = String(response.message || '').trim();
        if (/ipns publish failed|did document is not published yet/i.test(message)) {
          state.didPublishError = message;
          const didForPending = String(state.identity?.did || state.closetSessionDid || '').trim();
          const expiresAt = pendingDidPublishExpiresAt(didForPending) || markDidPublishPending(didForPending);
          const secondsLeft = Math.max(1, Math.ceil((expiresAt - Date.now()) / 1000));
          return {
            ok: true,
            latest_lobby_sequence: state.closetLobbySeq,
            message: `${message}\nBackground publish is still pending (TTL ${secondsLeft}s). You can continue and use go out.`
          };
        }
        throw new Error(message || 'Closet DID publish failed.');
      }
      const didForCache = didRoot(String(response.did || state.identity?.did || ''));
      if (didForCache) {
        state.didDocCache.delete(didForCache);
        clearDidPublishPending(didForCache);
      }
      return response;
    }

    const response = parseClosetResponse(
      await closet_command(endpointId, state.closetSessionId, normalizedInput)
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
