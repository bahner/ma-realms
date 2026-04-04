export function createDidRoot(aliasDidRoot) {
  return function didRoot(input) {
    return aliasDidRoot(String(input || ''));
  };
}

export const DID_MA_PREFIX = 'did:ma:';

export function isMaDid(value) {
  return String(value || '').trim().toLowerCase().startsWith(DID_MA_PREFIX);
}

export function isMaDidRoot(value) {
  const text = String(value || '').trim();
  return isMaDid(text) && !text.includes('#');
}

export function isMaDidTarget(value) {
  const text = String(value || '').trim();
  return isMaDid(text) && text.includes('#');
}

export function isConfiguredMaDidTarget(value) {
  return isMaDidTarget(value) && !isUnconfiguredDidTarget(value);
}

export function didToIpnsName(did, didRoot) {
  const root = didRoot(did);
  const prefix = DID_MA_PREFIX;
  if (!isMaDid(root)) {
    throw new Error(`Unsupported DID method for ${did}`);
  }
  return root.slice(prefix.length);
}

export function isUnconfiguredDidTarget(value) {
  const text = String(value || '').trim().toLowerCase();
  return text.startsWith('did:ma:unconfigured');
}

export function parseDidDocument(jsonText) {
  try {
    return JSON.parse(jsonText);
  } catch {
    return null;
  }
}

export function createDidRuntimeHelpers({
  state,
  didRoot,
  resolveAliasInput,
  findDidByEndpoint,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
  blocklistKey,
}) {
  function saveBlockedDidRoots() {
    if (!state.identity?.did) {
      return;
    }
    const key = blocklistKey(state.identity.did);
    if (!key) {
      return;
    }
    const entries = Array.from(state.blockedDidRoots || []).sort();
    localStorage.setItem(key, JSON.stringify(entries));
  }

  function resolveTargetDidRoot(token) {
    const key = String(token || '').trim();
    if (!key) {
      throw new Error('Usage: .block <did|alias|handle>');
    }
    const resolved = resolveAliasInput(key);
    const mappedDid = state.handleDidMap[key] || state.handleDidMap[resolved] || '';
    const candidate = isMaDid(key)
      ? key
      : (isMaDid(resolved) ? resolved : (mappedDid || findDidByEndpoint(resolved) || resolved));
    const root = didRoot(candidate);
    if (!isMaDid(root)) {
      throw new Error(`Could not resolve a did:ma target from '${key}'.`);
    }
    return root;
  }

  function primeDidLookupCacheFromWorldMessage(message) {
    const text = String(message || '').trim();
    if (!text) return;

    const bound = text.match(/\bbound\s+(@[A-Za-z0-9_-]+)\s*->\s*(did:ma:[^\s]+)(?:\s*\(object_id=([A-Za-z0-9_-]+)\))?/i);
    if (bound) {
      const alias = String(bound[1] || '').trim();
      const did = String(bound[2] || '').trim();
      const objectId = String(bound[3] || '').trim();
      if (isMaDid(did)) {
        cacheRoomDidLookup(alias, did);
        if (objectId) {
          cacheRoomDidLookup(objectId, did);
        }
        setActiveObjectTarget(alias, did, state.activeObjectTargetRequirement || 'none');
      }
      return;
    }

    const removed = text.match(/\bremoved\s+shortcut\s+(@[A-Za-z0-9_-]+)/i);
    if (removed) {
      const alias = String(removed[1] || '').trim();
      if (alias) {
        dropCachedRoomDidLookup(alias);
        clearActiveObjectTarget(alias);
      }
    }
  }

  function primeDidLookupCacheFromRoomObjectDids(roomObjectDids) {
    if (!roomObjectDids || typeof roomObjectDids !== 'object') {
      return;
    }
    for (const [objectIdRaw, didRaw] of Object.entries(roomObjectDids)) {
      const objectId = String(objectIdRaw || '').trim();
      const did = String(didRaw || '').trim();
      if (!objectId || !isMaDid(did)) {
        continue;
      }
      cacheRoomDidLookup(objectId, did);
    }
  }

  return {
    saveBlockedDidRoots,
    resolveTargetDidRoot,
    primeDidLookupCacheFromWorldMessage,
    primeDidLookupCacheFromRoomObjectDids,
  };
}

export function createDidDocFlow({
  state,
  didRoot,
  fetchGatewayTextByPath,
  logger,
  parseEnterDirective,
  extractWorldEndpointFromDidDoc,
  appendMessage,
  enterHome,
  didDocCacheTtlMs,
}) {
  async function fetchDidDocumentJsonByDid(did) {
    const rootDid = didRoot(did);
    const cached = state.didDocCache.get(rootDid);
    if (cached && Date.now() - cached.fetchedAt < didDocCacheTtlMs) {
      logger.log('did.cache', `hit for ${rootDid}`);
      return cached.documentJson;
    }

    logger.log('did.cache', `miss for ${rootDid}`);
    const ipns = didToIpnsName(rootDid, didRoot);
    const documentJson = await fetchGatewayTextByPath(`/ipns/${ipns}`);

    state.didDocCache.set(rootDid, {
      fetchedAt: Date.now(),
      documentJson
    });
    return documentJson;
  }

  async function autoFollowEnterDirective(message) {
    const targetDid = parseEnterDirective(message);
    if (!targetDid) {
      return;
    }

    const targetRoot = didRoot(targetDid);
    const roomFragment = targetDid.includes('#') ? targetDid.split('#')[1] : '';

    const targetDocJson = await fetchDidDocumentJsonByDid(targetRoot);
    const targetDoc = parseDidDocument(targetDocJson);
    const hintedWorldDid = typeof targetDoc?.ma?.world === 'string'
      ? targetDoc.ma.world
      : '';
    const worldDid = hintedWorldDid ? didRoot(hintedWorldDid) : targetRoot;

    const worldDocJson = await fetchDidDocumentJsonByDid(worldDid);
    const worldDoc = parseDidDocument(worldDocJson);
    const endpointId = extractWorldEndpointFromDidDoc(worldDoc);
    if (!endpointId) {
      throw new Error(`No iroh endpoint found in world DID document for ${worldDid}`);
    }

    appendMessage('system', `Following traveler route to ${targetDid}...`);
    await enterHome(endpointId, roomFragment || 'lobby');
  }

  return {
    didToIpnsName,
    fetchDidDocumentJsonByDid,
    parseDidDocument,
    autoFollowEnterDirective,
  };
}
