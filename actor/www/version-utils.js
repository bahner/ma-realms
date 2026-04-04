export function epochSecondsFromDate(value) {
  if (!value) return '';
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return '';
  return String(Math.floor(dt.getTime() / 1000));
}

export function sanitizeEtag(rawEtag) {
  const value = String(rawEtag || '').trim();
  if (!value) return '';
  return value.replace(/^W\//, '').replace(/^"|"$/g, '');
}

export function shortFingerprint(value) {
  const clean = String(value || '').trim();
  if (!clean) return '';
  return clean.length > 12 ? clean.slice(0, 12) : clean;
}

export function asSemverBuildToken(value) {
  return String(value || '')
    .trim()
    .replace(/[^0-9A-Za-z.-]/g, '')
    .replace(/^\.+|\.+$/g, '');
}

export function withEpochAsPatch(version, epochSeconds) {
  const source = String(version || '').trim();
  const epoch = String(epochSeconds || '').trim();
  if (!source || !epoch) return source;

  const core = source.split(/[+-]/)[0];
  const extra = source.slice(core.length);
  const parts = core.split('.');
  if (parts.length < 2 || !/^\d+$/.test(parts[0]) || !/^\d+$/.test(parts[1])) {
    return source;
  }
  return `${parts[0]}.${parts[1]}.${epoch}${extra}`;
}

export async function resolveAppVersionLabel() {
  const explicit = String(globalThis.MA_ACTOR_VERSION || '').trim();
  if (explicit) {
    return explicit;
  }

  let pkgVersion = '';
  let epoch = '';
  let etag = '';

  try {
    const response = await fetch('./pkg/package.json', { cache: 'no-store' });
    if (response.ok) {
      const pkg = await response.json();
      pkgVersion = String(pkg?.version || '').trim();
      epoch = epochSecondsFromDate(response.headers.get('last-modified'));
      etag = shortFingerprint(sanitizeEtag(response.headers.get('etag')));
    }
  } catch {
    // Ignore and fall back below.
  }

  if (!epoch || !etag) {
    try {
      const response = await fetch('./pkg/ma_actor_bg.wasm', { method: 'HEAD', cache: 'no-store' });
      if (response.ok) {
        if (!epoch) {
          epoch = epochSecondsFromDate(response.headers.get('last-modified'));
        }
        if (!etag) {
          etag = shortFingerprint(sanitizeEtag(response.headers.get('etag')));
        }
      }
    } catch {
      // Ignore and fall back below.
    }
  }

  const versionWithEpochPatch = withEpochAsPatch(pkgVersion, epoch);
  const etagToken = asSemverBuildToken(etag);

  if (versionWithEpochPatch) {
    if (etagToken) {
      return `${versionWithEpochPatch}+etag.${etagToken}`;
    }
    return versionWithEpochPatch;
  }

  if (epoch) {
    if (etagToken) {
      return `0.0.${epoch}+etag.${etagToken}`;
    }
    return `0.0.${epoch}`;
  }

  if (etagToken) {
    return `dev+etag.${etagToken}`;
  }

  return 'dev';
}

export function semverCore(value) {
  return String(value || '').trim().split('+')[0].trim();
}
