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

export async function fetchGatewayTextByPath(contentPath, { getApiBase, fallbackBases, timeoutMs }) {
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
  const perRequestTimeoutMs = Number.isFinite(Number(timeoutMs)) && Number(timeoutMs) > 0
    ? Number(timeoutMs)
    : 8000;
  for (const base of baseCandidates) {
    const url = `${base}${normalizedPath}`;
    try {
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), perRequestTimeoutMs);
      const response = await fetch(url, {
        cache: 'no-store',
        signal: controller.signal,
      }).finally(() => clearTimeout(timeout));
      if (!response.ok) {
        lastError = `HTTP ${response.status} from ${url}`;
        if (normalizedPath.startsWith('/ipns/') && response.status >= 500) {
          throw new Error(lastError);
        }
        continue;
      }
      return await response.text();
    } catch (error) {
      if (error && typeof error === 'object' && error.name === 'AbortError') {
        lastError = `timeout after ${perRequestTimeoutMs}ms from ${url}`;
      } else {
        lastError = error instanceof Error ? error.message : String(error);
      }
    }
  }

  throw new Error(`Unable to fetch ${normalizedPath} from configured gateway/fallbacks: ${lastError || 'unknown error'}`);
}

