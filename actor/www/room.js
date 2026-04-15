import { isMaDid } from './did.js';

export function createRoomStorage({ state, lastRoomKeyPrefix }) {
  function lastRoomKey(identityDid, endpointId) {
    const idPart = (identityDid || '').split(':').pop() || 'unknown';
    const epPart = (endpointId || '').slice(0, 16);
    return `${lastRoomKeyPrefix}.${idPart}.${epPart}`;
  }

  function saveLastRoom(endpointId, room) {
    if (!state.identity?.did || !endpointId || !room) return;
    try {
      localStorage.setItem(lastRoomKey(state.identity.did, endpointId), room);
    } catch (_) {
      // Ignore storage write errors.
    }
  }

  function loadLastRoom(endpointId) {
    if (!state.identity?.did || !endpointId) return null;
    try {
      return localStorage.getItem(lastRoomKey(state.identity.did, endpointId)) || null;
    } catch (_) {
      return null;
    }
  }

  return {
    lastRoomKey,
    saveLastRoom,
    loadLastRoom,
  };
}

export function parseExitCidsFromRoomYaml(sourceText) {
  const lines = String(sourceText || '').replace(/\r\n/g, '\n').split('\n');
  const map = {};
  let inBlock = false;
  let baseIndent = 0;

  for (const line of lines) {
    if (!inBlock) {
      const match = line.match(/^(\s*)exit_cids:\s*(.*)$/);
      if (!match) {
        continue;
      }
      inBlock = true;
      baseIndent = match[1].length;
      const tail = String(match[2] || '').trim();
      if (!tail || tail === '{}') {
        continue;
      }
    }

    if (!line.trim()) {
      continue;
    }

    const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
    if (indent <= baseIndent) {
      break;
    }

    const pair = line.match(/^\s*([A-Za-z0-9._-]+)\s*:\s*([A-Za-z0-9]+)\s*$/);
    if (!pair) {
      continue;
    }
    map[pair[1]] = pair[2];
  }

  return map;
}

export function sanitizeRoomYamlForEdit(sourceText) {
  const lines = String(sourceText || '').replace(/\r\n/g, '\n').split('\n');
  const output = [];

  for (const line of lines) {
    if (/^\s*cid\s*:\s*/i.test(line)) {
      continue;
    }

    if (/^\s*did\s*:\s*/i.test(line)) {
      continue;
    }

    output.push(line);
  }

  return output.join('\n').replace(/\n{3,}/g, '\n\n');
}

export function humanRoomTitle(rawName) {
  const name = String(rawName || '').trim();
  if (!name) return 'Welcome';
  return name
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

export function createRoomPresenceFlow({
  state,
  cacheRoomDidLookup,
  dropCachedRoomDidLookup,
  renderAvatarPanel,
}) {
  function trackRoomPresence(handle, did, identity) {
    if (!handle) return;
    state.roomPresence.set(handle, {
      handle,
      did: did || '',
      identity: identity || ''
    });
    if (isMaDid(String(did || ''))) {
      cacheRoomDidLookup(handle, did);
    }
    renderAvatarPanel();
  }

  function removeRoomPresence(handle) {
    if (!handle) return;
    state.roomPresence.delete(handle);
    dropCachedRoomDidLookup(handle);
    renderAvatarPanel();
  }

  function clearRoomPresence() {
    state.roomPresence.clear();
    renderAvatarPanel();
  }

  return {
    trackRoomPresence,
    removeRoomPresence,
    clearRoomPresence,
  };
}

function cleanYamlScalar(value) {
  const raw = String(value || '').trim();
  if (!raw) {
    return '';
  }
  if ((raw.startsWith('"') && raw.endsWith('"')) || (raw.startsWith("'") && raw.endsWith("'"))) {
    return raw.slice(1, -1);
  }
  return raw;
}

function parseYamlListBlock(lines, startIndex, containerIndent, itemIndent) {
  const values = [];
  for (let i = startIndex + 1; i < lines.length; i += 1) {
    const line = lines[i];
    if (!line.trim()) {
      continue;
    }
    const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
    if (indent <= containerIndent) {
      break;
    }
    const itemMatch = line.match(new RegExp(`^\\s{${itemIndent}}-\\s*(.+)$`));
    if (itemMatch) {
      values.push(cleanYamlScalar(itemMatch[1]));
    }
  }
  return values;
}

function parseInlineYamlList(rawList) {
  return String(rawList || '')
    .split(',')
    .map((value) => cleanYamlScalar(value))
    .filter(Boolean);
}

function parseYamlListField(lines, lineIndex, inlinePattern, containerIndent, itemIndent) {
  if (lineIndex === -1) {
    return [];
  }
  const inlineMatch = lines[lineIndex].match(inlinePattern);
  if (inlineMatch) {
    return parseInlineYamlList(inlineMatch[1]);
  }
  return parseYamlListBlock(lines, lineIndex, containerIndent, itemIndent);
}

function parseIndentedYamlMap(lines, startIndex, baseIndent, pairPattern) {
  const result = {};
  if (startIndex === -1) {
    return result;
  }

  for (let i = startIndex + 1; i < lines.length; i += 1) {
    const line = lines[i];
    if (!line.trim()) {
      continue;
    }
    const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
    if (indent <= baseIndent) {
      break;
    }
    const pair = line.match(pairPattern);
    if (pair) {
      result[pair[1]] = cleanYamlScalar(pair[2]);
    }
  }

  return result;
}

export function parseExitYamlSummary(sourceText) {
  const text = String(sourceText || '').replace(/\r\n/g, '\n');
  const lines = text.split('\n');

  const idMatch = text.match(/^\s{2}id:\s*(.+)$/m);
  const nameMatch = text.match(/^\s{2}name:\s*(.+)$/m);
  const toMatch = text.match(/^\s{2}to:\s*(.+)$/m);
  const hiddenMatch = text.match(/^\s{2}hidden:\s*(true|false)\s*$/mi);
  const lockedMatch = text.match(/^\s{2}locked:\s*(true|false)\s*$/mi);
  const oneWayMatch = text.match(/^\s{2}one_way:\s*(true|false)\s*$/mi);

  const result = {
    id: idMatch ? cleanYamlScalar(idMatch[1]) : '',
    name: nameMatch ? cleanYamlScalar(nameMatch[1]) : '',
    names: {},
    to: toMatch ? cleanYamlScalar(toMatch[1]) : '',
    hidden: hiddenMatch ? hiddenMatch[1].toLowerCase() === 'true' : false,
    locked: lockedMatch ? lockedMatch[1].toLowerCase() === 'true' : false,
    oneWay: oneWayMatch ? oneWayMatch[1].toLowerCase() === 'true' : false,
    aliases: [],
    aclAllow: [],
    aclDeny: [],
    travelTexts: {}
  };

  const aliasesLine = lines.findIndex((line) => /^\s{2}aliases:\s*/.test(line));
  result.aliases = parseYamlListField(lines, aliasesLine, /^\s{2}aliases:\s*\[(.*)\]\s*$/, 2, 4);

  const namesLine = lines.findIndex((line) => /^\s{2}names:\s*/.test(line));
  const namesInlineEmpty = namesLine !== -1 && /^\s{2}names:\s*\{\s*\}\s*$/.test(lines[namesLine]);
  if (!namesInlineEmpty) {
    result.names = parseIndentedYamlMap(lines, namesLine, 2, /^\s{4}([A-Za-z0-9_-]+):\s*(.+)$/);
  }

  const aclLine = lines.findIndex((line) => /^\s{2}acl:\s*$/.test(line));
  if (aclLine !== -1) {
    const allowLine = lines.findIndex((line, idx) => idx > aclLine && /^\s{4}allow:\s*/.test(line));
    result.aclAllow = parseYamlListField(lines, allowLine, /^\s{4}allow:\s*\[(.*)\]\s*$/, 4, 6);

    const denyLine = lines.findIndex((line, idx) => idx > aclLine && /^\s{4}deny:\s*/.test(line));
    result.aclDeny = parseYamlListField(lines, denyLine, /^\s{4}deny:\s*\[(.*)\]\s*$/, 4, 6);
  }

  const travelLine = lines.findIndex((line) => /^\s{2}travel_texts:\s*/.test(line));
  const travelInlineEmpty = travelLine !== -1 && /^\s{2}travel_texts:\s*\{\s*\}\s*$/.test(lines[travelLine]);
  if (!travelInlineEmpty) {
    result.travelTexts = parseIndentedYamlMap(lines, travelLine, 2, /^\s{4}([A-Za-z0-9_-]+):\s*(.+)$/);
  }

  return result;
}

export function createRoomPresencePayloadFlow({
  state,
  updateRoomHeading,
  trackRoomPresence,
  removeRoomPresence,
  clearRoomPresence,
  appendMessage,
}) {
  function applyPresenceRoomMetadata(payload) {
    if (typeof payload.room_title === 'string' && payload.room_title) {
      state.currentHome.roomTitle = payload.room_title;
    }
    if (typeof payload.room_description === 'string') {
      state.currentHome.roomDescription = payload.room_description;
      updateRoomHeading(state.currentHome.roomTitle || '', payload.room_description);
    }
  }

  function applyPresenceSnapshot(payload) {
    clearRoomPresence();
    if (!Array.isArray(payload.avatars)) {
      return;
    }
    for (const avatar of payload.avatars) {
      const handle = String(avatar?.handle || '').trim();
      const did = String(avatar?.did || '').trim();
      const identity = String(avatar?.identity || '').trim();
      if (handle) {
        trackRoomPresence(handle, did, identity);
      }
    }
  }

  function applyPresenceJoin(payload) {
    const handle = String(payload.actor_handle || '').trim();
    const did = String(payload.actor_did || '').trim();
    const identity = String(payload.actor_identity || '').trim();
    if (handle) {
      trackRoomPresence(handle, did, identity);
    }
    if (typeof appendMessage === 'function') {
      if (handle && (identity || did)) {
        appendMessage('world', `${handle} entered the room. (${identity || did})`);
      } else if (handle) {
        appendMessage('world', `${handle} entered the room.`);
      } else if (identity || did) {
        appendMessage('world', `${identity || did} entered the room.`);
      }
    }
  }

  function applyPresenceLeave(payload) {
    const handle = String(payload.actor_handle || '').trim();
    const did = String(payload.actor_did || '').trim();
    const identity = String(payload.actor_identity || '').trim();
    if (handle) {
      removeRoomPresence(handle);
    }
    if (typeof appendMessage === 'function') {
      if (handle && (identity || did)) {
        appendMessage('world', `${handle} left the room. (${identity || did})`);
      } else if (handle) {
        appendMessage('world', `${handle} left the room.`);
      } else if (identity || did) {
        appendMessage('world', `${identity || did} left the room.`);
      }
    }
  }

  function applyPresencePayload(payload) {
    if (!payload || typeof payload !== 'object') {
      return;
    }

    if (!state.currentHome) {
      return;
    }

    const roomName = String(payload.room || '').trim();
    if (roomName && roomName !== state.currentHome.room) {
      return;
    }

    applyPresenceRoomMetadata(payload);

    const kind = String(payload.kind || '').trim();
    if (kind === 'presence.snapshot') {
      applyPresenceSnapshot(payload);
      return;
    }

    if (kind === 'presence.join') {
      applyPresenceJoin(payload);
      return;
    }

    if (kind === 'presence.leave') {
      applyPresenceLeave(payload);
    }
  }

  return {
    applyPresencePayload,
  };
}

export function createRoomInspectFlow({
  state,
  sendWorldCommandQuery,
  parseRoomShowMeta,
  extractRoomCidFromShowResponse,
  fetchGatewayTextByPath,
  asIpfsGatewayPath,
  uiText,
  appendMessage,
}) {
  async function fetchCurrentRoomInspectData() {
    if (!state.currentHome) {
      throw new Error('Not connected to a world.');
    }

    const showResponse = await sendWorldCommandQuery(`@world show #${state.currentHome.room}`);
    const meta = parseRoomShowMeta(showResponse);
    const roomCid = meta.cid || extractRoomCidFromShowResponse(showResponse);
    if (!roomCid || roomCid === '(unknown)') {
      throw new Error(`No room CID available. Response: ${showResponse}`);
    }

    const roomYaml = await fetchGatewayTextByPath(asIpfsGatewayPath(roomCid));
    const exitCidMap = parseExitCidsFromRoomYaml(roomYaml);

    return { meta, roomCid, roomYaml, exitCidMap };
  }

  async function inspectExitByQuery(queryText) {
    const query = String(queryText || '').trim();
    if (!query) {
      throw new Error(uiText('Usage: .inspect @exit <name|alias>', 'Bruk: .inspect @exit <navn|alias>'));
    }

    const info = await fetchCurrentRoomInspectData();
    const exits = Object.entries(info.exitCidMap);
    if (!exits.length) {
      throw new Error(uiText('No exits found in current room content.', 'Fant ingen utganger i innholdet for nåværende rom.'));
    }

    const target = query.toLowerCase();
    let matched = null;
    const discoveredNames = [];

    for (const [exitId, exitCid] of exits) {
      const exitYaml = await fetchGatewayTextByPath(asIpfsGatewayPath(exitCid));
      const summary = parseExitYamlSummary(exitYaml);
      const localizedNames = Object.values(summary.names || {});
      const names = [summary.name, ...localizedNames, ...summary.aliases]
        .filter(Boolean)
        .map((value) => value.toLowerCase());
      if (summary.name) {
        discoveredNames.push(summary.name);
      }
      if (names.includes(target) || exitId.toLowerCase() === target) {
        matched = { exitId, exitCid, summary };
        break;
      }
    }

    if (!matched) {
      const options = discoveredNames.length ? discoveredNames.join(', ') : uiText('(none)', '(ingen)');
      throw new Error(uiText(
        `Exit '${query}' not found. Known exits: ${options}`,
        `Fant ikke utgang '${query}'. Kjente utganger: ${options}`
      ));
    }

    const { exitId, exitCid, summary } = matched;
    appendMessage('system', `.inspect @exit ${query}`);
    appendMessage('system', `  id: ${summary.id || exitId}`);
    appendMessage('system', `  cid: ${exitCid}`);
    appendMessage('system', uiText(
      `  name: ${summary.name || '(unknown)'}`,
      `  navn: ${summary.name || '(ukjent)'}`
    ));
    appendMessage('system', uiText(
      `  to: ${summary.to || '(unknown)'}`,
      `  til: ${summary.to || '(ukjent)'}`
    ));
    appendMessage('system', uiText(
      `  aliases: ${summary.aliases.length ? summary.aliases.join(', ') : '(none)'}`,
      `  aliaser: ${summary.aliases.length ? summary.aliases.join(', ') : '(ingen)'}`
    ));
    const nameLanguages = Object.keys(summary.names || {}).sort();
    appendMessage('system', uiText(
      `  names: ${nameLanguages.length ? nameLanguages.map((key) => `${key}=${summary.names[key]}`).join(', ') : '(none)'}`,
      `  navn: ${nameLanguages.length ? nameLanguages.map((key) => `${key}=${summary.names[key]}`).join(', ') : '(ingen)'}`
    ));
    appendMessage('system', `  flags: hidden=${summary.hidden} locked=${summary.locked} one_way=${summary.oneWay}`);
    appendMessage('system', uiText(
      `  acl allow: ${summary.aclAllow.length ? summary.aclAllow.join(', ') : '(none)'}`,
      `  acl tillat: ${summary.aclAllow.length ? summary.aclAllow.join(', ') : '(ingen)'}`
    ));
    appendMessage('system', uiText(
      `  acl deny: ${summary.aclDeny.length ? summary.aclDeny.join(', ') : '(none)'}`,
      `  acl nekt: ${summary.aclDeny.length ? summary.aclDeny.join(', ') : '(ingen)'}`
    ));
    const travelKeys = Object.keys(summary.travelTexts || {});
    appendMessage('system', uiText(
      `  travel_texts: ${travelKeys.length ? travelKeys.join(', ') : '(none)'}`,
      `  travel_texts: ${travelKeys.length ? travelKeys.join(', ') : '(ingen)'}`
    ));
    for (const key of travelKeys) {
      appendMessage('system', `    ${key}: ${summary.travelTexts[key]}`);
    }
  }

  return {
    fetchCurrentRoomInspectData,
    inspectExitByQuery,
  };
}
