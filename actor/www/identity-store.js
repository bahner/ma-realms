export function createIdentityStore({
  storagePrefix,
  legacy,
  isValidAliasName,
  normalizeLocale
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

  function saveIdentityRecord(aliasName, encryptedBundle, locale) {
    if (!isValidAliasName(aliasName)) {
      return;
    }

    localStorage.setItem(
      identityRecordKey(aliasName),
      JSON.stringify({
        aliasName,
        encryptedBundle,
        locale: normalizeLocale(locale)
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
      locale: normalizeLocale(parsed.locale)
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
      locale: legacy.defaultLocale
    };
  }

  function resolveIdentityRecord(aliasName) {
    return loadIdentityRecord(aliasName) || loadLegacyIdentityRecord(aliasName);
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
    scrubStoredRecoveryPhrases,
    setActiveAlias,
    resolveInitialAlias
  };
}
