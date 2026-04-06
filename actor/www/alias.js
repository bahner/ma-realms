import { isMaDid } from './did.js';

export function isValidAliasName(aliasName) {
  return /^[a-z0-9_-]{2,32}$/i.test(String(aliasName || '').trim());
}

export function isPrintableAliasLabel(label) {
  const value = String(label || '').trim();
  if (!value) return false;
  // Allow any printable Unicode label, excluding control/format/surrogate chars and spaces.
  if (/[\p{Cc}\p{Cf}\p{Cs}\s]/u.test(value)) return false;
  return value.length <= 64;
}

export function createAliasFlow({
  state,
  identityStore,
  aliasBookKey,
  tabAliasKey,
  lastAliasKey,
  aliasNormalizeEndpointId,
  aliasFindDidByEndpoint,
  aliasFindAliasForAddress,
  aliasResolveInput,
  aliasHumanizeIdentifier,
  aliasHumanizeText,
  roomDidCacheTtlMs,
}) {
  function saveAliasBook() {
    identityStore.saveAliasBook(aliasBookKey, state.aliasBook);
  }

  function loadAliasBook() {
    return identityStore.loadAliasBook(aliasBookKey);
  }

  function setActiveAlias(aliasName) {
    identityStore.setActiveAlias(aliasName, tabAliasKey, lastAliasKey);
  }

  function resolveInitialAlias() {
    return identityStore.resolveInitialAlias(tabAliasKey, lastAliasKey);
  }

  function loadAliasDraft(aliasName, options = {}) {
    const persistActive = options.persistActive !== false;
    const normalized = String(aliasName || '').trim();
    const byId = options.byId;
    const onNewPhrase = options.onNewPhrase;
    const normalizeLanguageOrder = options.normalizeLanguageOrder;
    const setUiLanguage = options.setUiLanguage;
    const defaultLanguageOrder = options.defaultLanguageOrder;
    const defaultUiLang = options.defaultUiLang;

    if (typeof byId !== 'function') {
      throw new Error('loadAliasDraft requires options.byId');
    }
    if (typeof onNewPhrase !== 'function') {
      throw new Error('loadAliasDraft requires options.onNewPhrase');
    }
    if (typeof normalizeLanguageOrder !== 'function') {
      throw new Error('loadAliasDraft requires options.normalizeLanguageOrder');
    }
    if (typeof setUiLanguage !== 'function') {
      throw new Error('loadAliasDraft requires options.setUiLanguage');
    }

    if (!normalized) {
      byId('bundle-text').value = '';
      if (!byId('recovery-phrase').value.trim()) {
        onNewPhrase();
      }
      return;
    }

    if (persistActive) {
      setActiveAlias(normalized);
    }

    const record = identityStore.resolveIdentityRecord(normalized);
    byId('bundle-text').value = record?.encryptedBundle || '';
    state.languageOrder = normalizeLanguageOrder(record?.language || defaultLanguageOrder);
    const languageInput = byId('language-order');
    if (languageInput) {
      languageInput.value = state.languageOrder;
    }
    setUiLanguage(record?.uiLang || defaultUiLang);

    if (!byId('recovery-phrase').value.trim()) {
      onNewPhrase();
    }
  }

  function roomDidLookupCacheKey(token) {
    if (!state.currentHome) return '';
    const endpoint = String(state.currentHome.endpointId || '').trim();
    const room = String(state.currentHome.room || '').trim();
    const normalized = String(token || '').trim().replace(/^@+/, '').toLowerCase();
    if (!endpoint || !room || !normalized) return '';
    return `${endpoint}::${room}::${normalized}`;
  }

  function getCachedRoomDidLookup(token) {
    const key = roomDidLookupCacheKey(token);
    if (!key) return '';
    const entry = state.roomDidLookupCache.get(key);
    if (!entry || typeof entry !== 'object') {
      return '';
    }
    const now = Date.now();
    if (!entry.expiresAt || entry.expiresAt <= now) {
      state.roomDidLookupCache.delete(key);
      return '';
    }
    return String(entry.did || '').trim();
  }

  function cacheRoomDidLookup(token, did) {
    const key = roomDidLookupCacheKey(token);
    const normalizedDid = String(did || '').trim();
    if (!key || !isMaDid(normalizedDid)) return;
    state.roomDidLookupCache.set(key, {
      did: normalizedDid,
      expiresAt: Date.now() + roomDidCacheTtlMs
    });
  }

  function dropCachedRoomDidLookup(token) {
    const key = roomDidLookupCacheKey(token);
    if (!key) return;
    state.roomDidLookupCache.delete(key);
  }

  function normalizeEndpointId(address) {
    const normalized = aliasNormalizeEndpointId(address);
    if (!normalized) {
      return '';
    }
    return normalized;
  }

  function findDidByEndpoint(endpointLike) {
    try {
      return aliasFindDidByEndpoint(
        String(endpointLike || ''),
        JSON.stringify(state.didEndpointMap || {})
      );
    } catch {
      return '';
    }
  }

  function findAliasForAddress(address) {
    try {
      return aliasFindAliasForAddress(
        String(address || ''),
        JSON.stringify(state.aliasBook || {})
      );
    } catch {
      return '';
    }
  }

  function resolveAliasInput(value) {
    const raw = String(value || '').trim();
    if (!raw) {
      return '';
    }

    try {
      return aliasResolveInput(raw, JSON.stringify(state.aliasBook || {}));
    } catch {
      return raw;
    }
  }

  function humanizeIdentifier(value) {
    try {
      return aliasHumanizeIdentifier(
        String(value || ''),
        JSON.stringify(state.aliasBook || {})
      );
    } catch {
      return String(value || '').trim();
    }
  }

  function humanizeText(text) {
    try {
      return aliasHumanizeText(
        String(text || ''),
        JSON.stringify(state.aliasBook || {})
      );
    } catch {
      return String(text || '');
    }
  }

  return {
    saveAliasBook,
    loadAliasBook,
    setActiveAlias,
    resolveInitialAlias,
    loadAliasDraft,
    roomDidLookupCacheKey,
    getCachedRoomDidLookup,
    cacheRoomDidLookup,
    dropCachedRoomDidLookup,
    normalizeEndpointId,
    findDidByEndpoint,
    findAliasForAddress,
    resolveAliasInput,
    humanizeIdentifier,
    humanizeText,
  };
}
