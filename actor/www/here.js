import { createRoomPresenceFlow, createRoomPresencePayloadFlow } from './room.js';

export function createHereFlow({
  state,
  byId,
  cacheRoomDidLookup,
  dropCachedRoomDidLookup,
  updateRoomHeading,
  appendMessage,
  formatDidForDialog,
  didParts,
}) {
  function labelForPresenceEntry(entry) {
    const didText = String(entry?.did || '').trim();
    const handle = String(entry?.handle || '').trim();
    if (!didText) {
      return handle;
    }

    const parts = typeof didParts === 'function'
      ? didParts(didText)
      : { root: '', fragment: '' };
    const fragment = String(parts?.fragment || '').trim();
    const identityRoot = String(parts?.root || '').trim();

    if (fragment && identityRoot) {
      return `${fragment}(${identityRoot})`;
    }
    if (handle && identityRoot) {
      return `${handle}(${identityRoot})`;
    }
    return handle || formatDidForDialog(didText);
  }

  function renderAvatarPanel() {
    const list = byId('avatar-list');
    if (!list) return;
    list.innerHTML = '';
    const sorted = Array.from(state.roomPresence.values()).sort((a, b) => {
      const left = String(a?.handle || a?.did || '').toLowerCase();
      const right = String(b?.handle || b?.did || '').toLowerCase();
      return left.localeCompare(right);
    });
    for (const entry of sorted) {
      const li = document.createElement('li');
      li.className = 'avatar-item';
      const didText = String(entry?.did || '').trim();
      li.textContent = labelForPresenceEntry(entry);
      if (didText) {
        li.title = didText;
      }
      list.appendChild(li);
    }
  }

  const { trackRoomPresence, removeRoomPresence, clearRoomPresence } = createRoomPresenceFlow({
    state,
    cacheRoomDidLookup,
    dropCachedRoomDidLookup,
    renderAvatarPanel,
  });

  const { applyPresencePayload } = createRoomPresencePayloadFlow({
    state,
    updateRoomHeading,
    trackRoomPresence,
    removeRoomPresence,
    clearRoomPresence,
    appendMessage,
  });

  return {
    renderAvatarPanel,
    trackRoomPresence,
    removeRoomPresence,
    clearRoomPresence,
    applyPresencePayload,
  };
}
