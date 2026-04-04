const DEFAULT_LOCAL_GATEWAY_BASE = 'http://localhost:8080';

export function normalizeIpfsGatewayBase(value) {
  return String(value || '').trim().replace(/\/$/, '') || DEFAULT_LOCAL_GATEWAY_BASE;
}

export function asIpfsGatewayPath(value) {
  const input = String(value || '').trim();
  if (!input) {
    return '';
  }
  if (input.startsWith('/ipfs/')) {
    return input;
  }
  if (input.startsWith('ipfs://')) {
    return `/ipfs/${input.slice('ipfs://'.length)}`;
  }
  return `/ipfs/${input}`;
}

export async function fetchGatewayTextByPath(contentPath, { getApiBase, fallbackBases }) {
  const normalizedPath = String(contentPath || '').trim();
  if (!normalizedPath.startsWith('/ipfs/') && !normalizedPath.startsWith('/ipns/')) {
    throw new Error(`Invalid gateway path: ${normalizedPath || '(empty)'}`);
  }

  const baseCandidates = [
    getApiBase(),
    ...(Array.isArray(fallbackBases) ? fallbackBases : []),
  ]
    .map((entry) => normalizeIpfsGatewayBase(entry))
    .filter((entry, idx, list) => list.indexOf(entry) === idx);

  let lastError = '';
  for (const base of baseCandidates) {
    const url = `${base}${normalizedPath}`;
    try {
      const response = await fetch(url, { cache: 'no-store' });
      if (!response.ok) {
        lastError = `HTTP ${response.status} from ${url}`;
        continue;
      }
      return await response.text();
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
  }

  throw new Error(`Unable to fetch ${normalizedPath} from configured gateway/fallbacks: ${lastError || 'unknown error'}`);
}

export function createIpfsClient({
  getApiBase,
  isLocalhostLikeHost,
}) {
  async function ipfsRpcPost(path, query = {}, body = null) {
    const base = getApiBase();
    const isLocalIpfsRpc = /^https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/i.test(base);
    const isRemotePage = !isLocalhostLikeHost(window.location.hostname);
    const params = query instanceof URLSearchParams
      ? query
      : new URLSearchParams(query);
    const url = `${base}${path}${params.toString() ? `?${params.toString()}` : ''}`;

    let response;

    try {
      response = await fetch(url, {
        method: 'POST',
        body
      });
    } catch (error) {
      if (error instanceof TypeError) {
        if (isLocalIpfsRpc && isRemotePage) {
          const blocked = new Error('Browser blocked access to local IPFS RPC from this origin.');
          blocked.ipfsReason = 'remote-localhost-block';
          throw blocked;
        }
        if (isLocalIpfsRpc && !isRemotePage) {
          const blocked = new Error('Local IPFS RPC call failed from a localhost-like origin.');
          blocked.ipfsReason = 'localhost-cors-or-network';
          throw blocked;
        }
        const mixedContentHint = window.location.protocol === 'https:'
          ? 'This page is loaded over HTTPS; browsers often block http://127.0.0.1 mixed-content requests.'
          : '';
        throw new Error(`Unable to reach IPFS RPC API from browser. Check API URL, ensure IPFS RPC is running, and allow CORS for the app origin and headers. ${mixedContentHint}`.trim());
      }
      throw error;
    }

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`IPFS RPC API ${response.status}: ${text || response.statusText}`);
    }

    try {
      return await response.json();
    } catch {
      const text = await response.text();
      throw new Error(`IPFS RPC API returned non-JSON response: ${text || '(empty body)'}`);
    }
  }

  return {
    ipfsRpcPost,
  };
}
