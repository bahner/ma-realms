export function createIdentityStore({
  storagePrefix,
  legacy,
  isValidAliasName
}) {
  const IDENTITY_PREFIX = `${storagePrefix}.identity.`;

  function identityRecordKey(aliasName) {
    return `${IDENTITY_PREFIX}${String(aliasName || '').trim().toLowerCase()}`;
  }

  function readStoredJson(key) {
    try {
      const raw = localStorage.getItem(key);
      if (!raw) return null;
      const parsed = JSON.parse(raw);
      return parsed && typeof parsed === 'object' ? parsed : null;
    } catch {
      return null;
    }
  }

  function saveAliasBook(key, aliasBook) {
    localStorage.setItem(key, JSON.stringify(aliasBook));
  }

  function loadAliasBook(key) {
    try {
      const raw = localStorage.getItem(key);
      if (!raw) return {};
      const parsed = JSON.parse(raw);
      if (!parsed || typeof parsed !== 'object') return {};
      return parsed;
    } catch {
      return {};
    }
  }

  function saveIdentityRecord(aliasName, encryptedBundle, language) {
    if (!isValidAliasName(aliasName)) {
      return;
    }

    localStorage.setItem(
      identityRecordKey(aliasName),
      JSON.stringify({
        aliasName,
        encryptedBundle,
        language: String(language || '').trim()
      })
    );
  }

  function loadIdentityRecord(aliasName) {
    if (!isValidAliasName(aliasName)) {
      return null;
    }

    const parsed = readStoredJson(identityRecordKey(aliasName));
    if (!parsed) {
      return null;
    }

    return {
      aliasName: typeof parsed.aliasName === 'string' ? parsed.aliasName : aliasName,
      encryptedBundle: typeof parsed.encryptedBundle === 'string' ? parsed.encryptedBundle : '',
      language: typeof parsed.language === 'string' ? parsed.language : ''
    };
  }

  function loadLegacyIdentityRecord(aliasName) {
    const legacyAlias = localStorage.getItem(legacy.aliasKey);
    if (!isValidAliasName(aliasName) || legacyAlias !== aliasName) {
      return null;
    }

    return {
      aliasName,
      encryptedBundle: localStorage.getItem(legacy.bundleKey) || '',
      language: ''
    };
  }

  function resolveIdentityRecord(aliasName) {
    return loadIdentityRecord(aliasName) || loadLegacyIdentityRecord(aliasName);
  }

  function findAnyIdentityRecord() {
    const keys = [];
    for (let i = 0; i < localStorage.length; i += 1) {
      const key = localStorage.key(i);
      if (key && key.startsWith(IDENTITY_PREFIX)) {
        keys.push(key);
      }
    }

    keys.sort();
    for (const key of keys) {
      const parsed = readStoredJson(key);
      if (!parsed) continue;
      const aliasName = String(parsed.aliasName || '').trim();
      if (!isValidAliasName(aliasName)) continue;
      const encryptedBundle = typeof parsed.encryptedBundle === 'string' ? parsed.encryptedBundle : '';
      if (!encryptedBundle.trim()) continue;
      return {
        aliasName,
        encryptedBundle,
        language: typeof parsed.language === 'string' ? parsed.language : ''
      };
    }

    const legacyAlias = String(localStorage.getItem(legacy.aliasKey) || '').trim();
    const legacyBundle = String(localStorage.getItem(legacy.bundleKey) || '').trim();
    if (isValidAliasName(legacyAlias) && legacyBundle) {
      return {
        aliasName: legacyAlias,
        encryptedBundle: legacyBundle,
        language: ''
      };
    }

    return null;
  }

  function scrubStoredRecoveryPhrases() {
    const keys = [];
    for (let i = 0; i < localStorage.length; i += 1) {
      const key = localStorage.key(i);
      if (key && key.startsWith(IDENTITY_PREFIX)) {
        keys.push(key);
      }
    }

    for (const key of keys) {
      const parsed = readStoredJson(key);
      if (!parsed || typeof parsed !== 'object') {
        continue;
      }
      if (!Object.prototype.hasOwnProperty.call(parsed, 'recoveryPhrase')) {
        continue;
      }
      delete parsed.recoveryPhrase;
      localStorage.setItem(key, JSON.stringify(parsed));
    }

    localStorage.removeItem(legacy.recoveryPhraseKey);
  }

  function setActiveAlias(aliasName, tabAliasKey, lastAliasKey) {
    const normalized = String(aliasName || '').trim();
    if (!normalized) {
      sessionStorage.removeItem(tabAliasKey);
      return;
    }

    sessionStorage.setItem(tabAliasKey, normalized);
    localStorage.setItem(lastAliasKey, normalized);
  }

  function resolveInitialAlias(tabAliasKey, lastAliasKey) {
    const urlAlias = new URLSearchParams(window.location.search).get('alias');
    if (isValidAliasName(urlAlias)) {
      return urlAlias.trim();
    }

    const tabAlias = sessionStorage.getItem(tabAliasKey);
    if (isValidAliasName(tabAlias)) {
      return tabAlias.trim();
    }

    const lastAlias = localStorage.getItem(lastAliasKey);
    if (isValidAliasName(lastAlias)) {
      return lastAlias.trim();
    }

    const legacyAlias = localStorage.getItem(legacy.aliasKey);
    if (isValidAliasName(legacyAlias)) {
      return legacyAlias.trim();
    }

    return '';
  }

  return {
    saveAliasBook,
    loadAliasBook,
    saveIdentityRecord,
    resolveIdentityRecord,
    findAnyIdentityRecord,
    scrubStoredRecoveryPhrases,
    setActiveAlias,
    resolveInitialAlias
  };
}

export function createIdentityLineFlow({ updateLocationContext }) {
  function updateIdentityLine() {
    updateLocationContext();
  }

  return {
    updateIdentityLine,
  };
}

export function createIdentityFlow({
  byId,
  state,
  isValidAliasName,
  saveIdentityRecord,
  setSetupStatus,
  setActiveAlias,
  getApiBase,
  normalizeIpfsGatewayBase,
  apiStorageKey,
  createIdentity,
  unlockIdentity,
  ensureBundleIrohSecret,
  setBundleLanguage,
  normalizeLanguageOrder,
  generateBip39Phrase,
  normalizeBip39Phrase,
  defaultLanguageOrder,
  defaultUiLang,
  setUiLanguage,
  setCurrentPublishInfo,
  showChat,
  restoreActiveHomeAfterUnlock,
  appendMessage,
  saveActiveHomeSnapshot,
  stopHomeEventPolling,
  disconnectWorld,
  clearRoomPresence,
  showSetup,
  showLockOverlay,
}) {
  function applyBundleLanguagePreference(languageOrder) {
    const normalized = normalizeLanguageOrder(languageOrder || defaultLanguageOrder);
    state.languageOrder = normalized;

    if (!state.passphrase || !state.encryptedBundle) {
      return false;
    }

    const updated = JSON.parse(setBundleLanguage(state.passphrase, state.encryptedBundle, normalized));
    state.identity = updated;
    state.encryptedBundle = updated.encrypted_bundle;
    const bundleEl = byId('bundle-text');
    if (bundleEl) {
      bundleEl.value = updated.encrypted_bundle;
    }
    return true;
  }

  function generateRecoveryPhrase(wordCount = 12) {
    return generateBip39Phrase(wordCount);
  }

  function normalizeRecoveryPhrase(input) {
    const value = String(input || '').trim();
    if (!value) {
      return '';
    }
    return normalizeBip39Phrase(value);
  }

  function resolveRecoveryPhraseFromInput() {
    const raw = byId('recovery-phrase').value;
    const trimmed = String(raw || '').trim();
    if (!trimmed) {
      return generateRecoveryPhrase(12);
    }
    return normalizeRecoveryPhrase(trimmed);
  }

  function validateSetupInputs(requireBundle) {
    const aliasName = byId('alias-name').value.trim();
    const passphrase = byId('passphrase').value;
    const bundle = byId('bundle-text').value.trim();
    const languageOrder = normalizeLanguageOrder(byId('language-order')?.value || defaultLanguageOrder);

    if (!isValidAliasName(aliasName)) {
      throw new Error('Alias must be 2-32 chars using letters, numbers, underscore, or dash.');
    }
    if (passphrase.length < 8) {
      throw new Error('Passphrase must be at least 8 characters.');
    }
    if (requireBundle && !bundle) {
      throw new Error('Provide an encrypted bundle to unlock.');
    }

    return { aliasName, passphrase, bundle, languageOrder };
  }

  async function onCreateIdentity() {
    setSetupStatus('Creating identity...');
    try {
      const { aliasName, passphrase, languageOrder } = validateSetupInputs(false);
      localStorage.setItem(apiStorageKey, normalizeIpfsGatewayBase(getApiBase()));
      setActiveAlias(aliasName);

      const created = JSON.parse(createIdentity(passphrase, aliasName));
      const result = JSON.parse(ensureBundleIrohSecret(passphrase, created.encrypted_bundle));

      state.identity = result;
      state.encryptedBundle = result.encrypted_bundle;
      state.passphrase = passphrase;
      state.aliasName = aliasName;
      applyBundleLanguagePreference(languageOrder);
      setUiLanguage(defaultUiLang);
      setCurrentPublishInfo({ ipns: result.ipns || '' });

      byId('bundle-text').value = result.encrypted_bundle;

      const phrase = resolveRecoveryPhraseFromInput();
      byId('recovery-phrase').value = phrase;
      saveIdentityRecord(aliasName, result.encrypted_bundle);

      setSetupStatus('Identity created and unlocked.');
      showChat();
      restoreActiveHomeAfterUnlock().catch((err) => {
        appendMessage('system', `Restore failed: ${err instanceof Error ? err.message : String(err)}`);
      });
    } catch (error) {
      setSetupStatus(error instanceof Error ? error.message : String(error));
    }
  }

  async function onUnlockIdentity() {
    setSetupStatus('Unlocking bundle...');
    try {
      const { aliasName, passphrase, bundle, languageOrder } = validateSetupInputs(true);
      localStorage.setItem(apiStorageKey, normalizeIpfsGatewayBase(getApiBase()));
      setActiveAlias(aliasName);

      JSON.parse(unlockIdentity(passphrase, bundle));
      const updated = JSON.parse(ensureBundleIrohSecret(passphrase, bundle));

      state.identity = updated;
      state.encryptedBundle = updated.encrypted_bundle;
      state.passphrase = passphrase;
      state.aliasName = aliasName;
      applyBundleLanguagePreference(languageOrder);
      setUiLanguage(defaultUiLang);
      setCurrentPublishInfo({ ipns: updated.ipns || '' });

      byId('bundle-text').value = updated.encrypted_bundle;

      const phrase = resolveRecoveryPhraseFromInput();
      byId('recovery-phrase').value = phrase;
      saveIdentityRecord(aliasName, updated.encrypted_bundle);

      setSetupStatus('Bundle unlocked.');
      showChat();
      restoreActiveHomeAfterUnlock().catch((err) => {
        appendMessage('system', `Restore failed: ${err instanceof Error ? err.message : String(err)}`);
      });
    } catch (error) {
      setSetupStatus(error instanceof Error ? error.message : String(error));
    }
  }

  function onNewPhrase() {
    const phrase = generateRecoveryPhrase(12);
    byId('recovery-phrase').value = phrase;

    const aliasName = byId('alias-name').value.trim();
    const bundle = byId('bundle-text').value.trim();
    state.languageOrder = normalizeLanguageOrder(byId('language-order')?.value || defaultLanguageOrder);
    if (isValidAliasName(aliasName)) {
      saveIdentityRecord(aliasName, bundle);
    }
  }

  function lockSession() {
    saveActiveHomeSnapshot();
    stopHomeEventPolling();
    disconnectWorld().catch(() => {});
    state.identity = null;
    state.encryptedBundle = '';
    state.passphrase = '';
    state.currentHome = null;
    state.didDocCache.clear();
    clearRoomPresence();
    byId('transcript').innerHTML = '';
    setSetupStatus('Session locked. Bundle remains stored unless removed manually.');
    showSetup();
    showLockOverlay();
  }

  return {
    normalizeLanguageOrder,
    applyBundleLanguagePreference,
    onCreateIdentity,
    onUnlockIdentity,
    onNewPhrase,
    lockSession,
  };
}
