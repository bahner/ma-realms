export function toSequenceNumber(value) {
  if (typeof value === 'bigint') {
    return Number(value);
  }

  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : 0;
}

export function toSequenceBigInt(value) {
  if (typeof value === 'bigint') {
    return value;
  }

  const numeric = toSequenceNumber(value);
  return BigInt(Math.max(0, Math.floor(numeric)));
}

export function normalizeRelayUrl(input) {
  let value = String(input || '').trim();
  while (value.endsWith('.') || value.endsWith('/')) {
    value = value.slice(0, -1);
  }
  return value + '/';
}

export function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function withTimeout(promise, ms, message) {
  let timer = null;
  const timeoutPromise = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), ms);
  });

  return Promise.race([promise, timeoutPromise]).finally(() => {
    if (timer) {
      clearTimeout(timer);
    }
  });
}
