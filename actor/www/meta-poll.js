import { isMaDid, isMaDidTarget } from './did.js';

export function createDidTargetMetaPollHandler(deps) {
  const {
    state,
    resolveAliasInput,
    didRoot,
    pollCurrentHomeEvents,
    appendMessage,
    sendWorldCommandQuery,
    parseRoomShowMeta,
    cacheRoomDidLookup
  } = deps;

  function normalizeMetaPollVerb(input) {
    const token = String(input || '').trim().toLowerCase();
    if (token === 'poll') {
      return 'poll';
    }
    return '';
  }

  return async function tryHandleDidTargetMetaPoll(targetToken, remainder) {
    const trimmedRemainder = String(remainder || '').trim();
    if (!trimmedRemainder) {
      return false;
    }

    const [verbRaw] = trimmedRemainder.split(/\s+/, 1);
    const metaCommand = normalizeMetaPollVerb(verbRaw);
    if (!metaCommand) {
      return false;
    }

    const resolved = String(resolveAliasInput(targetToken) || targetToken || '').trim();
    if (!isMaDidTarget(resolved)) {
      return false;
    }

    const roomFragment = String(resolved.split('#')[1] || '').trim();
    if (!roomFragment) {
      return false;
    }

    const targetWorldRoot = didRoot(resolved);
    const currentWorldRoot = didRoot(String(state.currentHome?.roomDid || '').trim());
    if (currentWorldRoot && targetWorldRoot && currentWorldRoot !== targetWorldRoot) {
      throw new Error(`poll target must be in current world (${currentWorldRoot}), got ${targetWorldRoot}`);
    }

    if (state.currentHome && state.currentHome.room === roomFragment) {
      await pollCurrentHomeEvents();
      appendMessage('system', `@${resolved} poll ok`);
      return true;
    }

    const showResponse = await sendWorldCommandQuery(`@world show #${roomFragment}`);
    const meta = parseRoomShowMeta(showResponse);
    if (meta.did && isMaDid(meta.did)) {
      cacheRoomDidLookup(roomFragment, meta.did);
    } else {
      cacheRoomDidLookup(roomFragment, resolved);
    }
    appendMessage('world', showResponse || `@world room='${roomFragment}'`);
    return true;
  };
}
