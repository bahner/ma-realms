export function parseShowAttributes(text) {
  const source = String(text || '').trim();
  const out = {};

  const keyMatches = Array.from(source.matchAll(/\b([a-zA-Z_][a-zA-Z0-9_\-]*)=/g));
  if (!keyMatches.length) {
    return out;
  }

  for (let i = 0; i < keyMatches.length; i += 1) {
    const match = keyMatches[i];
    const key = String(match[1] || '').trim();
    if (!key) continue;

    const valueStart = (match.index || 0) + String(match[0] || '').length;
    const next = keyMatches[i + 1];
    const valueEnd = next ? (next.index || source.length) : source.length;
    let rawValue = source.slice(valueStart, valueEnd).trim();

    if (!rawValue) {
      out[key] = '';
      continue;
    }

    if (rawValue.startsWith("'") && rawValue.endsWith("'")) {
      rawValue = rawValue.slice(1, -1);
    }

    out[key] = rawValue.trim();
  }

  return out;
}

export function parseAvatarDescriptionFromShowMessage(message) {
  const attrs = parseShowAttributes(message);
  return String(attrs.desc || '').trim();
}

export function parseDescriptionFromEditorText(sourceText) {
  const text = String(sourceText || '').replace(/\r\n/g, '\n');
  const blockMatch = text.match(/\bdescription:\s*\|\s*\n([\s\S]*)$/m);
  if (blockMatch && blockMatch[1] !== undefined) {
    const normalized = blockMatch[1]
      .split('\n')
      .map((line) => line.replace(/^\s{2}/, ''))
      .join('\n')
      .trim();
    if (normalized) return normalized;
  }

  const inlineMatch = text.match(/\bdescription:\s*(.*)$/m);
  if (inlineMatch && inlineMatch[1] !== undefined) {
    const inline = String(inlineMatch[1]).trim();
    if (inline) return inline;
  }

  return text.trim();
}

export function extractRoomCidFromShowResponse(message) {
  const attrs = parseShowAttributes(message);
  const cid = String(attrs.cid || '').trim();
  return /^[A-Za-z0-9]+$/.test(cid) ? cid : '';
}

export function parseRoomShowMeta(message) {
  const text = String(message || '').trim();
  const attrs = parseShowAttributes(text);
  const avatars = Number(attrs.avatars);
  const exits = Number(attrs.exits);
  return {
    room: String(attrs.room || '').trim(),
    url: String(attrs.url || attrs.did || '').replace(/[),.;]+$/, ''),
    owner: String(attrs.owner || '').trim(),
    cid: /^[A-Za-z0-9]+$/.test(String(attrs.cid || '')) ? String(attrs.cid || '') : '',
    avatars: Number.isFinite(avatars) ? avatars : null,
    exits: Number.isFinite(exits) ? exits : null,
    raw: text
  };
}

export function parseAvatarShowMeta(message) {
  const text = String(message || '').trim();
  const attrs = parseShowAttributes(text);
  return {
    owner: String(attrs.owner || '').trim(),
    description: String(attrs.desc || '').trim(),
    acl: String(attrs.acl || '').trim(),
    raw: text
  };
}

export function parseKeyValuePairs(text) {
  const source = String(text || '').trim();
  return parseShowAttributes(source);
}

export function extractDidFromLookupResponse(text) {
  const source = String(text || '').trim();
  const match = source.match(/\bdid=(did:ma:[^\s]+)/i);
  return match ? String(match[1]).trim() : '';
}
