import {
  create_identity_with_ipns,
  set_bundle_language,
  set_bundle_transports,
  set_bundle_updated_for_send,
  start_inbox_listener,
  closet_start,
  closet_command,
  closet_submit_citizenship,
  closet_publish_did_document,
  validate_did_document,
  validate_identity_bundle_keys,
} from './pkg/ma_actor.js';
import { DID_MA_PREFIX, isMaDid } from './did.js';

export function createClosetFlow({
  state,
  byId,
  appendMessage,
  didRoot,
  didPublishPendingTtlMs = 5 * 60 * 1000,
  closetRpcTimeoutMs = 8000,
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

    if (/^avatar$/i.test(raw)) {
      return '__resource_show__ avatar peek';
    }

    if (/^actor$/i.test(raw)) {
      return '__resource_show__ document peek';
    }

    // Dot-only scoped syntax, for example avatar.name bahner and actor.apply <key>.
    // Space-scoped compatibility forms (like "avatar apply") are intentionally removed.
    const dottedPathPeek = raw.match(/^([A-Za-z][A-Za-z0-9_-]*)\.([A-Za-z0-9_.-]+)\.(show|peek)$/i);
    if (dottedPathPeek) {
      const scope = String(dottedPathPeek[1] || '').trim().toLowerCase();
      const path = String(dottedPathPeek[2] || '').trim();
      const mode = String(dottedPathPeek[3] || 'peek').toLowerCase();
      const resolvedScope = scope === 'actor' ? 'document' : scope;
      return `__resource_show_path__ ${resolvedScope} ${mode} ${path}`;
    }

    const dotted = raw.match(/^([A-Za-z][A-Za-z0-9_-]*)\.([A-Za-z][A-Za-z0-9_-]*)(?:\s+(.*))?$/);
    if (dotted) {
      const scope = String(dotted[1] || '').trim().toLowerCase();
      const op = String(dotted[2] || '').trim().toLowerCase();
      const args = String(dotted[3] || '').trim();
      const resolvedScope = scope === 'actor' ? 'document' : scope;

      const dotTable = {
        avatar: {
          help: () => 'help',
          show: () => '__resource_show__ avatar peek',
          peek: () => '__resource_show__ avatar peek',
          apply: () => (args ? `apply ${args}` : 'apply'),
          name: () => (args ? `name ${args}` : 'name'),
          description: () => (args ? `description ${args}` : 'description'),
        },
        document: {
          help: () => `__resource_help__ ${resolvedScope}`,
          show: () => `__resource_show__ ${resolvedScope} show`,
          peek: () => `__resource_show__ ${resolvedScope} peek`,
          validate: () => `__resource_validate__ ${resolvedScope}`,
          apply: () => (args ? `__resource_apply__ ${resolvedScope} ${args}` : `__resource_apply__ ${resolvedScope}`),
          publish: () => (args ? `__resource_apply__ ${resolvedScope} ${args}` : `__resource_apply__ ${resolvedScope}`),
          republish: () => (args ? `__resource_apply__ ${resolvedScope} ${args}` : `__resource_apply__ ${resolvedScope}`),
          id: () => `__resource_show_path__ ${resolvedScope} peek id`,
        },
      };

      const resolver = dotTable?.[resolvedScope]?.[op];
      if (typeof resolver === 'function') {
        return resolver();
      }

      return null;
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

  function collectVerificationRefs(value) {
    if (!value) return [];
    if (typeof value === 'string') {
      const trimmed = String(value || '').trim();
      return trimmed ? [trimmed] : [];
    }
    if (Array.isArray(value)) {
      return value
        .map((entry) => {
          if (typeof entry === 'string') return entry.trim();
          if (entry && typeof entry === 'object') return String(entry.id || '').trim();
          return '';
        })
        .filter(Boolean);
    }
    if (value && typeof value === 'object') {
      const id = String(value.id || '').trim();
      return id ? [id] : [];
    }
    return [];
  }

  function assertPublishableDidDocument(documentJson, contextLabel = 'document publish') {
    const raw = String(documentJson || '').trim();
    if (!raw) {
      throw new Error(`${contextLabel}: missing DID document JSON`);
    }

    // Primary validation path: use did:ma native validate()/verify().
    try {
      validate_did_document(raw);
    } catch (error) {
      throw new Error(`${contextLabel}: ${error instanceof Error ? error.message : String(error)}`);
    }

    let document;
    try {
      document = JSON.parse(raw);
    } catch (error) {
      throw new Error(`${contextLabel}: DID document is not valid JSON (${error instanceof Error ? error.message : String(error)})`);
    }

    const did = String(document?.id || '').trim();
    if (!isMaDid(did)) {
      throw new Error(`${contextLabel}: document.id must be did:ma:*`);
    }

    const verificationMethods = Array.isArray(document?.verificationMethod)
      ? document.verificationMethod
      : [];
    if (!verificationMethods.length) {
      throw new Error(`${contextLabel}: verificationMethod is missing`);
    }

    const methodIds = new Set();
    let methodsWithPublicKeys = 0;
    for (const method of verificationMethods) {
      const id = String(method?.id || '').trim();
      if (id) {
        methodIds.add(id);
        if (id.startsWith('#')) {
          methodIds.add(`${did}${id}`);
        }
      }

      const hasPublicKey = Boolean(
        String(method?.publicKeyMultibase || method?.public_key_multibase || '').trim()
        || String(method?.publicKeyBase58 || method?.public_key_base58 || '').trim()
        || method?.publicKeyJwk
        || method?.public_key_jwk
      );
      if (hasPublicKey) {
        methodsWithPublicKeys += 1;
      }
    }
    if (methodsWithPublicKeys === 0) {
      throw new Error(`${contextLabel}: verificationMethod entries contain no public keys`);
    }

    const assertionRefs = collectVerificationRefs(document?.assertionMethod);
    if (!assertionRefs.length) {
      throw new Error(`${contextLabel}: assertionMethod is missing`);
    }
    for (const ref of assertionRefs) {
      if (!methodIds.has(ref)) {
        throw new Error(`${contextLabel}: assertionMethod reference '${ref}' has no matching verificationMethod.id`);
      }
    }

    const keyAgreementRefs = collectVerificationRefs(document?.keyAgreement);
    if (!keyAgreementRefs.length) {
      throw new Error(`${contextLabel}: keyAgreement is missing`);
    }
    for (const ref of keyAgreementRefs) {
      if (!methodIds.has(ref)) {
        throw new Error(`${contextLabel}: keyAgreement reference '${ref}' has no matching verificationMethod.id`);
      }
    }

    const ma = document?.ma && typeof document.ma === 'object' ? document.ma : null;
    const currentInbox = String(ma?.currentInbox || ma?.current_inbox || '').trim();
    if (!currentInbox) {
      throw new Error(`${contextLabel}: ma.currentInbox is missing`);
    }
    const transports = Array.isArray(ma?.transports) ? ma.transports : [];
    if (!transports.length) {
      throw new Error(`${contextLabel}: ma.transports is missing`);
    }
    if (!transports.some((entry) => String(entry || '').trim() === currentInbox)) {
      throw new Error(`${contextLabel}: ma.currentInbox must be included in ma.transports`);
    }

    return document;
  }

  function assertPublishableIdentityKeys(contextLabel = 'document publish') {
    if (!state.passphrase || !state.encryptedBundle) {
      throw new Error(`${contextLabel}: missing passphrase or encrypted bundle for key validation`);
    }
    try {
      validate_identity_bundle_keys(state.passphrase, state.encryptedBundle);
    } catch (error) {
      throw new Error(`${contextLabel}: ${error instanceof Error ? error.message : String(error)}`);
    }
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
      await invokeClosetRpc('closet show', () =>
        closet_command(endpointId, state.closetSessionId, 'show')
      )
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
    let snapshot = null;
    if (endpointId) {
      snapshot = await fetchClosetProfileSnapshot(endpointId);
    } else {
      snapshot = state.closetProfile || null;
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

  function isClosetTransportConnectionLost(message) {
    const text = String(message || '').toLowerCase();
    return (
      text.includes('closet request failed') && text.includes('connection lost')
      || text.includes('connection lost')
      || text.includes('endpoint.connect() failed')
      || text.includes('connection.open_bi() failed')
      || text.includes('broken pipe')
      || text.includes('timed out') && text.includes('closet')
    );
  }

  function resetClosetSessionState() {
    state.closetSessionId = '';
    state.closetSessionDid = '';
    state.closetLobbySeq = 0;
    state.closetSessionFragment = '';
    state.closetProfile = null;
  }

  async function invokeClosetRpc(operationName, invoke) {
    let timeoutId = null;
    try {
      const timeoutMs = Math.max(1000, Number(closetRpcTimeoutMs || 0));
      const timeoutPromise = new Promise((_, reject) => {
        timeoutId = setTimeout(() => {
          reject(new Error(`closet rpc timed out after ${timeoutMs}ms`));
        }, timeoutMs);
      });

      const result = await Promise.race([
        invoke(),
        timeoutPromise,
      ]);

      return result;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (isClosetTransportConnectionLost(message)) {
        resetClosetSessionState();
        throw new Error(
          `Lost closet transport while running ${operationName} (${message}). The world service is likely stopped/restarted. Run go <world> to reconnect.`
        );
      }
      throw error;
    } finally {
      if (timeoutId) {
        clearTimeout(timeoutId);
      }
    }
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

  function stampBundleLifecycleForClosetSend() {
    if (!state.passphrase || !state.encryptedBundle) {
      return;
    }
    const updated = JSON.parse(
      set_bundle_updated_for_send(state.passphrase, state.encryptedBundle)
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

        stampBundleLifecycleForClosetSend();
        updated = state.identity || updated;
        assertPublishableIdentityKeys('document publish');
        assertPublishableDidDocument(updated.document_json, 'document publish');

        const published = parseClosetResponse(
          await invokeClosetRpc('document publish', () =>
            closet_publish_did_document(
              endpointId,
              sessionId,
              updated.document_json,
              ipnsPrivateKeyBase64,
              desiredFragmentForCloset()
            )
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

      const assignedRoot = didRoot(assignedDid);
      const localRoot = didRoot(String(state.identity?.did || '').trim());
      const localAlreadyMatchesAssigned = Boolean(
        assignedRoot && localRoot && assignedRoot === localRoot
      );

      // Rebind only when the assigned DID root differs from the currently loaded local DID root.
      if (assignedDid && assignedDid !== previousSessionDid && !localAlreadyMatchesAssigned) {
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
    const response = parseClosetResponse(
      await invokeClosetRpc('closet start', () => closet_start(normalizedEndpoint))
    );
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
          message: 'document commands: document.help | document.show | document.validate | document.ma.<field> show | document.publish [ipns_private_key_base64] | document.republish [ipns_private_key_base64] | document.apply [ipns_private_key_base64]'
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

    const resourceValidateMatch = normalizedInput.match(/^__resource_validate__\s+([A-Za-z][A-Za-z0-9_-]*)$/);
    if (resourceValidateMatch) {
      const resource = String(resourceValidateMatch[1] || '').trim().toLowerCase();
      if (resource !== 'document') {
        throw new Error(`unsupported validate resource: ${resource}`);
      }
      if (!state.identity?.document_json) {
        throw new Error('No local DID document is available to validate. Unlock identity first.');
      }
      assertPublishableDidDocument(state.identity.document_json, 'document.validate');
      return {
        ok: true,
        latest_lobby_sequence: state.closetLobbySeq,
        message: 'document.validate ok: DID document is publishable (keys, refs, transports).'
      };
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
      // Idempotent apply: if local DID doc exists, try publish/update first.
      if (state.passphrase && state.encryptedBundle && state.identity?.document_json) {
        try {
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

          stampBundleLifecycleForClosetSend();
          updated = state.identity || updated;
          assertPublishableIdentityKeys('closet apply publish-existing');
          assertPublishableDidDocument(updated.document_json, 'closet apply publish-existing');

          const publishResponse = parseClosetResponse(
            await invokeClosetRpc('closet apply publish-existing', () =>
              closet_publish_did_document(
                endpointId,
                state.closetSessionId,
                updated.document_json,
                state.closetPendingIpnsPrivateKeyB64,
                desiredFragmentForCloset()
              )
            )
          );

          if (publishResponse.ok) {
            const didForCache = didRoot(String(publishResponse.did || state.identity?.did || ''));
            if (didForCache) {
              state.didDocCache.delete(didForCache);
              clearDidPublishPending(didForCache);
            }
            return publishResponse;
          }
        } catch (publishErr) {
          const detail = publishErr instanceof Error ? publishErr.message : String(publishErr);
          throw new Error(detail || 'Closet apply failed.');
        }
      }

      stampBundleLifecycleForClosetSend();
      const response = parseClosetResponse(
        await invokeClosetRpc('closet apply', () =>
          closet_submit_citizenship(
            endpointId,
            state.closetSessionId,
            state.closetPendingIpnsPrivateKeyB64,
            desiredFragmentForCloset()
          )
        )
      );
      if (!response.ok) {
        throw new Error(response.message || 'Closet citizenship request failed.');
      }
      state.closetLobbySeq = Number(response.latest_lobby_sequence || state.closetLobbySeq || 0);
      return response;
    }

    if (applyMatch && String(state.closetSessionDid || '').trim()) {
      if (state.passphrase && state.encryptedBundle && state.identity?.document_json) {
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

        stampBundleLifecycleForClosetSend();
        updated = state.identity || updated;
        assertPublishableIdentityKeys('closet apply publish-existing');
        assertPublishableDidDocument(updated.document_json, 'closet apply publish-existing');

        const response = parseClosetResponse(
          await invokeClosetRpc('closet apply publish-existing', () =>
            closet_publish_did_document(
              endpointId,
              state.closetSessionId,
              updated.document_json,
              state.closetPendingIpnsPrivateKeyB64,
              desiredFragmentForCloset()
            )
          )
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
        const didForCache = didRoot(String(response.did || state.identity?.did || state.closetSessionDid || ''));
        if (didForCache) {
          state.didDocCache.delete(didForCache);
          clearDidPublishPending(didForCache);
        }
        state.closetLobbySeq = Number(response.latest_lobby_sequence || state.closetLobbySeq || 0);
        return response;
      }

      const response = parseClosetResponse(
        await invokeClosetRpc('closet apply update', () =>
          closet_command(endpointId, state.closetSessionId, normalizedInput)
        )
      );
      if (!response.ok) {
        throw new Error(String(response.message || '').trim() || 'Closet apply failed.');
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
      stampBundleLifecycleForClosetSend();
      updated = state.identity || updated;
      assertPublishableIdentityKeys('document apply');
      assertPublishableDidDocument(updated.document_json, 'document apply');
      const response = parseClosetResponse(
        await invokeClosetRpc('document apply', () =>
          closet_publish_did_document(
            endpointId,
            state.closetSessionId,
            updated.document_json,
            providedKey,
            desiredFragmentForCloset()
          )
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
      await invokeClosetRpc('closet command', () =>
        closet_command(endpointId, state.closetSessionId, normalizedInput)
      )
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
