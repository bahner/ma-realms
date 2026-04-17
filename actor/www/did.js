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

export function didWithFragment(did, fragment) {
  const root = String(did || '').trim().split('#')[0];
  if (!root) return '';
  return `${root}#${fragment}`;
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

function normalizeMaType(document) {
  const kind = String(document?.ma?.type || document?.ma?.kind || '').trim().toLowerCase();
  return kind;
}

export async function resolveEndpointWithTypePolicy({
  targetRoot,
  targetDoc,
  fetchDidDocumentJsonByDid,
  didRoot,
  parseDidDocument,
  extractWorldEndpointFromDidDoc,
}) {
  const kind = normalizeMaType(targetDoc);
  const worldHintRaw = String(targetDoc?.ma?.world || '').trim();
  const worldHintRoot = worldHintRaw ? didRoot(worldHintRaw) : '';
  const localEndpoint = extractWorldEndpointFromDidDoc(targetDoc);

  const transportKinds = new Set(['world', 'agent', 'object']);
  const worldPointerKinds = new Set(['avatar', 'room', 'exit']);
  const knownType = Boolean(kind);

  if (transportKinds.has(kind) && localEndpoint) {
    return localEndpoint;
  }

  if (worldPointerKinds.has(kind) && worldHintRoot) {
    try {
      const worldDocJson = await fetchDidDocumentJsonByDid(worldHintRoot);
      const worldDoc = parseDidDocument(worldDocJson);
      const endpoint = extractWorldEndpointFromDidDoc(worldDoc);
      if (endpoint) {
        return endpoint;
      }
    } catch {
      // Fall through to local endpoint fallback.
    }
    return localEndpoint;
  }

  // Unknown type: prefer world pointer first (common case), then services.
  if (!knownType && worldHintRoot) {
    try {
      const worldDocJson = await fetchDidDocumentJsonByDid(worldHintRoot);
      const worldDoc = parseDidDocument(worldDocJson);
      const endpoint = extractWorldEndpointFromDidDoc(worldDoc);
      if (endpoint) {
        return endpoint;
      }
    } catch {
      // Fall through to local endpoint fallback.
    }
  }

  if (localEndpoint) {
    return localEndpoint;
  }

  // If we have a world hint and local endpoint did not resolve, try world once more.
  if (worldHintRoot) {
    const worldDocJson = await fetchDidDocumentJsonByDid(worldHintRoot);
    const worldDoc = parseDidDocument(worldDocJson);
    return extractWorldEndpointFromDidDoc(worldDoc);
  }

  return '';
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
}) {
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
  async function fetchDidDocumentJsonByDid(did, options = {}) {
    const forceRefresh = Boolean(options && options.forceRefresh);
    const localOnly = Boolean(options && options.localOnly);
    const timeoutMs = Number(options && options.timeoutMs);
    const rootDid = didRoot(did);
    const cached = forceRefresh ? null : state.didDocCache.get(rootDid);
    if (cached && Date.now() - cached.fetchedAt < didDocCacheTtlMs) {
      logger.log('did.cache', `hit for ${rootDid}`);
      return cached.documentJson;
    }

    logger.log('did.cache', `${forceRefresh ? 'refresh' : 'miss'} for ${rootDid}`);
    const ipns = didToIpnsName(rootDid, didRoot);
    const documentJson = await fetchGatewayTextByPath(`/ipns/${ipns}`, {
      localOnly,
      timeoutMs: Number.isFinite(timeoutMs) && timeoutMs > 0 ? timeoutMs : undefined,
    });

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
    const endpointId = await resolveEndpointWithTypePolicy({
      targetRoot,
      targetDoc,
      fetchDidDocumentJsonByDid,
      didRoot,
      parseDidDocument,
      extractWorldEndpointFromDidDoc,
    });
    if (!endpointId) {
      throw new Error(`No iroh endpoint found in DID document for ${targetRoot}`);
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
