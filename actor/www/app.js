import init, {
  create_identity,
  unlock_identity,
  ensure_bundle_iroh_secret,
  set_bundle_language_preferences,
  set_bundle_world,
  generate_bip39_phrase,
  normalize_bip39_phrase,
  connect_world,
  connect_world_with_relay,
  enter_world,
  poll_world_events,
  send_world_chat,
  send_world_whisper,
  send_world_cmd,
  send_world_message,
  decode_chat_event_message,
  decode_whisper_event_message,
  start_inbox_listener,
  poll_inbox_messages,
  inspect_signed_message,
  alias_did_root,
  alias_normalize_endpoint_id,
  alias_resolve_input,
  alias_find_alias_for_address,
  alias_find_did_by_endpoint,
  alias_humanize_identifier,
  alias_humanize_text,
  closet_start,
  closet_command,
  disconnect_world
} from './pkg/ma_actor.js';
import { createInboundDispatcher } from './inbox-dispatcher.js';
import { createDialogWriter } from './dialog-writer.js';
import { createIdentityStore } from './identity-store.js';
import { createInboxTransport } from './inbox-transport.js';
import { createDidTargetMetaPollHandler } from './meta-poll.js';

const STORAGE_PREFIX = 'ma.identity.v3';
const PROPER_NAME = '間';
const BRAND_SUBTITLE_STATIC = 'A text-first world for literate play';
const API_KEY = `${STORAGE_PREFIX}.kuboApi`;
const ALIAS_BOOK_KEY = `${STORAGE_PREFIX}.aliasBook`;
const LAST_ALIAS_KEY = `${STORAGE_PREFIX}.lastAlias`;
const TAB_ALIAS_KEY = `${STORAGE_PREFIX}.tabAlias`;
const DEBUG_KEY = `${STORAGE_PREFIX}.debug`;
const LEGACY_BUNDLE_KEY = 'ma.identity.v2.bundle';
const LAST_ROOM_KEY_PREFIX = `${STORAGE_PREFIX}.lastRoom`;
const LAST_ACTIVE_HOME_KEY_PREFIX = `${STORAGE_PREFIX}.lastActiveHome`;
const BLOCKLIST_KEY_PREFIX = `${STORAGE_PREFIX}.blockedDidRoots`;
const LAST_PUBLISHED_IPNS_KEY = `${STORAGE_PREFIX}.lastPublishedIpns`;
const LAST_PUBLISHED_CID_KEY = `${STORAGE_PREFIX}.lastPublishedCid`;
const HOME_PUBLISH_KEY_ALIAS = 'ma-actor';
const LEGACY_API_KEY = 'ma.identity.v2.kuboApi';
const LEGACY_ALIAS_KEY = 'ma.identity.v2.alias';
const DEFAULT_LANG = 'en';
const DEFAULT_LANGUAGE_PREFERENCES = 'en_UK';
const DEFAULT_UI_LANG = 'en';
const LOCAL_EDIT_SCRIPT_KEY = `${STORAGE_PREFIX}.localEditScript`;
const LOCAL_EDIT_SCRIPT_CID_KEY = `${STORAGE_PREFIX}.localEditScriptCid`;

const ROOM_POLL_INTERVAL_MS = 1500;
const DID_DOC_CACHE_TTL_MS = 60_000;
const LOCK_OVERLAY_TARGET_FPS = 45;
const LOCK_OVERLAY_MAX_DPR = 1.4;
const BRIDGE_REQUEST_TYPE = 'MA_KUBO_BRIDGE_REQUEST';
const BRIDGE_RESPONSE_TYPE = 'MA_KUBO_BRIDGE_RESPONSE';
const BRIDGE_READY_TYPE = 'MA_KUBO_BRIDGE_READY';
const BRIDGE_TIMEOUT_MS = 4500;
let bridgeRequestCounter = 0;
let bridgeReadySeen = false;
let bridgeMonitorInstalled = false;

function epochSecondsFromDate(value) {
  if (!value) return '';
  const dt = new Date(value);
  if (Number.isNaN(dt.getTime())) return '';
  return String(Math.floor(dt.getTime() / 1000));
}

function sanitizeEtag(rawEtag) {
  const value = String(rawEtag || '').trim();
  if (!value) return '';
  return value.replace(/^W\//, '').replace(/^"|"$/g, '');
}

function shortFingerprint(value) {
  const clean = String(value || '').trim();
  if (!clean) return '';
  return clean.length > 12 ? clean.slice(0, 12) : clean;
}

function asSemverBuildToken(value) {
  return String(value || '')
    .trim()
    .replace(/[^0-9A-Za-z.-]/g, '')
    .replace(/^\.+|\.+$/g, '');
}

function withEpochAsPatch(version, epochSeconds) {
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

async function resolveAppVersionLabel() {
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

function semverCore(value) {
  return String(value || '').trim().split('+')[0].trim();
}

async function fetchWorldActorWebInfo() {
  try {
    const response = await fetch('/actor/web/info', { cache: 'no-store' });
    if (!response.ok) return null;
    const payload = await response.json();
    if (!payload || payload.enabled !== true) return null;
    return {
      version: String(payload.version || '').trim(),
      cid: String(payload.cid || '').trim(),
    };
  } catch {
    return null;
  }
}

async function updateAppVersionFooter() {
  const versionEl = byId('app-version');
  if (!versionEl) return;
  const localVersion = await resolveAppVersionLabel();
  const worldWeb = await fetchWorldActorWebInfo();

  let label = `Version: ${localVersion}`;
  if (worldWeb) {
    const localCore = semverCore(localVersion);
    const worldCore = semverCore(worldWeb.version);
    const mismatch = worldCore && localCore && worldCore !== localCore;
    const cid = worldWeb.cid ? ` | cid: ${worldWeb.cid}` : '';
    label += ` | world: ${worldWeb.version || 'unknown'}${cid}`;
    if (mismatch) {
      label += ' | mismatch';
    }
  }

  versionEl.textContent = label;
}

function isLocalhostLikeHost(hostname) {
  const host = String(hostname || '').toLowerCase();
  return host === 'localhost' || host === '127.0.0.1' || host.endsWith('.localhost');
}

const state = {
  identity: null,
  encryptedBundle: '',
  aliasName: '',
  lang: DEFAULT_LANG,
  languagePreferences: DEFAULT_LANGUAGE_PREFERENCES,
  uiLang: DEFAULT_UI_LANG,
  debug: false,
  aliasBook: {},
  currentHome: null,
  roomPollTimer: null,
  roomPollInFlight: false,
  inboxPollInFlight: false,
  pollErrorShown: false,
  passphrase: '',
  handleDidMap: {},
  roomDidLookupCache: new Map(),
  roomDidLookupInFlight: new Map(),
  didEndpointMap: {},
  blockedDidRoots: new Set(),
  didDocCache: new Map(),
  inboxEndpointId: '',
  mailbox: [],
  mailboxSeq: 0,
  commandHistory: [],
  historyIndex: -1,
  historyDraft: '',
  roomPresence: new Map(),
  activeObjectTargetAlias: '',
  activeObjectTargetDid: '',
  activeObjectTargetRequirement: 'none',
  closetSessionId: '',
  closetEndpointId: '',
  closetLobbySeq: 0,
  transparentReentryPromise: null,
  editSession: null,
  editBusy: false,
  lockOverlayAnimationId: 0,
  lockOverlayStarDrift: 0
};

const RECONNECT_DELAY_MS = 3000;
const ROOM_DID_CACHE_TTL_MS = 30000;

function roomDidLookupCacheKey(token) {
  if (!state.currentHome) return '';
  const endpoint = String(state.currentHome.endpointId || '').trim();
  const room = String(state.currentHome.room || '').trim();
  const normalized = String(token || '').trim().replace(/^@+/, '').toLowerCase();
  if (!endpoint || !room || !normalized) return '';
  return `${endpoint}::${room}::${normalized}`;
}

function getCachedRoomDidLookup(token) {
  const key = roomDidLookupCacheKey(token);
  if (!key) return '';
  const entry = state.roomDidLookupCache.get(key);
  if (!entry || typeof entry !== 'object') {
    return '';
  }
  const now = Date.now();
  if (!entry.expiresAt || entry.expiresAt <= now) {
    state.roomDidLookupCache.delete(key);
    return '';
  }
  return String(entry.did || '').trim();
}

function cacheRoomDidLookup(token, did) {
  const key = roomDidLookupCacheKey(token);
  const normalizedDid = String(did || '').trim();
  if (!key || !normalizedDid.startsWith('did:ma:')) return;
  state.roomDidLookupCache.set(key, {
    did: normalizedDid,
    expiresAt: Date.now() + ROOM_DID_CACHE_TTL_MS
  });
}

function dropCachedRoomDidLookup(token) {
  const key = roomDidLookupCacheKey(token);
  if (!key) return;
  state.roomDidLookupCache.delete(key);
}

function lastRoomKey(identityDid, endpointId) {
  const idPart = (identityDid || '').split(':').pop() || 'unknown';
  const epPart = (endpointId || '').slice(0, 16);
  return `${LAST_ROOM_KEY_PREFIX}.${idPart}.${epPart}`;
}

function saveLastRoom(endpointId, room) {
  if (!state.identity?.did || !endpointId || !room) return;
  try { localStorage.setItem(lastRoomKey(state.identity.did, endpointId), room); } catch (_) {}
}

function loadLastRoom(endpointId) {
  if (!state.identity?.did || !endpointId) return null;
  try { return localStorage.getItem(lastRoomKey(state.identity.did, endpointId)) || null; } catch (_) { return null; }
}

function activeHomeKey(identityDid) {
  const rootDid = didRoot(identityDid || '');
  if (!rootDid) return '';
  return `${LAST_ACTIVE_HOME_KEY_PREFIX}.${rootDid}`;
}

function isUnconfiguredDidTarget(value) {
  const text = String(value || '').trim().toLowerCase();
  return text.startsWith('did:ma:unconfigured');
}

function buildCurrentHomeResumeTarget() {
  if (!state.currentHome) {
    return '';
  }

  const roomDid = String(state.currentHome.roomDid || '').trim();
  if (roomDid.startsWith('did:ma:') && !isUnconfiguredDidTarget(roomDid)) {
    return roomDid;
  }

  const worldDid = didRoot(findDidByEndpoint(state.currentHome.endpointId) || '');
  if (worldDid) {
    const room = String(state.currentHome.room || 'lobby').trim() || 'lobby';
    return `${worldDid}#${room}`;
  }

  return String(state.currentHome.endpointId || '').trim();
}

function saveActiveHomeSnapshot() {
  if (!state.identity?.did || !state.currentHome) {
    return;
  }
  const key = activeHomeKey(state.identity.did);
  if (!key) {
    return;
  }

  let target = buildCurrentHomeResumeTarget();
  if (isUnconfiguredDidTarget(target)) {
    target = String(state.currentHome.endpointId || '').trim();
  }
  if (!target) {
    return;
  }

  const snapshot = {
    target,
    room: String(state.currentHome.room || '').trim(),
    endpointId: String(state.currentHome.endpointId || '').trim(),
    savedAt: Date.now()
  };

  try {
    localStorage.setItem(key, JSON.stringify(snapshot));
  } catch (_) {
    // Ignore storage write failures.
  }
}

function loadActiveHomeSnapshot(identityDid) {
  const key = activeHomeKey(identityDid);
  if (!key) {
    return null;
  }

  try {
    const raw = localStorage.getItem(key);
    if (!raw) {
      return null;
    }
    const parsed = JSON.parse(raw);
    const target = String(parsed?.target || '').trim();
    if (!target) {
      return null;
    }
    return {
      target,
      room: String(parsed?.room || '').trim(),
      endpointId: String(parsed?.endpointId || '').trim()
    };
  } catch (_) {
    return null;
  }
}

async function restoreActiveHomeAfterUnlock() {
  if (!state.identity?.did) {
    return;
  }

  const snapshot = loadActiveHomeSnapshot(state.identity.did);
  if (!snapshot?.target) {
    return;
  }

  if (isUnconfiguredDidTarget(snapshot.target)) {
    const endpointFallback = String(snapshot.endpointId || '').trim();
    if (!endpointFallback) {
      appendSystemUi(
        'Skipping invalid saved location (did:ma:unconfigured).',
        'Hopper over ugyldig lagret lokasjon (did:ma:unconfigured).'
      );
      return;
    }
    snapshot.target = endpointFallback;
  }

  appendMessage('system', `Restoring last location: ${snapshot.target}`);
  try {
    await enterHome(snapshot.target, snapshot.room || null);
  } catch (error) {
    appendMessage('system', `Could not restore last location: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function blocklistKey(identityDid) {
  const rootDid = didRoot(identityDid || '');
  if (!rootDid) return '';
  return `${BLOCKLIST_KEY_PREFIX}.${rootDid}`;
}

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

function loadBlockedDidRootsForIdentity(identityDid) {
  const key = blocklistKey(identityDid);
  if (!key) {
    state.blockedDidRoots = new Set();
    return;
  }
  try {
    const raw = localStorage.getItem(key);
    if (!raw) {
      state.blockedDidRoots = new Set();
      return;
    }
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      state.blockedDidRoots = new Set();
      return;
    }
    state.blockedDidRoots = new Set(
      parsed
        .map((value) => didRoot(String(value || '')))
        .filter((value) => value.startsWith('did:ma:'))
    );
  } catch {
    state.blockedDidRoots = new Set();
  }
}

function resolveTargetDidRoot(token) {
  const key = String(token || '').trim();
  if (!key) {
    throw new Error('Usage: .block <did|alias|handle>');
  }
  const resolved = resolveAliasInput(key);
  const mappedDid = state.handleDidMap[key] || state.handleDidMap[resolved] || '';
  const candidate = mappedDid || findDidByEndpoint(resolved) || resolved;
  const root = didRoot(candidate);
  if (!root.startsWith('did:ma:')) {
    throw new Error(`Could not resolve a did:ma target from '${key}'.`);
  }
  return root;
}

function readStoredDebugFlag() {
  const raw = localStorage.getItem(DEBUG_KEY);
  if (!raw) return false;
  const value = raw.trim().toLowerCase();
  return value === '1' || value === 'true' || value === 'on';
}

function setDebugMode(enabled, announce = true) {
  state.debug = Boolean(enabled);
  localStorage.setItem(DEBUG_KEY, state.debug ? '1' : '0');
  if (announce) {
    appendMessage('system', `Debug mode: ${state.debug ? 'on' : 'off'}`);
  }
}

function byId(id) {
  return document.getElementById(id);
}

function isGatewayViewOrigin() {
  return isLocalhostLikeHost(window.location.hostname) && window.location.port === '8080';
}

function setSetupActionsEnabled(enabled) {
  const ids = ['btn-create', 'btn-unlock'];
  for (const id of ids) {
    const el = byId(id);
    if (el) {
      el.disabled = !enabled;
    }
  }
}

function stopLockOverlayAnimation() {
  if (state.lockOverlayAnimationId) {
    cancelAnimationFrame(state.lockOverlayAnimationId);
    state.lockOverlayAnimationId = 0;
  }
}

function hideLockOverlay() {
  const overlay = byId('lock-overlay');
  if (!overlay) return;
  overlay.classList.add('hidden');
  overlay.setAttribute('aria-hidden', 'true');
  stopLockOverlayAnimation();
}

function setYamlEditorBusy(busy) {
  state.editBusy = Boolean(busy);
  for (const id of ['yaml-editor-reload', 'yaml-editor-cancel', 'yaml-editor-save', 'yaml-editor-close-eval', 'yaml-editor-text']) {
    const el = byId(id);
    if (el) {
      el.disabled = state.editBusy;
    }
  }
}

function setYamlEditorStatus(message, tone = 'idle') {
  const statusEl = byId('yaml-editor-status');
  if (!statusEl) return;
  statusEl.textContent = String(message || '');
  statusEl.classList.remove('ok', 'error', 'working');
  if (tone === 'ok' || tone === 'error' || tone === 'working') {
    statusEl.classList.add(tone);
  }
}

function updateYamlEditorContext() {
  const contextEl = byId('yaml-editor-context');
  if (!contextEl) return;
  if (!state.editSession) {
    contextEl.textContent = uiText('No edit target loaded.', 'Ingen redigeringsmål lastet.');
    return;
  }

  if (state.editSession.mode === 'script') {
    const cid = String(state.editSession.sourceCid || '').trim();
    if (cid && cid !== '(not published yet)') {
      contextEl.textContent = uiText(
        `Mode: local script | CID: ${cid}`,
        `Modus: lokalt script | CID: ${cid}`
      );
    } else {
      contextEl.textContent = uiText(
        'Mode: local script | CID: (not published yet)',
        'Modus: lokalt script | CID: (ikke publisert ennå)'
      );
    }
    return;
  }

  if (state.editSession.mode === 'avatar') {
    contextEl.textContent = uiText('Mode: avatar (@me)', 'Modus: avatar (@me)');
    return;
  }

  if (state.editSession.mode === 'exit') {
    contextEl.textContent = uiText(
      `Mode: exit | Target: ${state.editSession.target} | Source CID: ${state.editSession.sourceCid}`,
      `Modus: utgang | Mål: ${state.editSession.target} | Kilde-CID: ${state.editSession.sourceCid}`
    );
    return;
  }

  contextEl.textContent = uiText(
    `Mode: room | Target: ${state.editSession.target} | Source CID: ${state.editSession.sourceCid}`,
    `Modus: rom | Mål: ${state.editSession.target} | Kilde-CID: ${state.editSession.sourceCid}`
  );
}

function updateYamlEditorControls() {
  const saveBtn = byId('yaml-editor-save');
  const reloadBtn = byId('yaml-editor-reload');
  const closeEvalBtn = byId('yaml-editor-close-eval');
  const textEl = byId('yaml-editor-text');
  if (!saveBtn || !reloadBtn || !textEl || !closeEvalBtn) return;
  const isNb = state.uiLang === 'nb';

  const mode = state.editSession?.mode || 'room';
  if (mode === 'script') {
    saveBtn.textContent = isNb ? 'Lagre lokalt script' : 'Save Local Script';
    reloadBtn.textContent = isNb ? 'Last lokalt' : 'Reload Local';
    closeEvalBtn.textContent = isNb ? 'Lukk og Evaluer' : 'Close and Eval';
    closeEvalBtn.classList.remove('hidden');
    textEl.placeholder = isNb ? 'Skriv lokal scripttekst her.' : 'Write local script text here.';
    return;
  }

  if (mode === 'avatar') {
    saveBtn.textContent = isNb ? 'Bruk avatar' : 'Apply Avatar';
    reloadBtn.textContent = isNb ? 'Last avatar' : 'Reload Avatar';
    closeEvalBtn.classList.add('hidden');
    textEl.placeholder = isNb ? 'Avatar-utkast (YAML-liknende).' : 'Avatar YAML-like draft.';
    return;
  }

  if (mode === 'exit') {
    saveBtn.textContent = isNb ? 'Lagre utgang' : 'Save Exit';
    reloadBtn.textContent = isNb ? 'Last utgang' : 'Reload Exit';
    closeEvalBtn.classList.add('hidden');
    textEl.placeholder = isNb ? 'Utgang-YAML vises her.' : 'Exit YAML will appear here.';
    return;
  }

  saveBtn.textContent = isNb ? 'Lagre + Publiser' : 'Save + Publish';
  reloadBtn.textContent = isNb ? 'Last kilde' : 'Reload Source';
  closeEvalBtn.classList.add('hidden');
  textEl.placeholder = isNb ? 'Rom-YAML vises her.' : 'Room YAML will appear here.';
}

async function closeAndEvalEditorScript() {
  if (!state.editSession || state.editSession.mode !== 'script') {
    appendSystemUi(
      'Close and Eval is only available in .edit script mode.',
      'Lukk og Evaluer er kun tilgjengelig i .edit script-modus.'
    );
    return;
  }

  await saveYamlEditorChanges();
  const cid = String(state.editSession.sourceCid || '').trim();
  if (!cid || cid === '(not published yet)') {
    return;
  }

  closeYamlEditorModal();
  parseDot(`.eval ${cid}`);
}

function closeYamlEditorModal() {
  const modal = byId('yaml-editor-modal');
  if (!modal) return;
  modal.classList.add('hidden');
  modal.setAttribute('aria-hidden', 'true');
  setYamlEditorBusy(false);
  const input = byId('command-input');
  if (input) input.focus();
}

function openYamlEditorModal() {
  const modal = byId('yaml-editor-modal');
  if (!modal) return;
  updateYamlEditorContext();
  updateYamlEditorControls();
  modal.classList.remove('hidden');
  modal.setAttribute('aria-hidden', 'false');
  setTimeout(() => {
    const textEl = byId('yaml-editor-text');
    if (textEl) textEl.focus();
  }, 0);
}

function normalizeEditTarget(rawTarget) {
  if (!state.currentHome) {
    throw new Error('Not connected to a world. Connect first.');
  }

  const input = String(rawTarget || '').trim();
  if (!input || input === 'here' || input === 'room') {
    return state.currentHome.room;
  }

  let target = input;
  if (target.startsWith('did:ma:')) {
    const hashIdx = target.indexOf('#');
    if (hashIdx === -1) {
      throw new Error('Usage: .edit [@here|@me|did:ma:<world>#room]');
    }
    target = target.slice(hashIdx + 1);
  }

  target = target.trim().replace(/^#/, '');
  if (!/^[A-Za-z0-9_-]+$/.test(target)) {
    throw new Error(`Invalid room target '${target}'. Expected [A-Za-z0-9_-]+.`);
  }
  return target;
}

function parseAvatarDescriptionFromShowMessage(message) {
  const text = String(message || '').trim();
  const tagged = text.match(/\bdesc=(.*)\sacl=/);
  if (tagged && tagged[1] !== undefined) {
    return String(tagged[1]).trim();
  }
  const fallback = text.match(/\bdesc=(.*)$/);
  if (fallback && fallback[1] !== undefined) {
    return String(fallback[1]).trim();
  }
  return '';
}

function parseDescriptionFromEditorText(sourceText) {
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

function extractRoomCidFromShowResponse(message) {
  const match = String(message || '').match(/\bcid=([A-Za-z0-9]+)\b/);
  return match ? match[1] : '';
}

function parseRoomShowMeta(message) {
  const text = String(message || '').trim();
  const roomMatch = text.match(/\broom='([^']+)'/i);
  const didMatch = text.match(/\bdid=(did:ma:[^\s]+)/i);
  const ownerMatch = text.match(/\bowner=([^\s]+)/i);
  const cidMatch = text.match(/\bcid=([A-Za-z0-9]+)/i);
  const avatarsMatch = text.match(/\bavatars=(\d+)/i);
  const exitsMatch = text.match(/\bexits=(\d+)/i);
  return {
    room: roomMatch ? roomMatch[1] : '',
    did: didMatch ? String(didMatch[1]).replace(/[),.;]+$/, '') : '',
    owner: ownerMatch ? ownerMatch[1] : '',
    cid: cidMatch ? cidMatch[1] : '',
    avatars: avatarsMatch ? Number(avatarsMatch[1]) : null,
    exits: exitsMatch ? Number(exitsMatch[1]) : null,
    raw: text
  };
}

function buildRoomYamlDraftFromMeta(meta, fallbackRoomName) {
  const roomName = String(meta?.room || fallbackRoomName || 'lobby').trim() || 'lobby';
  const owner = String(meta?.owner || '').trim();
  const ownerLine = owner && owner !== '(none)' ? `  owner: ${owner}` : '  owner: (none)';

  return [
    'room:',
    `  id: ${roomName}`,
    `  name: ${roomName}`,
    '  title: ',
    '  description: ',
    'acl:',
    ownerLine,
    'exit_cids:',
    '  # Add exits as: <exit_id>: <ipfs_cid>'
  ].join('\n');
}

function parseOwnerFromRoomYaml(sourceText) {
  const lines = String(sourceText || '').replace(/\r\n/g, '\n').split('\n');
  let inAcl = false;
  let aclIndent = 0;

  for (const line of lines) {
    const aclMatch = line.match(/^(\s*)acl:\s*(.*)$/);
    if (aclMatch) {
      inAcl = true;
      aclIndent = aclMatch[1].length;
      continue;
    }

    if (!inAcl) {
      continue;
    }

    if (!line.trim()) {
      continue;
    }

    const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
    if (indent <= aclIndent) {
      break;
    }

    const ownerMatch = line.match(/^\s*owner:\s*(.+)\s*$/);
    if (ownerMatch) {
      return cleanYamlScalar(ownerMatch[1]);
    }
  }

  return '';
}

function parseAvatarShowMeta(message) {
  const text = String(message || '').trim();
  const ownerAclMatch = text.match(/\bowner=([^\s]+)\s+desc=(.*?)\s+acl=(.+)$/i);
  if (ownerAclMatch) {
    return {
      owner: String(ownerAclMatch[1] || '').trim(),
      description: String(ownerAclMatch[2] || '').trim(),
      acl: String(ownerAclMatch[3] || '').trim(),
      raw: text
    };
  }

  const ownerDescMatch = text.match(/\bowner=([^\s]+)\s+desc=(.+)$/i);
  if (ownerDescMatch) {
    return {
      owner: String(ownerDescMatch[1] || '').trim(),
      description: String(ownerDescMatch[2] || '').trim(),
      acl: '',
      raw: text
    };
  }

  return { owner: '', description: '', acl: '', raw: text };
}

function parseExitCidsFromRoomYaml(sourceText) {
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

    if (!inBlock) {
      continue;
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

function parseExitYamlSummary(sourceText) {
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
  if (aliasesLine !== -1) {
    const inlineMatch = lines[aliasesLine].match(/^\s{2}aliases:\s*\[(.*)\]\s*$/);
    if (inlineMatch) {
      result.aliases = inlineMatch[1]
        .split(',')
        .map((value) => cleanYamlScalar(value))
        .filter(Boolean);
    } else {
      result.aliases = parseYamlListBlock(lines, aliasesLine, 2, 4);
    }
  }

  const namesLine = lines.findIndex((line) => /^\s{2}names:\s*/.test(line));
  if (namesLine !== -1) {
    const inlineEmpty = /^\s{2}names:\s*\{\s*\}\s*$/.test(lines[namesLine]);
    if (!inlineEmpty) {
      for (let i = namesLine + 1; i < lines.length; i += 1) {
        const line = lines[i];
        if (!line.trim()) {
          continue;
        }
        const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
        if (indent <= 2) {
          break;
        }
        const pair = line.match(/^\s{4}([A-Za-z0-9_-]+):\s*(.+)$/);
        if (pair) {
          result.names[pair[1]] = cleanYamlScalar(pair[2]);
        }
      }
    }
  }

  const aclLine = lines.findIndex((line) => /^\s{2}acl:\s*$/.test(line));
  if (aclLine !== -1) {
    const allowLine = lines.findIndex((line, idx) => idx > aclLine && /^\s{4}allow:\s*/.test(line));
    if (allowLine !== -1) {
      const inlineAllow = lines[allowLine].match(/^\s{4}allow:\s*\[(.*)\]\s*$/);
      if (inlineAllow) {
        result.aclAllow = inlineAllow[1]
          .split(',')
          .map((value) => cleanYamlScalar(value))
          .filter(Boolean);
      } else {
        result.aclAllow = parseYamlListBlock(lines, allowLine, 4, 6);
      }
    }
    const denyLine = lines.findIndex((line, idx) => idx > aclLine && /^\s{4}deny:\s*/.test(line));
    if (denyLine !== -1) {
      const inlineDeny = lines[denyLine].match(/^\s{4}deny:\s*\[(.*)\]\s*$/);
      if (inlineDeny) {
        result.aclDeny = inlineDeny[1]
          .split(',')
          .map((value) => cleanYamlScalar(value))
          .filter(Boolean);
      } else {
        result.aclDeny = parseYamlListBlock(lines, denyLine, 4, 6);
      }
    }
  }

  const travelLine = lines.findIndex((line) => /^\s{2}travel_texts:\s*/.test(line));
  if (travelLine !== -1) {
    const inlineEmpty = /^\s{2}travel_texts:\s*\{\s*\}\s*$/.test(lines[travelLine]);
    if (!inlineEmpty) {
      for (let i = travelLine + 1; i < lines.length; i += 1) {
        const line = lines[i];
        if (!line.trim()) {
          continue;
        }
        const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
        if (indent <= 2) {
          break;
        }
        const pair = line.match(/^\s{4}([A-Za-z0-9_-]+):\s*(.+)$/);
        if (pair) {
          result.travelTexts[pair[1]] = cleanYamlScalar(pair[2]);
        }
      }
    }
  }

  return result;
}

function replaceExitCidInRoomYaml(roomYamlText, exitId, newCid) {
  const lines = String(roomYamlText || '').replace(/\r\n/g, '\n').split('\n');
  let inBlock = false;
  let baseIndent = 0;
  let replaced = false;
  let blockEnd = -1;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (!inBlock) {
      const match = line.match(/^(\s*)exit_cids:\s*(.*)$/);
      if (!match) {
        continue;
      }
      inBlock = true;
      baseIndent = match[1].length;
      continue;
    }

    if (!line.trim()) {
      continue;
    }

    const indent = (line.match(/^(\s*)/) || ['', ''])[1].length;
    if (indent <= baseIndent) {
      blockEnd = i;
      break;
    }

    const pair = line.match(/^(\s*)([A-Za-z0-9._-]+)\s*:\s*([A-Za-z0-9]+)\s*$/);
    if (!pair) {
      continue;
    }
    if (pair[2] === exitId) {
      lines[i] = `${pair[1]}${exitId}: ${newCid}`;
      replaced = true;
    }
  }

  if (!inBlock) {
    throw new Error('Room YAML has no exit_cids block.');
  }

  if (!replaced) {
    const insertAt = blockEnd === -1 ? lines.length : blockEnd;
    const indent = ' '.repeat(baseIndent + 2);
    lines.splice(insertAt, 0, `${indent}${exitId}: ${newCid}`);
  }

  return lines.join('\n');
}

function roomLanguageKey() {
  const primary = normalizeLanguageTag(state.lang).toLowerCase();
  if (!primary) {
    return 'und';
  }
  return primary;
}

async function appendAmbientProseAfterSpeech() {
  if (!state.currentHome) {
    return;
  }

  const info = await fetchCurrentRoomInspectData();
  const languageKey = roomLanguageKey();
  const roomDescription = String(state.currentHome.roomDescription || '').trim() || uiText('(no description)', '(ingen beskrivelse)');
  const exits = Object.entries(info.exitCidMap);
  const labels = [];

  for (const [exitId, exitCid] of exits) {
    try {
      const exitYaml = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(exitCid) });
      const summary = parseExitYamlSummary(exitYaml);
      const localized = summary.names[languageKey] || summary.names.und || summary.name || exitId;
      labels.push(localized);
    } catch {
      labels.push(exitId);
    }
  }

  const exitsText = labels.length ? labels.join(', ') : uiText('(none)', '(ingen)');
  appendMessage('system', uiText(
    `You stand in ${roomDescription}. Exits: [${exitsText}]`,
    `Du står i ${roomDescription}. Utganger: [${exitsText}]`
  ));
}

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

  const roomYaml = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(roomCid) });
  const exitCidMap = parseExitCidsFromRoomYaml(roomYaml);
  const ownerFromYaml = parseOwnerFromRoomYaml(roomYaml);

  if (!meta.owner && ownerFromYaml) {
    meta.owner = ownerFromYaml;
  }

  return { meta, roomCid, roomYaml, exitCidMap };
}

async function inspectSelfAvatar() {
  if (!state.currentHome) {
    throw new Error(uiText('Not connected to a world.', 'Ikke koblet til en world.'));
  }

  const response = await sendWorldCommandQuery('@me show');
  const info = parseAvatarShowMeta(response);
  const handle = state.currentHome.handle || state.aliasName || '(unknown)';
  const did = state.identity?.did || '(unknown)';

  appendMessage('system', '.inspect @me');
  appendMessage('system', uiText(`  handle: ${handle}`, `  håndtak: ${handle}`));
  appendMessage('system', uiText(`  did: ${did}`, `  did: ${did}`));
  appendMessage('system', uiText(
    `  owner: ${info.owner || '(not exposed)'}`,
    `  eier: ${info.owner || '(ikke eksponert)'}`
  ));
  appendMessage('system', uiText(
    `  description: ${info.description || '(none)'}`,
    `  beskrivelse: ${info.description || '(ingen)'}`
  ));
  if (info.acl) {
    appendMessage('system', `  acl: ${info.acl}`);
  }
}

async function inspectCurrentRoom() {
  const info = await fetchCurrentRoomInspectData();
  const exits = Object.entries(info.exitCidMap);

  appendMessage('system', `.inspect @here`);
  appendMessage('system', uiText(
    `  room: ${info.meta.room || state.currentHome?.room || '(unknown)'}`,
    `  rom: ${info.meta.room || state.currentHome?.room || '(ukjent)'}`
  ));
  appendMessage('system', uiText(
    `  did: ${info.meta.did || '(not exposed)'}`,
    `  did: ${info.meta.did || '(ikke eksponert)'}`
  ));
  appendMessage('system', uiText(
    `  owner: ${info.meta.owner || '(not exposed)'}`,
    `  eier: ${info.meta.owner || '(ikke eksponert)'}`
  ));
  appendMessage('system', uiText(
    `  content cid: ${info.roomCid}`,
    `  innholds-cid: ${info.roomCid}`
  ));
  appendSystemUi('  state cid: (not exposed by @world show)', '  state-cid: (ikke eksponert av @world show)');
  appendSystemUi('  acl cid: (not exposed by @world show)', '  acl-cid: (ikke eksponert av @world show)');
  appendMessage('system', uiText(
    `  exits in content: ${exits.length}`,
    `  utganger i innhold: ${exits.length}`
  ));

  for (const [exitId, exitCid] of exits) {
    try {
      const exitYaml = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(exitCid) });
      const summary = parseExitYamlSummary(exitYaml);
      const localizedName = summary.names[state.uiLang] || summary.names.en || '';
      const label = localizedName || summary.name || exitId;
      appendMessage('system', uiText(
        `    - ${label} -> ${summary.to || '(unknown)'} [${exitCid}]`,
        `    - ${label} -> ${summary.to || '(ukjent)'} [${exitCid}]`
      ));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      appendMessage('system', uiText(
        `    - ${exitId} [${exitCid}] (inspect failed: ${message})`,
        `    - ${exitId} [${exitCid}] (inspect feilet: ${message})`
      ));
    }
  }
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
    const exitYaml = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(exitCid) });
    const summary = parseExitYamlSummary(exitYaml);
    const localizedNames = Object.values(summary.names || {});
    const names = [summary.name, ...localizedNames, ...summary.aliases]
      .filter(Boolean)
      .map((value) => value.toLowerCase());
    if (summary.name) {
      discoveredNames.push(summary.name);
    }
    if (names.includes(target) || exitId.toLowerCase() === target) {
      matched = { exitId, exitCid, summary, exitYaml };
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

function parseKeyValuePairs(text) {
  const source = String(text || '');
  const regex = /([a-zA-Z_][a-zA-Z0-9_\-]*)=([^\s]+)/g;
  const out = {};
  let match;
  while ((match = regex.exec(source)) !== null) {
    out[match[1]] = match[2];
  }
  return out;
}

function extractDidFromLookupResponse(text) {
  const source = String(text || '').trim();
  const match = source.match(/\bdid=(did:ma:[^\s]+)/i);
  return match ? String(match[1]).trim() : '';
}

async function lookupDidInCurrentRoom(query) {
  const token = String(query || '').trim().replace(/^@+/, '');
  if (!token) {
    throw new Error(uiText('Usage: .use <object|did> [as @alias]', 'Bruk: .use <objekt|did> [as @alias]'));
  }
  if (token.startsWith('did:ma:')) {
    return token;
  }
  const cached = getCachedRoomDidLookup(token);
  if (cached) {
    return cached;
  }

  const inflightKey = roomDidLookupCacheKey(token);
  if (inflightKey) {
    const inflight = state.roomDidLookupInFlight.get(inflightKey);
    if (inflight) {
      return await inflight;
    }
  }

  const lookupPromise = (async () => {
    const response = await sendWorldCommandQuery(`@here id ${token}`);
    const did = extractDidFromLookupResponse(response);
    if (!did) {
      throw new Error(uiText(
        `Could not resolve DID for '${token}'.`,
        `Fant ikke DID for '${token}'.`
      ));
    }
    cacheRoomDidLookup(token, did);
    return did;
  })();

  if (inflightKey) {
    state.roomDidLookupInFlight.set(inflightKey, lookupPromise);
  }

  try {
    return await lookupPromise;
  } finally {
    if (inflightKey) {
      state.roomDidLookupInFlight.delete(inflightKey);
    }
  }
}

function isBuiltinTargetToken(value) {
  const token = String(value || '').trim().toLowerCase();
  return token === 'here' || token === 'room' || token === 'avatar' || token === 'world';
}

async function resolveCommandTargetDidOrToken(targetToken) {
  const raw = String(targetToken || '').trim().replace(/^@+/, '');
  if (!raw) {
    throw new Error('Usage: @target <command>');
  }
  const activeAliasRaw = String(state.activeObjectTargetAlias || '').trim().replace(/^@+/, '');
  const activeDid = String(state.activeObjectTargetDid || '').trim();
  if (activeAliasRaw && activeAliasRaw.toLowerCase() === raw.toLowerCase() && activeDid.startsWith('did:ma:')) {
    cacheRoomDidLookup(raw, activeDid);
    return activeDid;
  }
  if (isBuiltinTargetToken(raw)) {
    return raw;
  }
  if (raw.startsWith('did:ma:')) {
    return raw;
  }

  const resolvedAlias = String(resolveAliasInput(raw) || '').trim();
  if (resolvedAlias.startsWith('did:ma:')) {
    return resolvedAlias;
  }

  const mappedDid = state.handleDidMap[raw]
    || state.handleDidMap[resolvedAlias]
    || '';
  if (String(mappedDid).startsWith('did:ma:')) {
    cacheRoomDidLookup(raw, mappedDid);
    return mappedDid;
  }

  return await lookupDidInCurrentRoom(raw);
}

async function inspectObjectByQuery(queryText) {
  const raw = String(queryText || '').trim();
  const target = raw.replace(/^@+/, '').trim();
  if (!target) {
    throw new Error(uiText('Usage: .inspect <object|@object>', 'Bruk: .inspect <objekt|@objekt>'));
  }

  const lookedUpDid = await lookupDidInCurrentRoom(target);
  const response = await sendWorldCommandQuery(`@${lookedUpDid} show`);
  const lowered = String(response || '').toLowerCase();
  if (lowered.includes('unknown actor or object')) {
    throw new Error(uiText(
      `Object '${target}' not found in current room.`,
      `Fant ikke objekt '${target}' i nåværende rom.`
    ));
  }

  const kv = parseKeyValuePairs(response);
  const objectDid = kv.did || lookedUpDid || '(not exposed)';
  const objectKind = kv.kind || '(unknown)';
  const objectName = String(response || '').match(/^@([^\s]+)/)?.[1] || target;

  appendMessage('system', `.inspect ${raw}`);
  appendMessage('system', uiText(`  object: ${objectName}`, `  objekt: ${objectName}`));
  appendMessage('system', `  did: ${objectDid}`);
  appendMessage('system', uiText(`  kind: ${objectKind}`, `  type: ${objectKind}`));

  if (objectDid !== '(not exposed)') {
    const suggestedAlias = objectName.replace(/[^A-Za-z0-9_-]/g, '').toLowerCase() || 'obj';
    appendMessage('system', `  use ${objectDid} as @${suggestedAlias}`);
  }

  if (kv.definition_cid) {
    appendMessage('system', `  definition_cid: ${kv.definition_cid}`);
  }
  if (kv.persistence) {
    appendMessage('system', `  persistence: ${kv.persistence}`);
  }
  if (kv.durable_inbox_messages || kv.ephemeral_inbox_messages) {
    appendMessage('system', `  inbox: durable=${kv.durable_inbox_messages || '0'} ephemeral=${kv.ephemeral_inbox_messages || '0'}`);
  }
}

async function onDotInspect(rawArgs) {
  try {
    const arg = String(rawArgs || '').trim();
    if (!arg || arg === '@here' || arg === 'here' || arg === 'room') {
      await inspectCurrentRoom();
      return;
    }

    if (arg === '@me' || arg === 'me' || arg === 'self') {
      await inspectSelfAvatar();
      return;
    }

    if (arg.startsWith('@exit')) {
      const exitQuery = arg.slice('@exit'.length).trim();
      await inspectExitByQuery(exitQuery);
      return;
    }

    await inspectObjectByQuery(arg);
    return;
  } catch (error) {
    appendMessage('system', uiText(
      `Inspect failed: ${error instanceof Error ? error.message : String(error)}`,
      `Inspect feilet: ${error instanceof Error ? error.message : String(error)}`
    ));
  }
}

function sanitizeRoomYamlForEdit(sourceText) {
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

async function sendWorldCommandQuery(commandText) {
  if (!state.identity || !state.currentHome) {
    throw new Error('Join a home before sending commands.');
  }

  const result = JSON.parse(
    await send_world_cmd(
      state.currentHome.endpointId,
      state.passphrase,
      state.encryptedBundle,
      state.aliasName,
      state.currentHome.room,
      state.languagePreferences,
      commandText
    )
  );

  if (!result.ok) {
    throw new Error(result.message || 'command failed');
  }
  if (result.broadcasted) {
    await pollCurrentHomeEvents();
  }
  return String(result.message || '');
}

async function loadLocalScriptEditor() {
  const storedCid = localStorage.getItem(LOCAL_EDIT_SCRIPT_CID_KEY) || '';
  state.editSession = {
    mode: 'script',
    target: 'local-script',
    sourceCid: storedCid || '(not published yet)'
  };

  const textEl = byId('yaml-editor-text');
  if (textEl) {
    textEl.value = localStorage.getItem(LOCAL_EDIT_SCRIPT_KEY) || '';
  }

  updateYamlEditorContext();
  setYamlEditorStatus('Local script mode.', 'ok');
  openYamlEditorModal();
}

function resolveEvalSourceToken(token) {
  const input = String(token || '').trim();
  if (!input) {
    throw new Error('Usage: .eval <cid|alias|/ipfs/cid>');
  }

  const aliasExact = String(state.aliasBook?.[input] || '').trim();
  let candidate = aliasExact || resolveAliasInput(input);
  candidate = String(candidate || '').trim();

  if (!candidate) {
    throw new Error(`Could not resolve eval source '${input}'.`);
  }

  if (candidate.startsWith('ipfs://')) {
    candidate = candidate.slice('ipfs://'.length);
  }
  if (candidate.startsWith('/ipfs/')) {
    candidate = candidate.slice('/ipfs/'.length);
  }

  return candidate.trim();
}

function asIpfsCatArg(value) {
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

async function executeScriptLine(line) {
  const text = String(line || '').trim();
  if (!text || text.startsWith('#')) {
    return;
  }

  if (text.startsWith('.')) {
    const dot = text.slice(1).trim();
    const [verbRaw] = dot.split(/\s+/);
    const verb = String(verbRaw || '').toLowerCase();

    if (!verb) {
      return;
    }

    if (verb === 'edit' || verb === 'eval') {
      throw new Error(`Script line '${text}' is not allowed inside .eval script`);
    }
    if (verb === 'help') {
      return;
    }

    const handled = parseDot(text);
    if (!handled) {
      throw new Error(`Unknown dot command in script: ${text}`);
    }
    return;
  }

  await sendCurrentWorldMessage(text);
}

async function onDotEval(rawArgs) {
  try {
    const resolved = resolveEvalSourceToken(rawArgs);
    appendMessage('system', `Evaluating script from ${resolved}...`);
    const scriptText = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(resolved) });
    const lines = String(scriptText || '').replace(/\r\n/g, '\n').split('\n');

    let executed = 0;
    for (const line of lines) {
      const candidate = String(line || '').trim();
      if (!candidate || candidate.startsWith('#')) {
        continue;
      }
      await executeScriptLine(candidate);
      executed += 1;
    }

    appendMessage('system', `.eval complete (${executed} command${executed === 1 ? '' : 's'}).`);
  } catch (error) {
    appendMessage('system', `Eval failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}

async function loadAvatarEditor() {
  if (!state.currentHome) {
    throw new Error('Not connected to a world. Connect first.');
  }

  setYamlEditorBusy(true);
  setYamlEditorStatus('Loading avatar state from @me show...', 'working');

  try {
    const response = await sendWorldCommandQuery('@me show');
    const description = parseAvatarDescriptionFromShowMessage(response);
    const yamlDraft = [
      'kind: avatar',
      'target: @me',
      'description: |',
      ...String(description || '')
        .split('\n')
        .map((line) => `  ${line}`)
    ].join('\n');

    state.editSession = {
      mode: 'avatar',
      target: '@me',
      sourceCid: '(runtime)'
    };

    const textEl = byId('yaml-editor-text');
    if (textEl) {
      textEl.value = yamlDraft;
    }

    updateYamlEditorContext();
    setYamlEditorStatus('Loaded avatar draft.', 'ok');
    openYamlEditorModal();
  } finally {
    setYamlEditorBusy(false);
  }
}

async function loadYamlEditorForTarget(target, announce = true) {
  setYamlEditorBusy(true);
  setYamlEditorStatus(`Loading source YAML for room '${target}'...`, 'working');

  try {
    const showResponse = await sendWorldCommandQuery(`@world show #${target}`);
    if (showResponse.includes('not found')) {
      throw new Error(showResponse);
    }

    const showMeta = parseRoomShowMeta(showResponse);
    const sourceCid = extractRoomCidFromShowResponse(showResponse);
    const cidMissing = !sourceCid || sourceCid === '(unknown)';

    let safeYamlText = '';
    let loadedFromDraft = false;

    if (cidMissing) {
      const isCurrentRoom = Boolean(state.currentHome) && String(state.currentHome.room || '').trim() === String(target || '').trim();
      if (!isCurrentRoom) {
        throw new Error(`No room CID available for '${target}'. Response: ${showResponse}`);
      }
      safeYamlText = buildRoomYamlDraftFromMeta(showMeta, target);
      loadedFromDraft = true;
    } else {
      const yamlText = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(sourceCid) });
      safeYamlText = sanitizeRoomYamlForEdit(yamlText);
    }

    state.editSession = {
      mode: 'room',
      target,
      sourceCid: sourceCid || '(runtime)'
    };

    const textEl = byId('yaml-editor-text');
    if (textEl) {
      textEl.value = safeYamlText;
    }

    updateYamlEditorContext();
    if (loadedFromDraft) {
      setYamlEditorStatus(`Loaded room '${target}' draft (no CID yet).`, 'ok');
    } else {
      setYamlEditorStatus(`Loaded room '${target}' from ${sourceCid}.`, 'ok');
    }
    openYamlEditorModal();

    if (announce) {
      if (loadedFromDraft) {
        appendMessage('system', `Loaded .edit draft for room '${target}' (no published CID yet). Save to publish first CID.`);
      } else {
        appendMessage('system', `Loaded .edit source for room '${target}' from CID ${sourceCid}.`);
      }
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setYamlEditorStatus(`Load failed: ${message}`, 'error');
    throw error;
  } finally {
    setYamlEditorBusy(false);
  }
}

async function loadExitEditorByQuery(queryText) {
  if (!state.currentHome) {
    throw new Error('Not connected to a world. Connect first.');
  }

  const query = String(queryText || '').trim();
  if (!query) {
    throw new Error('Usage: .edit @exit <name|alias>');
  }

  setYamlEditorBusy(true);
  setYamlEditorStatus(`Loading exit '${query}'...`, 'working');

  try {
    const info = await fetchCurrentRoomInspectData();
    const exits = Object.entries(info.exitCidMap);
    if (!exits.length) {
      throw new Error('No exits found in current room content.');
    }

    const target = query.toLowerCase();
    let matched = null;

    for (const [exitId, exitCid] of exits) {
      const exitYaml = await kuboPostText('/api/v0/cat', { arg: asIpfsCatArg(exitCid) });
      const summary = parseExitYamlSummary(exitYaml);
      const localizedNames = Object.values(summary.names || {});
      const names = [summary.name, ...localizedNames, ...summary.aliases]
        .filter(Boolean)
        .map((value) => value.toLowerCase());
      if (names.includes(target) || exitId.toLowerCase() === target) {
        matched = { exitId, exitCid, exitYaml, summary, roomInfo: info };
        break;
      }
    }

    if (!matched) {
      throw new Error(`Exit '${query}' not found in this room.`);
    }

    state.editSession = {
      mode: 'exit',
      target: `@exit ${matched.summary.name || matched.exitId}`,
      sourceCid: matched.exitCid,
      exitId: matched.exitId,
      roomTarget: matched.roomInfo.meta.room || state.currentHome.room,
      roomCid: matched.roomInfo.roomCid
    };

    const textEl = byId('yaml-editor-text');
    if (textEl) {
      textEl.value = matched.exitYaml;
    }

    updateYamlEditorContext();
    setYamlEditorStatus(`Loaded exit '${matched.exitId}' from ${matched.exitCid}.`, 'ok');
    openYamlEditorModal();
    appendMessage('system', `Loaded .edit source for exit '${matched.exitId}' from CID ${matched.exitCid}.`);
  } finally {
    setYamlEditorBusy(false);
  }
}

async function saveYamlEditorChanges() {
  if (state.editBusy) {
    return;
  }
  if (!state.editSession) {
    appendMessage('system', 'No active .edit session. Run .edit first.');
    return;
  }
  if (state.editSession.mode !== 'script' && !state.currentHome) {
    appendMessage('system', 'This edit mode requires an active world. Connect first.');
    return;
  }

  const textEl = byId('yaml-editor-text');
  const yamlText = textEl ? textEl.value : '';
  if (!yamlText.trim()) {
    setYamlEditorStatus('Refusing to save empty content.', 'error');
    return;
  }

  if (state.editSession.mode === 'script') {
    setYamlEditorBusy(true);
    setYamlEditorStatus('Saving local script and publishing CID...', 'working');
    try {
      localStorage.setItem(LOCAL_EDIT_SCRIPT_KEY, yamlText);

      const blob = new Blob([yamlText], { type: 'text/plain' });
      const formData = new FormData();
      formData.append('file', blob, 'local-script.ma');
      const addResult = await kuboPost('/api/v0/add', { pin: 'true' }, formData);
      const cid = String(addResult?.Hash || '').trim();
      if (!cid) {
        throw new Error('Kubo add did not return a CID.');
      }

      state.editSession.sourceCid = cid;
      localStorage.setItem(LOCAL_EDIT_SCRIPT_CID_KEY, cid);
      updateYamlEditorContext();
      setYamlEditorStatus(`Saved and published: ${cid}`, 'ok');
      appendMessage('system', `Saved local .edit script and published CID ${cid}.`);
      appendMessage('system', `Run .eval ${cid} to execute it.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setYamlEditorStatus(`Script save failed: ${message}`, 'error');
      appendMessage('system', `Script save failed: ${message}`);
    } finally {
      setYamlEditorBusy(false);
    }
    return;
  }

  if (state.editSession.mode === 'avatar') {
    const description = parseDescriptionFromEditorText(yamlText);
    if (!description) {
      setYamlEditorStatus('Avatar description is empty.', 'error');
      return;
    }

    setYamlEditorBusy(true);
    setYamlEditorStatus('Applying avatar update via @me describe ...', 'working');
    try {
      await sendCurrentWorldMessage(`@me describe ${description}`);
      setYamlEditorStatus('Avatar updated.', 'ok');
      appendMessage('system', 'Applied avatar edit from .edit @me.');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setYamlEditorStatus(`Avatar update failed: ${message}`, 'error');
      appendMessage('system', `Avatar edit failed: ${message}`);
    } finally {
      setYamlEditorBusy(false);
    }
    return;
  }

  if (state.editSession.mode === 'exit') {
    const exitId = String(state.editSession.exitId || '').trim();
    if (!exitId) {
      setYamlEditorStatus('Exit edit session is missing exit id.', 'error');
      return;
    }

    setYamlEditorBusy(true);
    setYamlEditorStatus(`Publishing YAML for exit '${exitId}'...`, 'working');
    try {
      const exitBlob = new Blob([yamlText], { type: 'application/yaml' });
      const exitForm = new FormData();
      exitForm.append('file', exitBlob, `${exitId}.yaml`);
      const exitAdd = await kuboPost('/api/v0/add', { pin: 'true' }, exitForm);
      const newExitCid = String(exitAdd?.Hash || '').trim();
      if (!newExitCid) {
        throw new Error('Kubo add did not return an exit CID.');
      }

      const info = await fetchCurrentRoomInspectData();
      const updatedRoomYaml = replaceExitCidInRoomYaml(info.roomYaml, exitId, newExitCid);

      const roomBlob = new Blob([updatedRoomYaml], { type: 'application/yaml' });
      const roomForm = new FormData();
      roomForm.append('file', roomBlob, `${info.meta.room || state.currentHome.room}.yaml`);
      const roomAdd = await kuboPost('/api/v0/add', { pin: 'true' }, roomForm);
      const newRoomCid = String(roomAdd?.Hash || '').trim();
      if (!newRoomCid) {
        throw new Error('Kubo add did not return a room CID.');
      }

      state.editSession.sourceCid = newExitCid;
      state.editSession.roomCid = newRoomCid;
      updateYamlEditorContext();

      await sendCurrentWorldMessage(`@here set cid ${newRoomCid}`);
      appendMessage('system', `Applied exit '${exitId}' as CID ${newExitCid}.`);
      appendMessage('system', `Applied updated room CID ${newRoomCid}.`);
      setYamlEditorStatus(`Published exit ${newExitCid} and applied room ${newRoomCid}.`, 'ok');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setYamlEditorStatus(`Exit save failed: ${message}`, 'error');
      appendMessage('system', `Exit edit failed: ${message}`);
    } finally {
      setYamlEditorBusy(false);
    }
    return;
  }

  setYamlEditorBusy(true);
  setYamlEditorStatus(`Publishing YAML for room '${state.editSession.target}'...`, 'working');

  try {
    const safeRoomYaml = sanitizeRoomYamlForEdit(yamlText);
    const blob = new Blob([safeRoomYaml], { type: 'application/yaml' });
    const formData = new FormData();
    formData.append('file', blob, `${state.editSession.target}.yaml`);

    const addResult = await kuboPost('/api/v0/add', { pin: 'true' }, formData);
    const newCid = String(addResult?.Hash || '').trim();
    if (!newCid) {
      throw new Error('Kubo add did not return a CID.');
    }

    state.editSession.sourceCid = newCid;
    updateYamlEditorContext();

    if (state.currentHome.room === state.editSession.target) {
      await sendCurrentWorldMessage(`@here set cid ${newCid}`);
      appendMessage('system', `Applied room '${state.editSession.target}' from new CID ${newCid}.`);
      setYamlEditorStatus(`Published and applied: ${newCid}`, 'ok');
      return;
    }

    appendMessage('system', `Published new CID for room '${state.editSession.target}': ${newCid}`);
    appendMessage('system', `To apply, enter that room and run: @here set cid ${newCid}`);
    setYamlEditorStatus(`Published ${newCid}. Manual apply required for non-current room.`, 'ok');
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setYamlEditorStatus(`Save failed: ${message}`, 'error');
    appendMessage('system', `Edit publish failed: ${message}`);
  } finally {
    setYamlEditorBusy(false);
  }
}

async function onDotEdit(rawArgs) {
  try {
    const arg = String(rawArgs || '').trim();
    if (!arg) {
      await loadLocalScriptEditor();
      return;
    }

    if (arg === '@here') {
      const target = normalizeEditTarget('here');
      await loadYamlEditorForTarget(target);
      return;
    }

    if (arg === '@me') {
      await loadAvatarEditor();
      return;
    }

    if (arg.startsWith('@exit')) {
      const query = arg.slice('@exit'.length).trim();
      await loadExitEditorByQuery(query);
      return;
    }

    if (arg.startsWith('did:ma:')) {
      const target = normalizeEditTarget(arg);
      await loadYamlEditorForTarget(target);
      return;
    }

    throw new Error('Usage: .edit | .edit @here | .edit @me | .edit @exit <name|alias> | .edit did:ma:<world>#<room>');
  } catch (error) {
    appendMessage('system', `Edit failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function onYamlEditorModalKeyDown(event) {
  if (event.key === 'Escape') {
    event.preventDefault();
    closeYamlEditorModal();
  }
}

function drawLockOverlayScene() {
  const canvas = byId('lock-canvas');
  if (!canvas) return;

  const dpr = Math.min(LOCK_OVERLAY_MAX_DPR, Math.max(1, window.devicePixelRatio || 1));
  const rect = canvas.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return;

  const width = Math.floor(rect.width * dpr);
  const height = Math.floor(rect.height * dpr);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }

  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const fishSchool = [
    { speed: 0.07, depth: 0.08, size: 1.0, dir: 1, colorA: 'rgba(114, 236, 255, 0.95)', colorB: 'rgba(116, 160, 255, 0.86)' },
    { speed: 0.055, depth: 0.18, size: 0.82, dir: -1, colorA: 'rgba(255, 188, 132, 0.95)', colorB: 'rgba(247, 107, 165, 0.82)' },
    { speed: 0.082, depth: 0.25, size: 0.72, dir: 1, colorA: 'rgba(146, 255, 214, 0.95)', colorB: 'rgba(77, 206, 255, 0.84)' },
    { speed: 0.048, depth: 0.33, size: 1.18, dir: -1, colorA: 'rgba(174, 205, 255, 0.94)', colorB: 'rgba(122, 139, 255, 0.86)' }
  ];
  const starCount = 72;
  const minFrameMs = 1000 / LOCK_OVERLAY_TARGET_FPS;
  let lastRenderMs = 0;

  function drawFish(x, y, scale, dir, phase, colorA, colorB) {
    const bodyLen = 46 * scale;
    const bodyH = 18 * scale;
    const tailW = 16 * scale;
    const tailSwing = Math.sin(phase * 8.5) * 4 * scale;

    ctx.save();
    ctx.translate(x, y);
    // Fish body points left by default, so flip by -dir to align heading with travel direction.
    ctx.scale(-dir, 1);

    const fishGrad = ctx.createLinearGradient(-bodyLen * 0.5, 0, bodyLen * 0.4, 0);
    fishGrad.addColorStop(0, colorA);
    fishGrad.addColorStop(1, colorB);

    ctx.fillStyle = fishGrad;
    ctx.beginPath();
    ctx.moveTo(-bodyLen * 0.56, 0);
    ctx.quadraticCurveTo(-bodyLen * 0.1, -bodyH * 0.95, bodyLen * 0.52, 0);
    ctx.quadraticCurveTo(-bodyLen * 0.1, bodyH * 0.95, -bodyLen * 0.56, 0);
    ctx.closePath();
    ctx.fill();

    ctx.fillStyle = 'rgba(232, 252, 255, 0.32)';
    ctx.beginPath();
    ctx.ellipse(-bodyLen * 0.08, -bodyH * 0.2, bodyLen * 0.24, bodyH * 0.26, 0, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = 'rgba(173, 244, 255, 0.85)';
    ctx.beginPath();
    ctx.moveTo(bodyLen * 0.52, 0);
    ctx.lineTo(bodyLen * 0.52 + tailW, -bodyH * 0.58 + tailSwing);
    ctx.lineTo(bodyLen * 0.52 + tailW, bodyH * 0.58 + tailSwing);
    ctx.closePath();
    ctx.fill();

    ctx.fillStyle = 'rgba(250, 255, 255, 0.92)';
    ctx.beginPath();
    ctx.arc(-bodyLen * 0.3, -bodyH * 0.15, 1.9 * scale, 0, Math.PI * 2);
    ctx.fill();

    ctx.restore();
  }

  const paint = (nowMs = performance.now()) => {
    if (lastRenderMs && nowMs - lastRenderMs < minFrameMs) {
      state.lockOverlayAnimationId = requestAnimationFrame(paint);
      return;
    }
    lastRenderMs = nowMs;

    const t = nowMs * 0.001;
    const w = canvas.width;
    const h = canvas.height;
    const horizon = h * 0.67;

    const bg = ctx.createLinearGradient(0, 0, 0, h);
    bg.addColorStop(0, '#070f22');
    bg.addColorStop(0.45, '#101f48');
    bg.addColorStop(1, '#1a1a42');
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);

    const nebulaA = ctx.createRadialGradient(
      w * (0.25 + 0.03 * Math.sin(t * 0.3)),
      h * 0.26,
      12 * dpr,
      w * 0.25,
      h * 0.26,
      w * 0.42
    );
    nebulaA.addColorStop(0, 'rgba(255, 130, 190, 0.27)');
    nebulaA.addColorStop(0.55, 'rgba(132, 112, 255, 0.18)');
    nebulaA.addColorStop(1, 'rgba(42, 60, 120, 0)');
    ctx.fillStyle = nebulaA;
    ctx.fillRect(0, 0, w, h);

    const nebulaB = ctx.createRadialGradient(
      w * 0.77,
      h * (0.32 + 0.02 * Math.sin(t * 0.47)),
      16 * dpr,
      w * 0.77,
      h * 0.32,
      w * 0.46
    );
    nebulaB.addColorStop(0, 'rgba(106, 240, 255, 0.2)');
    nebulaB.addColorStop(0.52, 'rgba(83, 145, 255, 0.15)');
    nebulaB.addColorStop(1, 'rgba(20, 38, 92, 0)');
    ctx.fillStyle = nebulaB;
    ctx.fillRect(0, 0, w, h);

    state.lockOverlayStarDrift += 0.34;
    const drift = state.lockOverlayStarDrift;
    for (let i = 0; i < starCount; i += 1) {
      const sx = ((i * 313 + drift * (i % 9 + 2)) % (w + 160)) - 80;
      const sy = ((i * 181) % Math.max(1, Math.floor(horizon))) + (i % 7 === 0 ? Math.sin(t + i) * 4 * dpr : 0);
      const twinkle = 0.24 + 0.68 * Math.abs(Math.sin(t * (0.5 + (i % 5) * 0.2) + i));
      const radius = (0.6 + (i % 4) * 0.55) * dpr;
      ctx.fillStyle = `rgba(222, 243, 255, ${Math.min(0.95, twinkle)})`;
      ctx.beginPath();
      ctx.arc(sx, sy, radius, 0, Math.PI * 2);
      ctx.fill();
    }

    const cometPhase = (t * 0.08) % 1;
    const cometX = w * (1.1 - cometPhase * 1.2);
    const cometY = h * (0.12 + cometPhase * 0.34);
    const tail = ctx.createLinearGradient(cometX - w * 0.22, cometY - h * 0.08, cometX, cometY);
    tail.addColorStop(0, 'rgba(130, 220, 255, 0)');
    tail.addColorStop(1, 'rgba(230, 250, 255, 0.75)');
    ctx.strokeStyle = tail;
    ctx.lineWidth = 3 * dpr;
    ctx.beginPath();
    ctx.moveTo(cometX - w * 0.2, cometY - h * 0.075);
    ctx.lineTo(cometX, cometY);
    ctx.stroke();

    ctx.fillStyle = 'rgba(240, 252, 255, 0.95)';
    ctx.beginPath();
    ctx.arc(cometX, cometY, 3.2 * dpr, 0, Math.PI * 2);
    ctx.fill();

    const sea = ctx.createLinearGradient(0, horizon, 0, h);
    sea.addColorStop(0, '#0c2d66');
    sea.addColorStop(1, '#0b1232');
    ctx.fillStyle = sea;
    ctx.fillRect(0, horizon, w, h - horizon);

    for (let i = 0; i < 5; i += 1) {
      const y = horizon + (12 + i * 14) * dpr;
      const amp = (3 + i * 1.25) * dpr;
      const alpha = 0.22 + (i / 8);
      ctx.strokeStyle = `rgba(120, 214, 255, ${alpha})`;
      ctx.lineWidth = (1.2 + i * 0.2) * dpr;
      ctx.beginPath();
      for (let x = 0; x <= w; x += 16) {
        const wave = Math.sin((x / w) * Math.PI * (5.4 + i * 0.3) + t * (1.1 + i * 0.15)) * amp;
        if (x === 0) ctx.moveTo(x, y + wave);
        else ctx.lineTo(x, y + wave);
      }
      ctx.stroke();
    }

    for (let i = 0; i < fishSchool.length; i += 1) {
      const fish = fishSchool[i];
      const progress = (t * fish.speed + i * 0.29) % 1;
      const laneY = horizon + fish.depth * (h - horizon);
      const bob = Math.sin(t * (1.4 + i * 0.27) + i * 1.3) * (6 + i * 1.8) * dpr;
      const x = fish.dir > 0
        ? -w * 0.18 + progress * w * 1.4
        : w * 1.18 - progress * w * 1.4;
      const y = laneY + bob;

      drawFish(x, y, fish.size * dpr, fish.dir, t + i * 0.6, fish.colorA, fish.colorB);

      // Soft bubble trail behind each fish to emphasize movement.
      for (let b = 0; b < 4; b += 1) {
        const trail = progress - b * 0.013;
        const bx = fish.dir > 0
          ? -w * 0.18 + trail * w * 1.4 - 18 * dpr
          : w * 1.18 - trail * w * 1.4 + 18 * dpr;
        const by = y - b * 6 * dpr - Math.sin(t * 1.6 + b + i) * 2.5 * dpr;
        const alpha = Math.max(0, 0.24 - b * 0.04);
        if (alpha <= 0) continue;
        ctx.strokeStyle = `rgba(224, 247, 255, ${alpha})`;
        ctx.lineWidth = Math.max(1, 1.25 * dpr - b * 0.12 * dpr);
        ctx.beginPath();
        ctx.arc(bx, by, (2.1 + b * 0.45) * dpr, 0, Math.PI * 2);
        ctx.stroke();
      }
    }

    const pulse = (Math.sin(t * 2.2) + 1) / 2;
    const letterSize = Math.max(40, Math.floor(Math.min(w * 0.15, h * 0.24)));
    const subSize = Math.max(12, Math.floor(letterSize * 0.2));
    const textY = h * 0.42;
    const headline = "DON'T PANIC";

    ctx.save();
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';

    const glow = 0.28 + pulse * 0.4;
    ctx.shadowColor = `rgba(119, 222, 255, ${glow})`;
    ctx.shadowBlur = 22 * dpr;

    ctx.lineWidth = Math.max(2, letterSize * 0.055);
    ctx.strokeStyle = 'rgba(6, 22, 48, 0.72)';
    ctx.fillStyle = '#fff9de';
    ctx.font = `900 ${letterSize}px "Iowan Old Style", "Palatino Linotype", serif`;

    // Draw whole headline in layered jitter passes so kerning (including apostrophe) stays natural.
    const passCount = 5;
    for (let i = 0; i < passCount; i += 1) {
      const phase = t * (1.1 + i * 0.1);
      const wobbleX = Math.sin(phase + i * 0.7) * (1.4 + i * 0.22) * dpr;
      const wobbleY = Math.cos(phase * 1.07 + i * 0.5) * (1.1 + i * 0.18) * dpr;
      const tilt = Math.sin(phase * 0.6 + i) * 0.015;
      const alpha = 0.24 + i * 0.12;

      ctx.save();
      ctx.translate(w * 0.5 + wobbleX, textY + wobbleY);
      ctx.rotate(tilt);
      ctx.strokeStyle = `rgba(6, 22, 48, ${0.48 + i * 0.06})`;
      ctx.fillStyle = `rgba(255, 249, 222, ${Math.min(0.98, alpha)})`;
      ctx.strokeText(headline, 0, 0);
      ctx.fillText(headline, 0, 0);
      ctx.restore();
    }

    ctx.shadowBlur = 0;
    ctx.fillStyle = 'rgba(214, 241, 255, 0.95)';
    ctx.font = `700 ${subSize}px "Avenir Next", "Segoe UI", sans-serif`;
    ctx.fillText('in LARGE FRIENDLY LETTERS', w * 0.5, textY + letterSize * 0.52);

    ctx.restore();

    state.lockOverlayAnimationId = requestAnimationFrame(paint);
  };

  stopLockOverlayAnimation();
  state.lockOverlayStarDrift = 0;
  state.lockOverlayAnimationId = requestAnimationFrame(paint);
}

function showLockOverlay() {
  const overlay = byId('lock-overlay');
  if (!overlay) return;

  overlay.classList.remove('hidden');
  overlay.setAttribute('aria-hidden', 'false');
  overlay.focus();
  drawLockOverlayScene();
}

function onLockOverlayKeydown(event) {
  if (event.key === 'Escape' || event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    hideLockOverlay();
  }
}

function applyProperName() {
  updateDocumentTitle();
  const brand = byId('brand-proper-name');
  if (brand) {
    brand.textContent = PROPER_NAME;
  }
  const subtitle = byId('brand-subtitle');
  if (subtitle) {
    subtitle.textContent = BRAND_SUBTITLE_STATIC;
  }
}

function currentWorldName() {
  const alias = String(state.currentHome?.alias || '').trim();
  if (alias) return alias;
  const endpoint = String(state.currentHome?.endpointId || '').trim();
  if (endpoint) return endpoint.slice(0, 10);
  return '';
}

function updateDocumentTitle() {
  const world = currentWorldName();
  const activeTarget = String(state.activeObjectTargetAlias || '').trim();
  const context = world ? `${world}${activeTarget}` : activeTarget;
  document.title = context ? `${PROPER_NAME} - ${context}` : PROPER_NAME;
}

function humanRoomTitle(rawName) {
  const name = String(rawName || '').trim();
  if (!name) return 'Welcome';
  return name
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function trackRoomPresence(handle, did) {
  if (!handle) return;
  state.roomPresence.set(handle, { handle, did: did || '' });
  if (String(did || '').startsWith('did:ma:')) {
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

function renderAvatarPanel() {
  const list = byId('avatar-list');
  if (!list) return;
  list.innerHTML = '';
  const sorted = Array.from(state.roomPresence.values()).sort((a, b) =>
    a.handle.localeCompare(b.handle)
  );
  for (const entry of sorted) {
    const li = document.createElement('li');
    li.className = 'avatar-item';
    li.textContent = entry.handle;
    if (entry.did) li.title = entry.did;
    list.appendChild(li);
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

  if (typeof payload.room_title === 'string' && payload.room_title) {
    state.currentHome.roomTitle = payload.room_title;
  }
  if (typeof payload.room_description === 'string') {
    state.currentHome.roomDescription = payload.room_description;
    updateRoomHeading(state.currentHome.roomTitle || '', payload.room_description);
  }

  const kind = String(payload.kind || '').trim();
  if (kind === 'presence.snapshot') {
    clearRoomPresence();
    if (Array.isArray(payload.avatars)) {
      for (const avatar of payload.avatars) {
        const handle = String(avatar?.handle || '').trim();
        const did = String(avatar?.did || '').trim();
        if (handle) {
          trackRoomPresence(handle, did);
        }
      }
    }
    return;
  }

  if (kind === 'presence.join') {
    const handle = String(payload.actor_handle || '').trim();
    const did = String(payload.actor_did || '').trim();
    if (handle) {
      trackRoomPresence(handle, did);
    }
    return;
  }

  if (kind === 'presence.leave') {
    const handle = String(payload.actor_handle || '').trim();
    if (handle) {
      removeRoomPresence(handle);
    }
  }
}

function updateLocationContext() {
  updateDocumentTitle();
}

function setKuboStatus(message, kind = 'idle') {
  const el = byId('kubo-status');
  el.textContent = message;
  el.className = `status ${kind}`;
}

function setKuboInstallNoteVisible(visible, mode = 'install') {
  const note = byId('kubo-install-note');
  if (!note) return;

  const ipnsSubdomainMatch = String(window.location.hostname || '').toLowerCase().match(/^([^.]+)\.ipns\.localhost$/);
  const ipnsPathGatewayUrl = ipnsSubdomainMatch
    ? `http://localhost:${window.location.port || '8080'}/ipns/${ipnsSubdomainMatch[1]}/`
    : '';
  const origin = window.location.origin;
  const allowOrigins = Array.from(new Set([
    'http://127.0.0.1:8080',
    'http://localhost:8080',
    'http://127.0.0.1:8081',
    'http://localhost:8081',
    'http://127.0.0.1:8082',
    'http://localhost:8082',
    origin,
  ]));
  const allowOriginsJson = JSON.stringify(allowOrigins);
  const allowOriginsCmd = `ipfs config --json API.HTTPHeaders.Access-Control-Allow-Origin '${allowOriginsJson}'`;

  if (mode === 'origin-blocked') {
    note.innerHTML =
      `<p>Kubo appears to run locally, but this page origin cannot call the configured local Kubo API (typically <code>http://localhost:8080</code>).</p>` +
      `<p>Use IPFS Desktop (Settings -> IPFS Config) or Kubo CLI (<code>ipfs config</code>) to merge the generated JSON file into your config:</p>` +
      `<p>Recommended: install <code>ma-extension</code> and reload this page to proxy local Kubo API calls safely.</p>` +
      `<p><code>${origin}</code><button type="button" class="copy-origin" id="copy-kubo-origin">Copy</button></p>` +
      `<p>Open generated file from this app: <a href="./kubo-config.merge.json" target="_blank" rel="noreferrer"><code>kubo-config.merge.json</code></a></p>` +
      `<p>If Desktop editor ignores full JSON paste, edit only <code>Gateway.PublicGateways</code>. If it is <code>null</code>, replace that value with the object from <code>kubo-config.merge.json</code>.</p>`;
  } else if (mode === 'gateway-api-blocked') {
    note.innerHTML =
      `<p>Kubo is running, but API calls from gateway origin <code>${origin}</code> are blocked by Kubo security policy.</p>` +
      `<p>This is expected for pages opened from local gateway on port <code>8080</code>.</p>` +
      `<p>This app requires Kubo API at runtime (key lookup and DID publish), so gateway-only mode is not sufficient.</p>` +
      `<p>Preferred fix: install <code>ma-extension</code> and reload this page.</p>` +
      `<p>If you are in private/incognito mode, enable the extension for private windows too.</p>` +
      `<p>Fallback: open local runtime URL <code>http://127.0.0.1:8081</code> (or <code>http://localhost:8081</code>).</p>`;
  } else if (mode === 'cors-blocked') {
    note.innerHTML =
      `<p>Kubo likely runs, but CORS for this origin is missing:</p>` +
      `<p>Recommended: install <code>ma-extension</code> and reload this page.</p>` +
      `<p><code>${origin}</code><button type="button" class="copy-origin" id="copy-kubo-origin">Copy</button></p>` +
      `<p>Open generated file from this app: <a href="./kubo-config.merge.json" target="_blank" rel="noreferrer"><code>kubo-config.merge.json</code></a></p>` +
      `<p>Ensure these origins are included: <code>${allowOrigins.join('</code>, <code>')}</code></p>` +
      `<p>CLI quick-fix for origins:</p>` +
      `<p><code>${allowOriginsCmd}</code></p>` +
      `<p>Important: if this page is on <code>http://127.0.0.1:8080</code>, that exact origin must be present in Kubo allow-origin.</p>` +
      `<p>Apply this via IPFS Desktop config or Kubo CLI (<code>ipfs config</code>).</p>` +
      `<p>If <code>Gateway.PublicGateways</code> is <code>null</code>, replace it with the object from <code>kubo-config.merge.json</code> (do not paste over the entire config file).</p>` +
      `<p>To avoid subdomain behavior, open the app as <code>http://localhost:8080/ipns/&lt;key&gt;/</code>, not <code>http://&lt;key&gt;.ipns.localhost:8080/</code>.</p>` +
      (ipnsPathGatewayUrl
        ? `<p>Workaround: open the same app via localhost path gateway, which Kubo accepts better for CORS:<br><a href="${ipnsPathGatewayUrl}"><code>${ipnsPathGatewayUrl}</code></a></p>`
        : '');
  } else {
    note.innerHTML = '<p>Kubo not available. If you have not installed it yet, download from <a href="https://docs.ipfs.tech/install/" target="_blank" rel="noreferrer">IPFS/Kubo install docs</a>.</p>';
  }

  const copyBtn = byId('copy-kubo-origin');
  if (copyBtn) {
    copyBtn.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(window.location.origin);
        setSetupStatus(`Copied origin: ${window.location.origin}`);
      } catch {
        setSetupStatus(`Could not copy origin. Use: ${window.location.origin}`);
      }
    });
  }

  note.classList.toggle('hidden', !visible);
}

function setCurrentPublishInfo({ ipns = '', cid = '' } = {}) {
  const ipnsLink = byId('current-ipns-link');
  const cidLink = byId('current-cid-link');
  if (!ipnsLink || !cidLink) return;

  const persistedIpns = localStorage.getItem(LAST_PUBLISHED_IPNS_KEY) || '';
  const persistedCid = localStorage.getItem(LAST_PUBLISHED_CID_KEY) || '';

  const effectiveIpns = String(ipns || persistedIpns || '').trim();
  const effectiveCid = String(cid || persistedCid || '').trim();

  if (effectiveIpns) {
    ipnsLink.textContent = `/ipns/${effectiveIpns}`;
    ipnsLink.href = `https://ipfs.io/ipns/${effectiveIpns}/`;
  } else {
    ipnsLink.textContent = '(not published yet)';
    ipnsLink.href = '#';
  }

  if (effectiveCid) {
    cidLink.textContent = `/ipfs/${effectiveCid}`;
    cidLink.href = `https://ipfs.io/ipfs/${effectiveCid}/`;
  } else {
    cidLink.textContent = '(not published yet)';
    cidLink.href = '#';
  }

  if (effectiveIpns) {
    localStorage.setItem(LAST_PUBLISHED_IPNS_KEY, effectiveIpns);
  }
  if (effectiveCid) {
    localStorage.setItem(LAST_PUBLISHED_CID_KEY, effectiveCid);
  }
}

async function resolveIpnsToCid(ipnsName) {
  const name = String(ipnsName || '').trim();
  if (!name) return '';
  try {
    const resolved = await kuboPost('/api/v0/name/resolve', {
      arg: `/ipns/${name}`,
      recursive: 'true'
    });
    const path = String(resolved?.Path || '').trim();
    if (!path.startsWith('/ipfs/')) return '';
    return path.slice('/ipfs/'.length).split('/')[0] || '';
  } catch {
    return '';
  }
}

async function refreshHomePublishInfoFromKubo(keys = null) {
  let resolvedKeys = keys;
  if (!Array.isArray(resolvedKeys)) {
    const payload = await kuboPost('/api/v0/key/list', { l: 'true' });
    resolvedKeys = Array.isArray(payload?.Keys) ? payload.Keys : [];
  }

  const homeKey = resolvedKeys.find((k) => String(k?.Name || '') === HOME_PUBLISH_KEY_ALIAS);
  const ipns = String(homeKey?.Id || '').trim();
  if (!ipns) {
    return;
  }

  const cid = await resolveIpnsToCid(ipns);
  setCurrentPublishInfo({ ipns, cid });
}

function setSetupStatus(message) {
  byId('setup-status').textContent = message;
}

const dialogWriter = createDialogWriter({ byId, displayActor });
const { appendMessage } = dialogWriter;

// Logging system: logs are shown when debug mode is enabled
const logger = {
  log(scope, ...args) {
    if (!state.debug) return;
    const message = args
      .map(arg => {
        if (typeof arg === 'object') {
          try {
            return JSON.stringify(arg);
          } catch {
            return String(arg);
          }
        }
        return String(arg);
      })
      .join(' ');
    appendMessage('system', `[${scope}] ${message}`);
  }
};

function stopHomeEventPolling() {
  if (state.roomPollTimer) {
    clearInterval(state.roomPollTimer);
    state.roomPollTimer = null;
  }
  state.roomPollInFlight = false;
  state.inboxPollInFlight = false;
  state.pollErrorShown = false;
}

const inboundDispatcher = createInboundDispatcher({
  state,
  logger,
  appendMessage,
  displayActor,
  humanizeText,
  fetchDidDocumentJsonByDid,
  decodeChatEventMessage: decode_chat_event_message,
  decodeWhisperEventMessage: decode_whisper_event_message,
  onPresenceEvent: applyPresencePayload,
  didRoot
});

const { dispatchInboundEvent } = inboundDispatcher;

const inboxTransport = createInboxTransport({
  state,
  logger,
  startInboxListener: start_inbox_listener,
  pollInboxMessages: poll_inbox_messages,
  inspectSignedMessage: inspect_signed_message,
  dispatchInboundEvent,
  notifyMailboxMessage(entry) {
    const preview = String(entry?.content_text || '').replace(/\s+/g, ' ').slice(0, 80) || '(binary)';
    appendMessage(
      'system',
      `Mailbox +1 (#${entry.id}) from ${humanizeIdentifier(entry.from_did || '(unknown)')} type=${entry.content_type || '(unknown)'} text=${preview}`
    );
  }
});

const { ensureInboxListener, pollDirectInbox } = inboxTransport;

async function pollCurrentHomeEvents() {
  if (!state.currentHome || state.roomPollInFlight) {
    return;
  }

  state.roomPollInFlight = true;

  const home = state.currentHome;
  const pollStart = Date.now();
  
  try {
    logger.log('poll.events', `room=${home.room} since_seq=${home.lastEventSequence || 0} endpoint=${home.endpointId.slice(0, 8)}...`);
    
    const result = JSON.parse(
      await poll_world_events(
        home.endpointId,
        state.passphrase,
        state.encryptedBundle,
        state.aliasName,
        home.room,
        toSequenceBigInt(home.lastEventSequence || 0)
      )
    );
    const elapsed = Date.now() - pollStart;
    logger.log('poll.events', `response ok=${result.ok} events_count=${(result.events || []).length} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

    if (!result.ok) {
      throw new Error(result.message || 'room poll failed');
    }

    if (!state.currentHome || state.currentHome.endpointId !== home.endpointId || state.currentHome.room !== home.room) {
      logger.log('poll.events', `room context changed, discarding response`);
      return;
    }

    // Poll responses carry a full room roster so clients converge even if a push is missed.
    if (Array.isArray(result.avatars)) {
      clearRoomPresence();
      for (const avatar of result.avatars) {
        const handle = String(avatar?.handle || '').trim();
        if (handle) trackRoomPresence(handle, String(avatar?.did || ''));
      }
    }
    if (typeof result.room_did === 'string' && result.room_did) {
      state.currentHome.roomDid = result.room_did;
    }
    if (typeof result.room_title === 'string' && result.room_title) {
      state.currentHome.roomTitle = result.room_title;
    }
    if (typeof result.room_description === 'string') {
      state.currentHome.roomDescription = result.room_description;
      updateRoomHeading(state.currentHome.roomTitle || '', result.room_description);
    }

    let nextSequence = toSequenceNumber(home.lastEventSequence || 0);
    for (const event of result.events || []) {
      const eventSequence = toSequenceNumber(event.sequence);
      if (eventSequence <= nextSequence) {
        logger.log('poll.events', `skipping duplicate event seq=${eventSequence}`);
        continue;
      }
      const preview = String(event.message || event.message_cbor_b64 || '').slice(0, 40);
      logger.log('poll.events', `dispatching event seq=${eventSequence} kind=${event.kind} sender=${event.sender || '(system)'}: ${preview}`);

      // Track presence from event metadata.
      if (event.sender) {
        trackRoomPresence(event.sender, event.sender_did || '');
      }
      // Detect leave events: system message of the form "<handle> left <room>".
      if (event.kind === 'system') {
        const leaveMatch = String(event.message || '').match(/^(\S+) left /);
        if (leaveMatch) {
          removeRoomPresence(leaveMatch[1]);
        }
      }

      await dispatchInboundEvent(event);
      nextSequence = eventSequence;
    }

    state.currentHome.lastEventSequence = Math.max(
      nextSequence,
      toSequenceNumber(result.latest_event_sequence || home.lastEventSequence || 0)
    );
    state.pollErrorShown = false;
  } catch (error) {
    const elapsed = Date.now() - pollStart;
    logger.log('poll.events', `failed after ${elapsed}ms: ${error instanceof Error ? error.message : String(error)}`);
    throw error;
  } finally {
    state.roomPollInFlight = false;
  }
}

function startHomeEventPolling() {
  stopHomeEventPolling();
  state.roomPollTimer = setInterval(() => {
    Promise.resolve()
      .then(() =>
        pollDirectInbox().catch((error) => {
          logger.log('inbox.poll', `non-fatal inbox poll failure: ${error instanceof Error ? error.message : String(error)}`);
        })
      )
      .then(() => pollCurrentHomeEvents())
      .catch((error) => {
      if (state.debug) {
        logger.log('poll.error', `room event poll failed: ${error instanceof Error ? error.message : String(error)}`);
      }
      const home = state.currentHome;
      if (home) {
        stopHomeEventPolling();
        logger.log('reconnect', `poll failed, attempting re-entry to ${home.endpointId.slice(0, 8)}...`);
        if (!state.pollErrorShown) {
          appendSystemUi(
            'Connection lost. Attempting re-entry...',
            'Mistet forbindelse. Forsøker re-entry...'
          );
          state.pollErrorShown = true;
        }
        const fallbackTarget = buildCurrentHomeResumeTarget() || home.endpointId;
        delay(RECONNECT_DELAY_MS)
          .then(() => enterHome(fallbackTarget, home.room))
          .then(() => {
            state.pollErrorShown = false;
            appendSystemUi('Re-entry complete.', 'Re-entry fullført.');
          })
          .catch(err => {
            appendMessage('system', uiText(
              `Re-entry failed: ${err instanceof Error ? err.message : String(err)}`,
              `Re-entry feilet: ${err instanceof Error ? err.message : String(err)}`
            ));
            startHomeEventPolling();
          });
      } else {
        appendMessage('system', `Room sync failed: ${error instanceof Error ? error.message : String(error)}`);
        state.pollErrorShown = true;
      }
      });
  }, ROOM_POLL_INTERVAL_MS);
}

function updateIdentityLine() {
  updateLocationContext();
}

function showChat() {
  byId('setup-view').classList.add('hidden');
  byId('chat-view').classList.remove('hidden');
  byId('session-tools').classList.remove('hidden');
  const brandRow = document.querySelector('.brand-row');
  if (brandRow) brandRow.classList.add('hidden');
  updateIdentityLine();

  const aliases = Object.keys(state.aliasBook).length;
  appendMessage('system', `Saved aliases: ${aliases}. Use .help for commands.`);
  if (state.debug) {
    logger.log('app', 'debug mode is on');
  }
  byId('command-input').focus();
}

async function runSmokeTest(targetAlias) {
  if (!state.identity) {
    throw new Error('Load or create an identity before running smoke test.');
  }

  const alias = String(targetAlias || state.currentHome?.alias || 'home').trim();
  if (!alias) {
    throw new Error('Usage: .smoke [alias]');
  }

  const marker = `smoke-${Date.now().toString(36)}`;
  appendMessage('system', `Smoke: enter ${alias} -> send marker -> poll`);

  await enterHome(alias);

  if (!state.currentHome) {
    throw new Error('Smoke failed: no active home after enter.');
  }

  const sendResult = JSON.parse(
    await withTimeout(
      send_world_cmd(
        state.currentHome.endpointId,
        state.passphrase,
        state.encryptedBundle,
        state.aliasName,
        state.currentHome.room,
        state.languagePreferences,
        marker
      ),
      12000,
      'smoke send timed out'
    )
  );

  if (!sendResult.ok) {
    throw new Error(`Smoke failed: send returned ok=false (${sendResult.message || 'no message'})`);
  }

  const beforeSeq = toSequenceNumber(state.currentHome.lastEventSequence || 0);
  await withTimeout(pollCurrentHomeEvents(), 12000, 'smoke poll timed out');
  const afterSeq = toSequenceNumber(state.currentHome.lastEventSequence || 0);

  appendMessage(
    'system',
    `Smoke PASS: enter ok, send ok (broadcasted=${Boolean(sendResult.broadcasted)}), sequence ${beforeSeq} -> ${afterSeq}, marker=${marker}`
  );
}

function showSetup() {
  stopHomeEventPolling();
  byId('session-tools').classList.add('hidden');
  byId('chat-view').classList.add('hidden');
  byId('setup-view').classList.remove('hidden');
  const brandRow = document.querySelector('.brand-row');
  if (brandRow) brandRow.classList.remove('hidden');
  updateLocationContext();
}

function saveAliasBook() {
  identityStore.saveAliasBook(ALIAS_BOOK_KEY, state.aliasBook);
}

function loadAliasBook() {
  return identityStore.loadAliasBook(ALIAS_BOOK_KEY);
}

function resolveCurrentPositionTarget() {
  if (!state.currentHome) {
    return '';
  }

  const roomDid = String(state.currentHome.roomDid || '').trim();
  if (roomDid.startsWith('did:ma:') && !isUnconfiguredDidTarget(roomDid)) {
    return roomDid;
  }

  const worldDid = didRoot(findDidByEndpoint(state.currentHome.endpointId) || '');
  if (worldDid) {
    const room = String(state.currentHome.room || 'lobby').trim() || 'lobby';
    return `${worldDid}#${room}`;
  }

  return '';
}

function isValidAliasName(aliasName) {
  return /^[a-z0-9_-]{2,32}$/i.test(String(aliasName || '').trim());
}

function isPrintableAliasLabel(label) {
  const value = String(label || '').trim();
  if (!value) return false;
  // Allow any printable Unicode label, excluding control/format/surrogate chars and spaces.
  if (/[\p{Cc}\p{Cf}\p{Cs}\s]/u.test(value)) return false;
  return value.length <= 64;
}

function normalizeLanguageTag(value) {
  const normalized = String(value || '').trim().replace(/-/g, '_');
  if (!normalized) {
    return DEFAULT_LANG;
  }
  if (!/^[A-Za-z0-9_]+$/.test(normalized)) {
    return DEFAULT_LANG;
  }
  return normalized;
}

function normalizeLanguagePreferences(value) {
  const items = String(value || '')
    .split(':')
    .map((entry) => normalizeLanguageTag(entry))
    .filter((entry, index, arr) => entry && arr.indexOf(entry) === index);
  if (items.length === 0) {
    return DEFAULT_LANGUAGE_PREFERENCES;
  }
  return items.join(':');
}

function normalizeUiLang(value) {
  const normalized = String(value || '').trim().replace(/_/g, '-').toLowerCase();
  if (['nb', 'nb-no', 'no'].includes(normalized)) {
    return 'nb';
  }
  if (['en', 'en-us', 'en-gb'].includes(normalized)) {
    return 'en';
  }
  if (['se', 'sv', 'sv-se'].includes(normalized)) {
    return 'se';
  }
  if (['da', 'da-dk'].includes(normalized)) {
    return 'da';
  }
  return '';
}

function uiLangFromLanguage(languageValue) {
  const lang = normalizeLanguageTag(languageValue).toLowerCase();
  if (lang.startsWith('nb') || lang.startsWith('nn') || lang === 'no') {
    return 'nb';
  }
  if (lang.startsWith('en')) {
    return 'en';
  }
  return DEFAULT_UI_LANG;
}

function setUiLanguage(value) {
  const normalized = normalizeUiLang(value) || uiLangFromLanguage(state.lang);
  state.uiLang = normalized;
  if (typeof document !== 'undefined' && document.documentElement) {
    document.documentElement.lang = normalized;
  }
}

function uiText(enText, nbText) {
  return state.uiLang === 'nb' ? nbText : enText;
}

function appendSystemUi(enText, nbText) {
  appendMessage('system', uiText(enText, nbText));
}

const identityStore = createIdentityStore({
  storagePrefix: STORAGE_PREFIX,
  legacy: {
    aliasKey: LEGACY_ALIAS_KEY,
    bundleKey: LEGACY_BUNDLE_KEY,
    recoveryPhraseKey: 'ma.identity.v2.recoveryPhrase',
    defaultLang: DEFAULT_LANG,
    defaultLanguage: DEFAULT_LANGUAGE_PREFERENCES
  },
  isValidAliasName,
  normalizeLanguageTag,
  normalizeLanguagePreferences
});

function setLanguageSelection(langValue, languageListValue) {
  const lang = normalizeLanguageTag(langValue);
  const language = normalizeLanguagePreferences(languageListValue || lang);
  byId('actor-language').value = lang;
  const languageListInput = byId('actor-language-list');
  if (languageListInput) {
    languageListInput.value = language;
  }
  state.lang = lang;
  state.languagePreferences = language;
  setUiLanguage(uiLangFromLanguage(lang));
}

function toSequenceNumber(value) {
  if (typeof value === 'bigint') {
    return Number(value);
  }

  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : 0;
}

function toSequenceBigInt(value) {
  if (typeof value === 'bigint') {
    return value;
  }

  const numeric = toSequenceNumber(value);
  return BigInt(Math.max(0, Math.floor(numeric)));
}

function saveIdentityRecord(aliasName, encryptedBundle) {
  identityStore.saveIdentityRecord(
    aliasName,
    encryptedBundle,
    byId('actor-language').value,
    byId('actor-language-list')?.value || byId('actor-language').value
  );
}

function resolveIdentityRecord(aliasName) {
  return identityStore.resolveIdentityRecord(aliasName);
}

function scrubStoredRecoveryPhrases() {
  identityStore.scrubStoredRecoveryPhrases();
}

function setActiveAlias(aliasName) {
  identityStore.setActiveAlias(aliasName, TAB_ALIAS_KEY, LAST_ALIAS_KEY);
}

function resolveInitialAlias() {
  return identityStore.resolveInitialAlias(TAB_ALIAS_KEY, LAST_ALIAS_KEY);
}

function loadAliasDraft(aliasName, options = {}) {
  const persistActive = options.persistActive !== false;
  const normalized = String(aliasName || '').trim();
  if (!normalized) {
    byId('bundle-text').value = '';
    if (!byId('recovery-phrase').value.trim()) {
      onNewPhrase();
    }
    return;
  }

  if (persistActive) {
    setActiveAlias(normalized);
  }
  const record = resolveIdentityRecord(normalized);
  byId('bundle-text').value = record?.encryptedBundle || '';
  setLanguageSelection(record?.lang || DEFAULT_LANG, record?.language || record?.lang || DEFAULT_LANGUAGE_PREFERENCES);

  if (!byId('recovery-phrase').value.trim()) {
    onNewPhrase();
  }
}

function exportBundle() {
  if (!state.encryptedBundle) {
    appendMessage('system', 'No bundle loaded in memory to export.');
    return;
  }
  const blob = new Blob([state.encryptedBundle], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `ma-identity-${state.aliasName || 'bundle'}.json`;
  a.click();
  URL.revokeObjectURL(url);
  appendMessage('system', `Bundle exported as ${a.download}`);
}

function getApiBase() {
  return (byId('kubo-api').value.trim() || 'http://localhost:8080').replace(/\/$/, '');
}

function nextBridgeRequestId() {
  bridgeRequestCounter += 1;
  return `ma-kubo-${Date.now()}-${bridgeRequestCounter}`;
}

function isBridgeCapable() {
  return typeof window !== 'undefined' && typeof window.postMessage === 'function';
}

function installBridgeMonitor() {
  if (bridgeMonitorInstalled || typeof window === 'undefined') {
    return;
  }
  bridgeMonitorInstalled = true;
  window.addEventListener('message', (event) => {
    if (event.source !== window) return;
    const data = event.data || {};
    if (data.type === BRIDGE_READY_TYPE) {
      bridgeReadySeen = true;
    }
  });
}

function bridgeRequest(payload, timeoutMs = BRIDGE_TIMEOUT_MS) {
  if (!isBridgeCapable()) {
    return Promise.reject(new Error('Bridge is not available in this runtime.'));
  }

  return new Promise((resolve, reject) => {
    const requestId = nextBridgeRequestId();
    let done = false;

    function finishWithError(error) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      window.removeEventListener('message', onMessage);
      reject(error);
    }

    function finishWithSuccess(result) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      window.removeEventListener('message', onMessage);
      resolve(result);
    }

    function onMessage(event) {
      if (event.source !== window) return;
      const data = event.data || {};
      if (data.type !== BRIDGE_RESPONSE_TYPE || data.requestId !== requestId) return;
      if (!data.ok) {
        finishWithError(new Error(String(data.error || 'Bridge call failed.')));
        return;
      }
      finishWithSuccess(data.result);
    }

    const timer = setTimeout(() => {
      finishWithError(new Error('Bridge timeout. Is ma-extension installed and active?'));
    }, timeoutMs);

    window.addEventListener('message', onMessage);
    window.postMessage({ type: BRIDGE_REQUEST_TYPE, requestId, payload }, '*');
  });
}

async function kuboPostViaBridge(base, path, queryParams, body = null) {
  const query = queryParams instanceof URLSearchParams
    ? queryParams.toString()
    : new URLSearchParams(queryParams || {}).toString();

  let serializedBody = { kind: 'none' };
  if (typeof body === 'string') {
    serializedBody = { kind: 'text', text: body };
  } else if (body instanceof FormData) {
    const entries = [];
    for (const [name, value] of body.entries()) {
      if (typeof value === 'string') {
        entries.push({ name, type: 'text', value });
      } else {
        const text = await value.text();
        entries.push({
          name,
          type: 'file',
          filename: value.name || 'upload.bin',
          contentType: value.type || 'application/octet-stream',
          value: text
        });
      }
    }
    serializedBody = { kind: 'formData', entries };
  }

  return bridgeRequest({
    base,
    path,
    query,
    body: serializedBody,
    pageOrigin: window.location.origin
  });
}

async function kuboPost(path, query = {}, body = null) {
  const base = getApiBase();
  const isLocalKubo = /^https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/i.test(base);
  const isRemotePage = !isLocalhostLikeHost(window.location.hostname);
  const params = query instanceof URLSearchParams
    ? query
    : new URLSearchParams(query);
  const url = `${base}${path}${params.toString() ? `?${params.toString()}` : ''}`;

  if (isLocalKubo) {
    try {
      return await kuboPostViaBridge(base, path, params, body);
    } catch (bridgeError) {
      if (isGatewayViewOrigin()) {
        const detail = bridgeError instanceof Error ? bridgeError.message : String(bridgeError || '');
        const blocked = new Error(`ma-extension bridge is unavailable from this tab${detail ? ` (${detail})` : ''}.`);
        blocked.kuboReason = 'bridge-unavailable';
        blocked.bridgeDetail = detail;
        throw blocked;
      }
      // Fallback to direct browser fetch when extension bridge is unavailable.
    }
  }

  let response;

  try {
    response = await fetch(url, {
      method: 'POST',
      body
    });
  } catch (error) {
    if (error instanceof TypeError) {
      if (isLocalKubo && isRemotePage) {
        const blocked = new Error('Browser blocked access to local Kubo from this origin.');
        blocked.kuboReason = 'remote-localhost-block';
        throw blocked;
      }
      if (isLocalKubo && !isRemotePage) {
        const blocked = new Error('Local Kubo call failed from a localhost-like origin.');
        blocked.kuboReason = 'localhost-cors-or-network';
        throw blocked;
      }
      const mixedContentHint = window.location.protocol === 'https:'
        ? 'This page is loaded over HTTPS; browsers often block http://127.0.0.1 mixed-content requests.'
        : '';
      throw new Error(`Unable to reach Kubo API from browser. Check API URL, ensure Kubo is running, and allow CORS for the app origin and headers. ${mixedContentHint}`.trim());
    }
    throw error;
  }

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Kubo API ${response.status}: ${text || response.statusText}`);
  }

  try {
    return await response.json();
  } catch {
    const text = await response.text();
    throw new Error(`Kubo API returned non-JSON response: ${text || '(empty body)'}`);
  }
}

async function kuboPostText(path, query = {}, body = null) {
  const base = getApiBase();
  const isLocalKubo = /^https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/i.test(base);
  const isRemotePage = !isLocalhostLikeHost(window.location.hostname);
  const params = query instanceof URLSearchParams
    ? query
    : new URLSearchParams(query);
  const url = `${base}${path}${params.toString() ? `?${params.toString()}` : ''}`;

  if (isLocalKubo) {
    try {
      const bridged = await kuboPostViaBridge(base, path, params, body);
      if (typeof bridged === 'string') {
        return bridged;
      }
      if (bridged && typeof bridged.__rawText === 'string') {
        return bridged.__rawText;
      }
      if (bridged && typeof bridged === 'object') {
        return JSON.stringify(bridged);
      }
      return String(bridged || '');
    } catch (bridgeError) {
      if (isGatewayViewOrigin()) {
        const detail = bridgeError instanceof Error ? bridgeError.message : String(bridgeError || '');
        const blocked = new Error(`ma-extension bridge is unavailable from this tab${detail ? ` (${detail})` : ''}.`);
        blocked.kuboReason = 'bridge-unavailable';
        blocked.bridgeDetail = detail;
        throw blocked;
      }
      // Fallback to direct browser fetch when extension bridge is unavailable.
    }
  }

  let response;
  try {
    response = await fetch(url, {
      method: 'POST',
      body
    });
  } catch (error) {
    if (error instanceof TypeError) {
      if (isLocalKubo && isRemotePage) {
        const blocked = new Error('Browser blocked access to local Kubo from this origin.');
        blocked.kuboReason = 'remote-localhost-block';
        throw blocked;
      }
      if (isLocalKubo && !isRemotePage) {
        const blocked = new Error('Local Kubo call failed from a localhost-like origin.');
        blocked.kuboReason = 'localhost-cors-or-network';
        throw blocked;
      }
      const mixedContentHint = window.location.protocol === 'https:'
        ? 'This page is loaded over HTTPS; browsers often block http://127.0.0.1 mixed-content requests.'
        : '';
      throw new Error(`Unable to reach Kubo API from browser. Check API URL, ensure Kubo is running, and allow CORS for the app origin and headers. ${mixedContentHint}`.trim());
    }
    throw error;
  }

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Kubo API ${response.status}: ${text || response.statusText}`);
  }

  return await response.text();
}

async function checkKubo() {
  setKuboStatus('checking...', 'working');
  try {
    const payload = await kuboPost('/api/v0/key/list', { l: 'true' });
    const keys = Array.isArray(payload?.Keys) ? payload.Keys : [];
    setKuboStatus(`connected (${keys.length} keys)`, 'ok');
    setSetupActionsEnabled(true);
    setKuboInstallNoteVisible(false);
    await refreshHomePublishInfoFromKubo(keys);
    setSetupStatus('Kubo API reachable.');
    return keys;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const isLocalGatewayOrigin = isGatewayViewOrigin();
    if (isLocalGatewayOrigin && message.startsWith('Kubo API 403')) {
      setKuboStatus('blocked by Kubo gateway policy', 'warn');
      setSetupActionsEnabled(false);
      setKuboInstallNoteVisible(true, 'gateway-api-blocked');
      setSetupStatus('Kubo API is required, but blocked on gateway origin (:8080). Open local runtime URL on :8081.');
      throw error;
    }
    if (error && error.kuboReason === 'bridge-unavailable') {
      setKuboStatus('extension bridge missing', 'warn');
      setSetupActionsEnabled(false);
      setKuboInstallNoteVisible(true, 'gateway-api-blocked');
      const detail = error.bridgeDetail ? ` (${error.bridgeDetail})` : '';
      setSetupStatus(`ma-extension is not active in this tab${detail}. Install/enable extension (also in private mode), then reload.`);
      throw error;
    }
    if (error && error.kuboReason === 'remote-localhost-block') {
      setKuboStatus('blocked from this origin', 'warn');
      setSetupActionsEnabled(false);
      setKuboInstallNoteVisible(true, 'origin-blocked');
      setSetupStatus('This browser tab cannot call local Kubo from the current origin.');
      throw error;
    }
    if (error && error.kuboReason === 'localhost-cors-or-network') {
      if (isLocalGatewayOrigin) {
        setKuboStatus('blocked by Kubo gateway policy', 'warn');
        setSetupActionsEnabled(false);
        setKuboInstallNoteVisible(true, 'gateway-api-blocked');
        setSetupStatus('Kubo API is required, but blocked on gateway origin (:8080). Open local runtime URL on :8081.');
        throw error;
      }
      setKuboStatus('blocked by CORS/network', 'warn');
      setSetupActionsEnabled(false);
      setKuboInstallNoteVisible(true, 'cors-blocked');
      setSetupStatus(`Kubo API call failed from localhost-like origin. Add ${window.location.origin} to Kubo CORS allow-origin.`);
      throw error;
    }
    setKuboStatus('not reachable from browser', 'error');
    setSetupActionsEnabled(false);
    setKuboInstallNoteVisible(true, 'install');
    setSetupStatus(message);
    throw error;
  }
}

async function ensureKuboAliasKey(aliasName) {
  const keys = await checkKubo();
  const found = keys.find((k) => k.Name === aliasName);
  if (found) {
    return found.Id;
  }

  const created = await kuboPost('/api/v0/key/gen', {
    arg: aliasName,
    type: 'ed25519'
  });

  return created.Id;
}

function validateSetupInputs(requireBundle) {
  const aliasName = byId('alias-name').value.trim();
  const passphrase = byId('passphrase').value;
  const bundle = byId('bundle-text').value.trim();

  if (!isValidAliasName(aliasName)) {
    throw new Error('Alias must be 2-32 chars using letters, numbers, underscore, or dash.');
  }
  if (passphrase.length < 8) {
    throw new Error('Passphrase must be at least 8 characters.');
  }
  if (requireBundle && !bundle) {
    throw new Error('Provide an encrypted bundle to unlock.');
  }

  return { aliasName, passphrase, bundle };
}

function generateRecoveryPhrase(wordCount = 12) {
  return generate_bip39_phrase(wordCount);
}

function normalizeRecoveryPhrase(input) {
  const value = String(input || '').trim();
  if (!value) {
    return '';
  }
  return normalize_bip39_phrase(value);
}

function resolveRecoveryPhraseFromInput() {
  const raw = byId('recovery-phrase').value;
  const trimmed = String(raw || '').trim();
  if (!trimmed) {
    return generateRecoveryPhrase(12);
  }
  return normalizeRecoveryPhrase(trimmed);
}

async function onCreateIdentity() {
  setSetupStatus('Creating identity...');
  try {
    const { aliasName, passphrase } = validateSetupInputs(false);
    const lang = normalizeLanguageTag(byId('actor-language').value);
    const language = normalizeLanguagePreferences(byId('actor-language-list')?.value || lang);
    localStorage.setItem(API_KEY, getApiBase());
    setActiveAlias(aliasName);

    const created = JSON.parse(create_identity(passphrase));
    const localized = JSON.parse(set_bundle_language_preferences(passphrase, created.encrypted_bundle, lang, language));
    const result = JSON.parse(ensure_bundle_iroh_secret(passphrase, localized.encrypted_bundle));

    state.identity = result;
    state.encryptedBundle = result.encrypted_bundle;
    state.passphrase = passphrase;
    state.aliasName = aliasName;
    state.lang = lang;
    state.languagePreferences = language;
    loadBlockedDidRootsForIdentity(result.did);
    setCurrentPublishInfo({ ipns: result.ipns || '' });

    byId('bundle-text').value = result.encrypted_bundle;

    const phrase = resolveRecoveryPhraseFromInput();
    byId('recovery-phrase').value = phrase;
    saveIdentityRecord(aliasName, result.encrypted_bundle);

    setSetupStatus('Identity created and unlocked.');
    showChat();
    restoreActiveHomeAfterUnlock().catch((err) => {
      appendMessage('system', `Restore failed: ${err instanceof Error ? err.message : String(err)}`);
    });
  } catch (error) {
    setSetupStatus(error instanceof Error ? error.message : String(error));
  }
}

async function onUnlockIdentity() {
  setSetupStatus('Unlocking bundle...');
  try {
    const { aliasName, passphrase, bundle } = validateSetupInputs(true);
    const lang = normalizeLanguageTag(byId('actor-language').value);
    const language = normalizeLanguagePreferences(byId('actor-language-list')?.value || lang);
    localStorage.setItem(API_KEY, getApiBase());
    setActiveAlias(aliasName);

    const unlocked = JSON.parse(unlock_identity(passphrase, bundle));

    const localized = JSON.parse(set_bundle_language_preferences(passphrase, bundle, lang, language));
    const updated = JSON.parse(ensure_bundle_iroh_secret(passphrase, localized.encrypted_bundle));

    state.identity = updated;
    state.encryptedBundle = updated.encrypted_bundle;
    state.passphrase = passphrase;
    state.aliasName = aliasName;
    state.lang = lang;
    state.languagePreferences = language;
    loadBlockedDidRootsForIdentity(updated.did);
    setCurrentPublishInfo({ ipns: updated.ipns || '' });

    byId('bundle-text').value = updated.encrypted_bundle;

    const phrase = resolveRecoveryPhraseFromInput();
    byId('recovery-phrase').value = phrase;
    saveIdentityRecord(aliasName, updated.encrypted_bundle);

    setSetupStatus('Bundle unlocked.');
    showChat();
    restoreActiveHomeAfterUnlock().catch((err) => {
      appendMessage('system', `Restore failed: ${err instanceof Error ? err.message : String(err)}`);
    });
  } catch (error) {
    setSetupStatus(error instanceof Error ? error.message : String(error));
  }
}

function onNewPhrase() {
  const phrase = generateRecoveryPhrase(12);
  byId('recovery-phrase').value = phrase;

  const aliasName = byId('alias-name').value.trim();
  const bundle = byId('bundle-text').value.trim();
  if (isValidAliasName(aliasName)) {
    saveIdentityRecord(aliasName, bundle);
  }
}

function onLanguageChange() {
  const lang = normalizeLanguageTag(byId('actor-language').value);
  const language = normalizeLanguagePreferences(byId('actor-language-list')?.value || lang);
  applyLanguageChange(lang, language).catch((error) => {
    appendMessage('system', `Language update failed: ${error instanceof Error ? error.message : String(error)}`);
  });
}

async function applyLanguageChange(langValue, languageValue) {
  const lang = normalizeLanguageTag(langValue);
  const language = normalizeLanguagePreferences(languageValue || lang);
  setLanguageSelection(lang, language);

  const aliasName = (state.aliasName || byId('alias-name').value || '').trim();
  const phrase = byId('recovery-phrase').value.trim();
  const passphrase = byId('passphrase').value;

  if (state.identity && state.encryptedBundle && passphrase.length >= 8) {
    const updated = JSON.parse(
      set_bundle_language_preferences(passphrase, state.encryptedBundle, lang, language)
    );
    state.identity = updated;
    state.encryptedBundle = updated.encrypted_bundle;
    byId('bundle-text').value = updated.encrypted_bundle;
  }

  if (isValidAliasName(aliasName)) {
    saveIdentityRecord(aliasName, byId('bundle-text').value.trim());
  }

  updateIdentityLine();
  setSetupStatus(`Actor language preferences set to ${lang} / ${language}.`);
}

function lockSession() {
  saveActiveHomeSnapshot();
  stopHomeEventPolling();
  disconnect_world().catch(() => {});
  state.identity = null;
  state.encryptedBundle = '';
  state.passphrase = '';
  state.currentHome = null;
  state.didDocCache.clear();
  state.blockedDidRoots = new Set();
  clearRoomPresence();
  byId('transcript').innerHTML = '';
  setSetupStatus('Session locked. Bundle remains stored unless removed manually.');
  showSetup();
  showLockOverlay();
}

function normalizeIrohAddress(address) {
  const value = String(address || '').trim();
  if (!value) return '';
  if (value.startsWith('/iroh-ma/')) {
    // Strip /iroh-ma/ prefix and any ALPN suffix after the node id
    return value.slice('/iroh-ma/'.length).split('/')[0];
  }
  if (value.startsWith('/ma-iroh/')) {
    // Strip /ma-iroh/ prefix and any ALPN suffix after the node id
    return value.slice('/ma-iroh/'.length).split('/')[0];
  }
  if (value.startsWith('/iroh+ma/')) {
    // Strip /iroh+ma/ prefix and any ALPN suffix after the node id
    return value.slice('/iroh+ma/'.length).split('/')[0];
  }
  if (value.startsWith('/iroh/')) {
    // Strip /iroh/ prefix and any ALPN suffix after the node id
    return value.slice('/iroh/'.length).split('/')[0];
  }
  return value;
}

function normalizeEndpointId(address) {
  const normalized = alias_normalize_endpoint_id(address);
  if (!normalized) {
    return '';
  }
  return normalized;
}

function didRoot(input) {
  return alias_did_root(String(input || ''));
}

function findDidByEndpoint(endpointLike) {
  try {
    return alias_find_did_by_endpoint(
      String(endpointLike || ''),
      JSON.stringify(state.didEndpointMap || {})
    );
  } catch {
    return '';
  }
}

function findAliasForAddress(address) {
  try {
    return alias_find_alias_for_address(
      String(address || ''),
      JSON.stringify(state.aliasBook || {})
    );
  } catch {
    return '';
  }
}

function resolveAliasInput(value) {
  try {
    return alias_resolve_input(
      String(value || ''),
      JSON.stringify(state.aliasBook || {})
    );
  } catch {
    return String(value || '').trim();
  }
}

function humanizeIdentifier(value) {
  try {
    return alias_humanize_identifier(
      String(value || ''),
      JSON.stringify(state.aliasBook || {})
    );
  } catch {
    return String(value || '').trim();
  }
}

function humanizeText(text) {
  try {
    return alias_humanize_text(
      String(text || ''),
      JSON.stringify(state.aliasBook || {})
    );
  } catch {
    return String(text || '');
  }
}

function didToIpnsName(did) {
  const root = didRoot(did);
  const prefix = 'did:ma:';
  if (!root.startsWith(prefix)) {
    throw new Error(`Unsupported DID method for ${did}`);
  }
  return root.slice(prefix.length);
}

async function fetchDidDocumentJsonByDid(did) {
  const rootDid = didRoot(did);
  const cached = state.didDocCache.get(rootDid);
  if (cached && Date.now() - cached.fetchedAt < DID_DOC_CACHE_TTL_MS) {
    logger.log('did.cache', `hit for ${rootDid}`);
    return cached.documentJson;
  }

  logger.log('did.cache', `miss for ${rootDid}`);
  const ipns = didToIpnsName(rootDid);
  const resolved = await kuboPost('/api/v0/name/resolve', {
    arg: `/ipns/${ipns}`,
    recursive: 'true'
  });
  const path = String(resolved?.Path || '').trim();
  if (!path.startsWith('/ipfs/')) {
    throw new Error(`name/resolve did not return /ipfs path for ${rootDid}`);
  }

  const documentJson = await kuboPostText('/api/v0/cat', { arg: path });
  state.didDocCache.set(rootDid, {
    fetchedAt: Date.now(),
    documentJson
  });
  return documentJson;
}

function parseDidDocument(jsonText) {
  try {
    return JSON.parse(jsonText);
  } catch {
    return null;
  }
}

function extractEndpointFromTransportEntry(entry) {
  if (!entry) return '';
  if (typeof entry === 'string') {
    const endpoint = normalizeIrohAddress(entry);
    return isLikelyIrohAddress(endpoint) ? endpoint : '';
  }
  if (typeof entry !== 'object') {
    return '';
  }

  const candidates = [
    entry.endpoint_id,
    entry.endpointId,
    entry.iroh,
    entry.address,
    entry.currentInbox,
    entry.current_inbox,
    entry.presence_hint,
    entry.presenceHint
  ];
  for (const candidate of candidates) {
    const endpoint = normalizeIrohAddress(candidate || '');
    if (isLikelyIrohAddress(endpoint)) {
      return endpoint;
    }
  }

  return '';
}

function extractWorldEndpointFromDidDoc(document) {
  if (!document || typeof document !== 'object') {
    return '';
  }

  const ma = document.ma && typeof document.ma === 'object' ? document.ma : null;

  const transports = ma?.transports;
  if (Array.isArray(transports)) {
    for (const entry of transports) {
      const endpoint = extractEndpointFromTransportEntry(entry);
      if (endpoint) {
        return endpoint;
      }
    }
  } else {
    const endpoint = extractEndpointFromTransportEntry(transports);
    if (endpoint) {
      return endpoint;
    }
  }

  const inbox = normalizeIrohAddress(ma?.currentInbox || ma?.current_inbox || '');
  if (isLikelyIrohAddress(inbox)) {
    return inbox;
  }

  const fallback = normalizeIrohAddress(ma?.presenceHint || '');
  if (isLikelyIrohAddress(fallback)) {
    return fallback;
  }

  return '';
}

function parseEnterDirective(message) {
  const text = String(message || '');
  const match = text.match(/(?:^|\s)go\s+(did:ma:[^\s]+)/i);
  if (!match) {
    return null;
  }
  const rawDid = String(match[1] || '').replace(/[),.;]+$/, '');
  if (!rawDid.startsWith('did:ma:')) {
    return null;
  }
  return rawDid;
}

async function autoFollowEnterDirective(message) {
  const targetDid = parseEnterDirective(message);
  if (!targetDid) {
    return;
  }

  const targetRoot = didRoot(targetDid);
  const roomFragment = targetDid.includes('#') ? targetDid.split('#')[1] : '';

  // Resolve target DID first; if it points at a world via ma:world, use that world doc.
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

function displayActor(senderDid, senderHandle) {
  const fullDid = String(senderDid || '').trim();
  const root = didRoot(fullDid);
  const alias = findAliasForAddress(fullDid) || findAliasForAddress(root) || '';
  if (alias) return alias;
  if (fullDid) return fullDid;
  if (root) return root;
  if (senderHandle) return senderHandle;
  return 'unknown';
}

function currentActorDid() {
  return String(state.identity?.did || '').trim();
}

function renderLocalBroadcastMessage(text) {
  const senderDid = currentActorDid();
  const actor = displayActor(senderDid, state.aliasName);
  appendMessage('world', humanizeText(`${actor}: ${text}`));
}

function updateRoomHeading(title, desc) {
  const heading = byId('room-heading');
  const description = byId('room-description');
  if (!heading || !description) return;

  const resolvedTitle = String(title || '').trim()
    || String(state.currentHome?.roomTitle || '').trim()
    || humanRoomTitle(state.currentHome?.room || '');
  const resolvedDescription = String(desc || '').trim()
    || String(state.currentHome?.roomDescription || '').trim();

  heading.textContent = resolvedTitle;
  description.textContent = resolvedDescription;
  updateLocationContext();
}

function normalizeUseRequirement(requirement) {
  const value = String(requirement || '').trim().toLowerCase();
  if (value === 'held') return 'held';
  return 'none';
}

function setActiveObjectTarget(alias, did, requirement = 'none') {
  const normalizedAlias = String(alias || '').trim();
  const normalizedDid = String(did || '').trim();
  if (!normalizedAlias.startsWith('@') || !normalizedDid.startsWith('did:ma:')) {
    return;
  }
  state.activeObjectTargetAlias = normalizedAlias;
  state.activeObjectTargetDid = normalizedDid;
  state.activeObjectTargetRequirement = normalizeUseRequirement(requirement);
  updateLocationContext();
}

function clearActiveObjectTarget(alias = '') {
  const token = String(alias || '').trim();
  if (!token) {
    state.activeObjectTargetAlias = '';
    state.activeObjectTargetDid = '';
    state.activeObjectTargetRequirement = 'none';
    updateLocationContext();
    return;
  }
  if (state.activeObjectTargetAlias && state.activeObjectTargetAlias.toLowerCase() === token.toLowerCase()) {
    state.activeObjectTargetAlias = '';
    state.activeObjectTargetDid = '';
    state.activeObjectTargetRequirement = 'none';
    updateLocationContext();
  }
}

function refillCommandInputWithActiveTarget() {
  const inputEl = byId('command-input');
  if (!inputEl) return;
  const alias = String(state.activeObjectTargetAlias || '').trim();
  if (!alias) {
    inputEl.value = '';
    return;
  }
  inputEl.value = `${alias} `;
  inputEl.setSelectionRange(inputEl.value.length, inputEl.value.length);
}

function shouldAutoPrefixActiveTarget(text) {
  const source = String(text || '').trim();
  if (!source) return false;
  if (source.startsWith('.')) return false;
  if (source.startsWith('@')) return false;
  if (source.startsWith("'")) return false;
  return true;
}

function maybePrefixActiveObjectTarget(text) {
  const alias = String(state.activeObjectTargetAlias || '').trim();
  const did = String(state.activeObjectTargetDid || '').trim();
  if (!alias) return String(text || '');
  if (!shouldAutoPrefixActiveTarget(text)) return String(text || '');
  const target = did.startsWith('did:ma:') ? did : alias;
  return `@${target} ${String(text || '').trim()}`;
}

async function ensureHeldRequirementSatisfied(alias, objectDid) {
  const normalizedDid = String(objectDid || '').trim();
  if (!normalizedDid.startsWith('did:ma:')) return;
  const response = await sendWorldCommandQuery(`@${normalizedDid} show`);
  const kv = parseKeyValuePairs(response);
  const holder = String(kv.holder || '').trim();
  const currentHandle = String(state.currentHome?.handle || state.aliasName || '').trim();
  const aliasLabel = String(alias || '').trim() || '@object';
  if (!holder || holder === '(none)' || !currentHandle || holder !== currentHandle) {
    throw new Error(uiText(
      `${aliasLabel}: you are not holding this object yet. Pick it up first.`,
      `${aliasLabel}: du har ikke plukket opp denne tingen enda. Plukk den opp først.`
    ));
  }
}

async function sendWithActiveTargetRequirementsIfNeeded(rawText) {
  const source = String(rawText || '').trim();
  if (!source) return;

  const hasActiveAlias = Boolean(String(state.activeObjectTargetAlias || '').trim());
  if (hasActiveAlias && shouldAutoPrefixActiveTarget(source)) {
    const requirement = normalizeUseRequirement(state.activeObjectTargetRequirement);
    if (requirement === 'held') {
      await ensureHeldRequirementSatisfied(state.activeObjectTargetAlias, state.activeObjectTargetDid);
    }
  }

  const outgoing = maybePrefixActiveObjectTarget(source);
  await sendCurrentWorldMessage(outgoing);
}

function primeDidLookupCacheFromWorldMessage(message) {
  const text = String(message || '').trim();
  if (!text) return;

  const bound = text.match(/\bbound\s+(@[A-Za-z0-9_-]+)\s*->\s*(did:ma:[^\s]+)(?:\s*\(object_id=([A-Za-z0-9_-]+)\))?/i);
  if (bound) {
    const alias = String(bound[1] || '').trim();
    const did = String(bound[2] || '').trim();
    const objectId = String(bound[3] || '').trim();
    if (did.startsWith('did:ma:')) {
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
    if (!objectId || !did.startsWith('did:ma:')) {
      continue;
    }
    cacheRoomDidLookup(objectId, did);
  }
}

function applyWorldResponse(result) {
  if (!state.currentHome) {
    return;
  }

  if (result.room) {
    const previousRoom = state.currentHome.room;
    state.currentHome.room = result.room;
    if (result.room_did) state.currentHome.roomDid = result.room_did;
    if (result.room_title) state.currentHome.roomTitle = result.room_title;
    if (typeof result.room_description === 'string') state.currentHome.roomDescription = result.room_description;
    saveLastRoom(state.currentHome.endpointId, result.room);
    updateIdentityLine();
    updateRoomHeading(state.currentHome.roomTitle || '', state.currentHome.roomDescription || '');
    // Clear presence panel on room change and seed from server roster.
    if (result.room !== previousRoom) {
      clearActiveObjectTarget();
      state.roomDidLookupCache.clear();
      state.roomDidLookupInFlight.clear();
      clearRoomPresence();
      if (Array.isArray(result.avatars) && result.avatars.length > 0) {
        for (const avatar of result.avatars) {
          const handle = String(avatar?.handle || '').trim();
          if (handle) trackRoomPresence(handle, String(avatar?.did || ''));
        }
      } else {
        // Fallback: seed with self only (snapshot push will fill the rest).
        trackRoomPresence(state.currentHome.handle || state.aliasName, state.identity?.did || '');
      }
    }
    saveActiveHomeSnapshot();
  } else if (result.room_description !== undefined || result.room_title !== undefined) {
    if (typeof result.room_title === 'string' && result.room_title) {
      state.currentHome.roomTitle = result.room_title;
    }
    if (typeof result.room_description === 'string') {
      state.currentHome.roomDescription = result.room_description;
    }
    updateRoomHeading(state.currentHome.roomTitle || '', state.currentHome.roomDescription || '');
  }

  state.currentHome.lastEventSequence = toSequenceNumber(
    result.latest_event_sequence || state.currentHome.lastEventSequence || 0
  );

  primeDidLookupCacheFromRoomObjectDids(result.room_object_dids);

  if (!result.broadcasted) {
    primeDidLookupCacheFromWorldMessage(result.message);
    appendMessage('world', result.message || '(no response)');
    autoFollowEnterDirective(result.message).catch((err) => {
      appendMessage('system', `Auto-enter failed: ${err instanceof Error ? err.message : String(err)}`);
    });
      if (state.activeObjectTargetAlias) {
        refillCommandInputWithActiveTarget();
      }
  }
}

function isLikelyIrohAddress(address) {
  return /^[a-f0-9]{64}$/i.test(normalizeIrohAddress(address));
}

function normalizeRelayUrl(input) {
  let value = String(input || '').trim();
  // Remove all trailing dots and slashes
  while (value.endsWith('.') || value.endsWith('/')) {
    value = value.slice(0, -1);
  }
  // Ensure it ends with a single /
  return value + '/';
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function withTimeout(promise, ms, message) {
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

async function lookupWorldRelayHint(endpointId) {
  const lookupStart = Date.now();
  logger.log('relay.lookup', `fetching status for endpoint ${endpointId.slice(0, 8)}...`);
  
  try {
    const response = await withTimeout(fetch('http://127.0.0.1:5002/status.json'), 1500, 'status fetch timed out');
    const elapsed = Date.now() - lookupStart;
    
    if (!response.ok) {
      logger.log('relay.lookup', `status fetch returned ${response.status} in ${elapsed}ms`);
      return null;
    }
    
    const status = await response.json();
    const world = status && status.world ? status.world : null;
    
    if (!world || world.endpoint_id !== endpointId) {
      logger.log('relay.lookup', `endpoint mismatch in status (expected ${endpointId.slice(0, 8)}..., got ${world?.endpoint_id?.slice(0, 8)}...) in ${elapsed}ms`);
      return null;
    }
    
    const relayUrls = Array.isArray(world.relay_urls) ? world.relay_urls : [];
    if (relayUrls.length === 0) {
      logger.log('relay.lookup', `no relay urls in status in ${elapsed}ms`);
      return null;
    }
    
    const rawUrl = relayUrls[0];
    const normalizedUrl = normalizeRelayUrl(rawUrl);
    logger.log('relay.lookup', `found relay in ${elapsed}ms: raw="${rawUrl}" normalized="${normalizedUrl}"`);
    
    return normalizedUrl;
  } catch (error) {
    const elapsed = Date.now() - lookupStart;
    logger.log('relay.lookup', `failed after ${elapsed}ms: ${error instanceof Error ? error.message : String(error)}`);
    return null;
  }
}

async function enterWorldWithRetry(endpointId, actorName, room) {
  const maxAttempts = 3;
  let lastError = null;
  logger.log('enter.world', `starting enter sequence for endpoint=${endpointId.slice(0, 8)}... actor=${actorName} room=${room}`);

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const attemptStart = Date.now();
    logger.log(`enter.attempt.${attempt}`, `starting attempt`);
    
    try {
      // Phase 1: Relay discovery and connection
      logger.log(`enter.attempt.${attempt}`, `phase 1/2: relay discovery and connect`);
      const relayHint = await lookupWorldRelayHint(endpointId);
      
      if (relayHint) {
        logger.log(`enter.attempt.${attempt}`, `using relay hint: ${relayHint}`);
      } else {
        logger.log(`enter.attempt.${attempt}`, `no relay hint found, falling back to discovery-only`);
      }

      const connectStart = Date.now();
      await withTimeout(
        relayHint
          ? connect_world_with_relay(endpointId, relayHint)
          : connect_world(endpointId),
        17000,
        'connect phase timed out'
      );
      const connectElapsed = Date.now() - connectStart;
      logger.log(`enter.attempt.${attempt}`, `connected in ${connectElapsed}ms`);

      // Phase 2: World enter request
      logger.log(`enter.attempt.${attempt}`, `phase 2/2: sending enter request`);
      const requestStart = Date.now();
      const response = await withTimeout(
        enter_world(endpointId, state.passphrase, state.encryptedBundle, actorName, room),
        12000,
        'enter request timed out'
      );
      const requestElapsed = Date.now() - requestStart;
      logger.log(`enter.attempt.${attempt}`, `enter request succeeded in ${requestElapsed}ms`);
      
      const result = JSON.parse(response);
      logger.log(`enter.attempt.${attempt}`, `response: ok=${result.ok} room=${result.room} latest_seq=${result.latest_event_sequence || 0} endpoint=${result.endpoint_id?.slice(0, 8)}...`);
      logger.log(`enter.world`, `success after ${Date.now() - attemptStart}ms total on attempt ${attempt}/${maxAttempts}`);
      
      return response;
    } catch (error) {
      lastError = error;
      const message = error instanceof Error ? error.message : String(error);
      const elapsedTotal = Date.now() - attemptStart;
      const isTimeout = message.includes('timed out');
      const isConnectionLost = message.includes('connection lost');
      const isRetryable = isTimeout || isConnectionLost;
      
      logger.log(`enter.attempt.${attempt}`, `failed after ${elapsedTotal}ms: ${message} (retryable=${isRetryable})`);

      if (!isRetryable || attempt === maxAttempts) {
        logger.log('enter.world', `giving up after attempt ${attempt}/${maxAttempts}: ${message}`);
        throw error;
      }

      const backoffMs = 1500 * attempt;
      appendMessage(
        'system',
        `iroh attempt ${attempt}/${maxAttempts} failed (${message}). Retrying...`
      );
      logger.log(`enter.attempt.${attempt}`, `waiting ${backoffMs}ms before attempt ${attempt + 1}`);
      await delay(backoffMs);
    }
  }

  logger.log('enter.world', `failed: all ${maxAttempts} attempts exhausted`);
  throw lastError || new Error('iroh connect failed');
}

async function enterHome(target, preferredRoom = null) {
  const options = (typeof arguments[2] === 'object' && arguments[2] !== null) ? arguments[2] : {};
  const silent = Boolean(options.silent);
  if (!state.identity) {
    throw new Error('Load or create an identity before entering a home.');
  }

  const alias = String(target || '').trim();
  if (!alias) {
    throw new Error('enterHome() requires a target (did:ma:world[#room] or alias).');
  }

  const resolvedInput = resolveAliasInput(alias);
  const resolvedDidRoot = String(resolvedInput).startsWith('did:ma:') ? didRoot(resolvedInput) : '';
  const resolvedDidFragment = String(resolvedInput).includes('#') ? String(resolvedInput).split('#')[1] : '';
  let worldDidForBundle = '';
  let endpointId = '';
  if (String(resolvedInput).startsWith('did:ma:')) {
    endpointId = state.didEndpointMap[didRoot(resolvedInput)] || endpointId;
  } else {
    endpointId = normalizeIrohAddress(resolvedInput);
  }

  if (!endpointId && resolvedDidRoot) {
    const targetDocJson = await fetchDidDocumentJsonByDid(resolvedDidRoot);
    const targetDoc = parseDidDocument(targetDocJson);
    const hintedWorldDid = typeof targetDoc?.ma?.world === 'string'
      ? targetDoc.ma.world
      : '';
    const worldDid = hintedWorldDid ? didRoot(hintedWorldDid) : resolvedDidRoot;
    worldDidForBundle = worldDid;
    const worldDocJson = await fetchDidDocumentJsonByDid(worldDid);
    const worldDoc = parseDidDocument(worldDocJson);
    endpointId = extractWorldEndpointFromDidDoc(worldDoc);
  }

  if (worldDidForBundle && state.passphrase && state.encryptedBundle) {
    try {
      const updated = JSON.parse(set_bundle_world(state.passphrase, state.encryptedBundle, worldDidForBundle));
      state.identity = updated;
      state.encryptedBundle = updated.encrypted_bundle;
      const bundleEl = byId('bundle-text');
      if (bundleEl) {
        bundleEl.value = updated.encrypted_bundle;
      }
      if (isValidAliasName(state.aliasName || '')) {
        saveIdentityRecord(state.aliasName, updated.encrypted_bundle);
      }
    } catch (err) {
      logger.log('enter.home', `warning: could not persist ma.world=${worldDidForBundle}: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  const effectivePreferredRoom = String(preferredRoom || '').trim() || resolvedDidFragment;
  logger.log('enter.home', `alias=${alias} resolved=${resolvedInput} endpoint=${endpointId.slice(0, 8)}...`);
  
  if (!isLikelyIrohAddress(endpointId)) {
    if (resolvedDidRoot) {
      throw new Error(
        `DID ${resolvedDidRoot} did not resolve to a valid iroh endpoint. Ensure its DID document has ma.transports, ma.currentInbox, or ma.presenceHint with /iroh/<endpoint-id>.`
      );
    }
    throw new Error(
      `Alias ${alias} is not a valid endpoint id (expected 64 hex chars, got ${endpointId.length}).`
    );
  }

  if (!silent) {
    appendMessage('system', `Connecting to ${humanizeIdentifier(endpointId)}...`);
  }
  const requestedRoom = effectivePreferredRoom;
  const savedRoom = requestedRoom || loadLastRoom(endpointId);
  let result;
  try {
    if (savedRoom && savedRoom !== 'lobby') {
      try {
        result = JSON.parse(await enterWorldWithRetry(endpointId, state.aliasName, savedRoom));
        if (!result.ok) {
          logger.log('enter.home', `last room '${savedRoom}' denied (${result.message}), falling back to lobby`);
          result = JSON.parse(await enterWorldWithRetry(endpointId, state.aliasName, 'lobby'));
        }
      } catch (_) {
        result = JSON.parse(await enterWorldWithRetry(endpointId, state.aliasName, 'lobby'));
      }
    } else {
      result = JSON.parse(await enterWorldWithRetry(endpointId, state.aliasName, 'lobby'));
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (isClosetBootstrapFailureMessage(message)) {
      const closet = await closetStartSessionForEndpoint(endpointId);
      appendMessage('system', 'Could not verify avatar identity for world enter yet. Starting closet onboarding.');
      appendMessage('system', `Closet session: ${closet.session_id || state.closetSessionId}`);
      renderClosetResponse(closet);
      return;
    }
    throw error;
  }
  logger.log('enter.home', `result ok=${result.ok} room=${result.room} endpoint=${result.endpoint_id?.slice(0, 8)}... latest_seq=${result.latest_event_sequence || 0}`);

  if (!result.ok) {
    if (isClosetRequiredMessage(result.message) || isClosetBootstrapFailureMessage(result.message)) {
      const closet = await closetStartSessionForEndpoint(endpointId);
      appendMessage('system', result.message || 'Entry denied. Closet onboarding is required first.');
      appendMessage('system', `Closet session: ${closet.session_id || state.closetSessionId}`);
      renderClosetResponse(closet);
      return;
    }
    throw new Error(result.message || 'enter failed');
  }

  const activeRoom = result.room || 'lobby';

  state.roomDidLookupCache.clear();
  state.roomDidLookupInFlight.clear();

  state.currentHome = {
    alias: findAliasForAddress(endpointId) || alias,
    endpointId,
    room: activeRoom,
    roomTitle: result.room_title || humanRoomTitle(activeRoom),
    roomDescription: result.room_description || '',
    roomDid: result.room_did || '',
    lastEventSequence: toSequenceNumber(result.latest_event_sequence || 0),
    handle: result.handle || state.aliasName
  };
  state.closetSessionId = '';
  state.closetEndpointId = '';
  state.closetLobbySeq = 0;
  primeDidLookupCacheFromRoomObjectDids(result.room_object_dids);
  saveLastRoom(endpointId, activeRoom);
  saveActiveHomeSnapshot();
  clearRoomPresence();
  if (Array.isArray(result.avatars) && result.avatars.length > 0) {
    for (const avatar of result.avatars) {
      const handle = String(avatar?.handle || '').trim();
      if (handle) trackRoomPresence(handle, String(avatar?.did || ''));
    }
  } else {
    trackRoomPresence(result.handle || state.aliasName, state.identity?.did || '');
  }
  updateIdentityLine();
  updateRoomHeading(state.currentHome.roomTitle, state.currentHome.roomDescription);

  await ensureInboxListener();
  updateIdentityLine();

  startHomeEventPolling();
  await pollCurrentHomeEvents();

  if (!silent) {
    appendMessage('system', `Entered ${humanizeIdentifier(endpointId)}.`);
    appendMessage('system', `Home endpoint: ${humanizeIdentifier(result.endpoint_id)}`);
    appendMessage('system', `Current room: ${activeRoom}`);
    if (result.handle) {
      appendMessage('system', `Assigned handle: ${result.handle}`);
    }
    appendMessage('system', humanizeText(result.message || 'Connected to home.'));
  }
}

function isNotRegisteredInRoomMessage(message) {
  const text = String(message || '').toLowerCase();
  return text.includes('not registered in room');
}

function isClosetRequiredMessage(message) {
  const text = String(message || '').toLowerCase();
  return text.includes('closet') || text.includes('not registered in room');
}

function isClosetBootstrapFailureMessage(message) {
  const text = String(message || '').toLowerCase();
  return (
    text.includes('document signature is invalid')
    || text.includes('failed to decode did document')
    || text.includes('did document')
    || text.includes('verify') && text.includes('message')
    || text.includes('verify') && text.includes('signature')
    || text.includes('sender document')
    || text.includes('unknown did')
  );
}

function isActiveTargetGoneMessage(message) {
  const text = String(message || '').toLowerCase();
  return (
    text.includes('unknown actor or object')
    || text.includes('object alias') && text.includes('stale')
    || text.includes('shortcut') && text.includes('not found')
    || text.includes('object') && text.includes('not found')
  );
}

function reportActiveTargetVanished(alias) {
  const normalizedAlias = String(alias || '').trim() || '@dings';
  appendMessage('system', `${normalizedAlias} vanished in a puff of logic.`);
}

async function restoreActiveObjectTargetAfterReentry(alias, did) {
  const normalizedAlias = String(alias || '').trim();
  const normalizedDid = String(did || '').trim();
  if (!normalizedAlias.startsWith('@') || !normalizedDid.startsWith('did:ma:')) {
    return;
  }

  try {
    await sendWorldCommandQuery(`@${normalizedDid} id`);
    cacheRoomDidLookup(normalizedAlias, normalizedDid);
    setActiveObjectTarget(normalizedAlias, normalizedDid);
    refillCommandInputWithActiveTarget();
  } catch (error) {
    logger.log(
      'reconnect',
      `could not restore active target ${normalizedAlias}: ${error instanceof Error ? error.message : String(error)}`
    );
    dropCachedRoomDidLookup(normalizedAlias);
    clearActiveObjectTarget(normalizedAlias);
    refillCommandInputWithActiveTarget();
    reportActiveTargetVanished(normalizedAlias);
  }
}

async function performTransparentReentry(reason) {
  if (state.transparentReentryPromise) {
    return await state.transparentReentryPromise;
  }

  if (!state.currentHome) {
    throw new Error('Not connected to a world.');
  }

  const home = state.currentHome;
  const endpointId = String(home.endpointId || '').trim();
  const room = String(home.room || '').trim() || 'lobby';
  const activeAlias = String(state.activeObjectTargetAlias || '').trim();
  const activeDid = String(state.activeObjectTargetDid || '').trim();
  const resumeTarget = buildCurrentHomeResumeTarget() || endpointId;

  const work = (async () => {
    logger.log(
      'reconnect',
      `transparent re-entry triggered (${reason || 'unknown reason'}) endpoint=${endpointId.slice(0, 8)}... room=${room}`
    );
    await enterHome(resumeTarget, room, { silent: true });
    await restoreActiveObjectTargetAfterReentry(activeAlias, activeDid);
  })();

  state.transparentReentryPromise = work;
  try {
    await work;
  } finally {
    if (state.transparentReentryPromise === work) {
      state.transparentReentryPromise = null;
    }
  }
}

const tryHandleDidTargetMetaPoll = createDidTargetMetaPollHandler({
  state,
  resolveAliasInput,
  didRoot,
  pollCurrentHomeEvents,
  appendMessage,
  sendWorldCommandQuery,
  parseRoomShowMeta,
  cacheRoomDidLookup
});

async function sendCurrentWorldMessage(text) {
  const attempt = (arguments[1] && typeof arguments[1] === 'object' && Number.isFinite(arguments[1].attempt))
    ? Number(arguments[1].attempt)
    : 0;

  try {
    const trimmedText = text.trim();

    if (!state.identity) {
      appendMessage('system', 'Create or unlock an identity first.');
      return;
    }

    if (!state.currentHome) {
      if (state.closetSessionId && state.closetEndpointId) {
        const response = await closetCommandForCurrentWorld(trimmedText);
        renderClosetResponse(response);
        return;
      }

      const bootstrapMatch = trimmedText.match(/^go\s+(.+)$/i);
      if (bootstrapMatch) {
        const target = String(bootstrapMatch[1] || '').trim();
        const looksLikeDid = target.startsWith('did:ma:');
        const looksLikeAlias = Object.prototype.hasOwnProperty.call(state.aliasBook, target);
        const looksLikeEndpoint = isLikelyIrohAddress(normalizeIrohAddress(target));

        if (looksLikeDid || looksLikeAlias || looksLikeEndpoint) {
          await enterHome(target);
          return;
        }
      }

      appendMessage('system', 'Not connected. Use go did:ma:<world>#<room> or go home (after .set home).');
      return;
    }

    if (state.closetSessionId && state.closetEndpointId
      && state.currentHome.endpointId === state.closetEndpointId) {
      const response = await closetCommandForCurrentWorld(trimmedText);
      renderClosetResponse(response);
      return;
    }

    if (/^use\s+/i.test(trimmedText) || /^unuse\s+/i.test(trimmedText)) {
      parseDot(`.${trimmedText}`);
      return;
    }

    const pickUpMatch = trimmedText.match(/^(?:pick\s+up|pickup)\s+(.+)$/i);
    if (pickUpMatch) {
      const targetToken = String(pickUpMatch[1] || '').trim();
      if (!targetToken) {
        appendMessage('system', 'Usage: pick up <object>');
        return;
      }
      const targetDid = await resolveCommandTargetDidOrToken(targetToken);
      const result = await sendWorldCommandQuery(`@${targetDid} take`);
      appendMessage('system', result || `Picked up ${targetToken}.`);
      return;
    }

    if (trimmedText.startsWith("'")) {
      const payload = trimmedText.substring(1);
      const sendStart = Date.now();
      logger.log('send.chat', `room=${state.currentHome.room} actor=${state.aliasName} msg_len=${payload.length}`);

      const result = JSON.parse(
        await send_world_chat(
          state.currentHome.endpointId,
          state.passphrase,
          state.encryptedBundle,
          state.aliasName,
          state.currentHome.room,
          payload
        )
      );
      const elapsed = Date.now() - sendStart;
      logger.log('send.chat', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

      if (!result.ok) {
        throw new Error(result.message || 'chat failed');
      }

      renderLocalBroadcastMessage(payload);
      await pollCurrentHomeEvents();
      appendAmbientProseAfterSpeech().catch((err) => {
        logger.log('ambient.prose', `failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      return;
    }

    if (trimmedText.startsWith('@@')) {
      const sendStart = Date.now();
      logger.log('send.command', `room=${state.currentHome.room} actor=${state.aliasName} msg_len=${trimmedText.length}`);

      const result = JSON.parse(
        await send_world_message(
          state.currentHome.endpointId,
          state.passphrase,
          state.encryptedBundle,
          state.aliasName,
          state.currentHome.room,
          state.languagePreferences,
          trimmedText
        )
      );
      const elapsed = Date.now() - sendStart;
      logger.log('send.command', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

      if (!result.ok) {
        throw new Error(result.message || 'send failed');
      }

      if (!result.broadcasted) {
        applyWorldResponse(result);
        return;
      }

      await pollCurrentHomeEvents();
      return;
    }

    // Generic actor message-passing syntax: @target with explicit message type.
    // @target 'payload     -> x-ma-chat whisper (everything after ' is payload)
    // @target command args -> generic command message (opaque to client, parsed server-side)
    // @target              -> no-op, output "?"
    if (trimmedText.startsWith('@')) {
      const trimmed = trimmedText;
      const spaceIdx = trimmed.indexOf(' ');
    
      if (spaceIdx === -1) {
        // Just "@target" with nothing after
        appendMessage('system', '?');
        return;
      }

      const target = trimmed.substring(1, spaceIdx);
      const remainder = trimmed.substring(spaceIdx + 1);

      if (remainder.startsWith("'")) {
        const payload = remainder.substring(1); // Everything after the '
        try {
          const targetDid = await resolveCommandTargetDidOrToken(target);
          if (!String(targetDid).startsWith('did:ma:')) {
            throw new Error(`Whisper target must resolve to did:ma, got: ${targetDid}`);
          }
          await sendWhisperToDid(targetDid, payload);
          appendMessage('system', `Chat sent to ${targetDid}.`);
          return;
        } catch (err) {
          appendMessage('system', `Error sending chat to ${target}: ${err.message}`);
          return;
        }
      }

      if (!remainder.trim()) {
        appendMessage('system', '?');
        return;
      }

      if (await tryHandleDidTargetMetaPoll(target, remainder)) {
        return;
      }

      const resolvedTarget = await resolveCommandTargetDidOrToken(target);
      const normalized = `@${resolvedTarget} ${remainder}`;

      const sendStart = Date.now();
      logger.log('send.command', `room=${state.currentHome.room} actor=${state.aliasName} msg_len=${trimmed.length}`);
      const result = JSON.parse(
        await send_world_cmd(
          state.currentHome.endpointId,
          state.passphrase,
          state.encryptedBundle,
          state.aliasName,
          state.currentHome.room,
          state.languagePreferences,
          normalized
        )
      );
      const elapsed = Date.now() - sendStart;
      logger.log('send.command', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

      if (!result.ok) {
        throw new Error(result.message || 'send failed');
      }

      if (!result.broadcasted) {
        applyWorldResponse(result);
        return;
      }

      await pollCurrentHomeEvents();
      return;
    }

    const sendStart = Date.now();
    logger.log('send.command', `room=${state.currentHome.room} actor=${state.aliasName} msg_len=${trimmedText.length}`);

    const result = JSON.parse(
      await send_world_cmd(
        state.currentHome.endpointId,
        state.passphrase,
        state.encryptedBundle,
        state.aliasName,
        state.currentHome.room,
        state.languagePreferences,
        trimmedText
      )
    );
    const elapsed = Date.now() - sendStart;
    logger.log('send.command', `response ok=${result.ok} broadcasted=${result.broadcasted} latest_seq=${result.latest_event_sequence || 0} in ${elapsed}ms`);

    if (!result.ok) {
      throw new Error(result.message || 'send failed');
    }
    
    if (!result.broadcasted) {
      applyWorldResponse(result);
      return;
    }

    if (trimmedText.startsWith("'")) {
      renderLocalBroadcastMessage(trimmedText.substring(1));
    }
    await pollCurrentHomeEvents();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (attempt >= 1 || !isNotRegisteredInRoomMessage(message)) {
      throw error;
    }
    await performTransparentReentry(message);
    return await sendCurrentWorldMessage(text, { attempt: attempt + 1 });
  }
}

async function sendWhisperToDid(targetDidOrAlias, text) {
  if (!state.identity || !state.currentHome) {
    throw new Error('Join a home before sending chat.');
  }

  const key = String(targetDidOrAlias || '').trim();
  if (!key) {
    throw new Error('Usage: @target \'<message>');
  }

  const resolved = resolveAliasInput(key);
  const mappedDid = state.handleDidMap[key] || state.handleDidMap[resolved] || '';
  const targetDid = mappedDid || findDidByEndpoint(resolved) || resolved;
  if (!String(targetDid).startsWith('did:ma:')) {
    throw new Error(`Chat target must be a did:ma: DID, alias, or known handle mapped to a DID. Got: ${targetDid}`);
  }

  const recipientDocumentJson = await fetchDidDocumentJsonByDid(targetDid);
  const result = JSON.parse(
    await send_world_whisper(
      state.currentHome.endpointId,
      state.passphrase,
      state.encryptedBundle,
      state.aliasName,
      recipientDocumentJson,
      text
    )
  );

  if (!result.ok) {
    throw new Error(result.message || 'whisper failed');
  }
}

function parseClosetResponse(rawJson) {
  const parsed = JSON.parse(String(rawJson || '{}'));
  if (!parsed || typeof parsed !== 'object') {
    throw new Error('Invalid closet response.');
  }
  return parsed;
}

function renderClosetResponse(response) {
  const message = String(response?.message || '').trim();
  if (message) {
    appendMessage('system', message);
  }
  const prompt = String(response?.prompt || '').trim();
  if (prompt) {
    appendMessage('system', prompt);
  }
  const events = Array.isArray(response?.lobby_events) ? response.lobby_events : [];
  for (const event of events) {
    const body = String(event?.message || '').trim();
    const sender = String(event?.sender || 'lobby');
    if (body) {
      appendMessage('system', `[lobby/${sender}] ${body}`);
    }
  }
  if (response?.did) {
    appendMessage('system', `Assigned DID: ${response.did}`);
  }
  if (response?.fragment) {
    appendMessage('system', `Assigned fragment: ${response.fragment}`);
  }
}

async function closetStartSessionForEndpoint(endpointId) {
  const normalizedEndpoint = String(endpointId || '').trim();
  if (!normalizedEndpoint) {
    throw new Error('Missing world endpoint for closet session.');
  }
  const response = parseClosetResponse(await closet_start(normalizedEndpoint));
  if (!response.ok) {
    throw new Error(response.message || 'Closet session failed to start.');
  }
  state.closetSessionId = String(response.session_id || '').trim();
  state.closetEndpointId = normalizedEndpoint;
  state.closetLobbySeq = Number(response.latest_lobby_sequence || 0);
  return response;
}

async function closetCommandForCurrentWorld(input) {
  if (!state.closetSessionId) {
    throw new Error('No active closet session.');
  }
  const endpointId = String(
    (state.currentHome && state.currentHome.endpointId) || state.closetEndpointId || ''
  ).trim();
  if (!endpointId) {
    throw new Error('No world endpoint available for closet command.');
  }
  const response = parseClosetResponse(
    await closet_command(endpointId, state.closetSessionId, String(input || ''))
  );
  if (!response.ok) {
    throw new Error(response.message || 'Closet command failed.');
  }
  state.closetLobbySeq = Number(response.latest_lobby_sequence || state.closetLobbySeq || 0);
  if (response.did) {
    state.closetSessionId = '';
    state.closetEndpointId = '';
    state.closetLobbySeq = 0;
  }
  return response;
}

function parseDot(input) {
  const trimmed = String(input || '').trim();
  if (!trimmed.startsWith('.')) {
    return false;
  }

  const rest = trimmed.slice(1).trim();
  if (!rest) {
    appendSystemUi('Usage: .<command> (try .help)', 'Bruk: .<kommando> (prøv .help)');
    return true;
  }

  const [verbRaw, ...args] = rest.split(/\s+/);
  const verb = String(verbRaw || '').toLowerCase();
  const tail = args.join(' ').trim();

  if (verb === 'help') {
    appendSystemUi('Dot commands:', 'Punktkommandoer:');
    appendSystemUi('  .help                      - this message', '  .help                      - denne meldingen');
    appendSystemUi('  .identity                  - show current identity details', '  .identity                  - vis detaljer for aktiv identitet');
    appendSystemUi('  .alias <name> <address>    - save an address alias', '  .alias <name> <address>    - lagre adressealias');
    appendSystemUi('  .set home [did:ma:...#room]- set home target (or current position)', '  .set home [did:ma:...#room]- sett home-mål (eller nåværende posisjon)');
    appendSystemUi('  .set lang <nb|en|se|da>    - set ma-actor UI language for debugging', '  .set lang <nb|en|se|da>    - sett ma-actor UI-språk for feilsøking');
    appendSystemUi('  .unalias <name>            - remove a saved alias', '  .unalias <name>            - fjern et lagret alias');
    appendSystemUi('  .aliases                   - list saved aliases', '  .aliases                   - list lagrede alias');
    appendSystemUi('  .inspect @here|@me|@exit <name>|<object>- inspect room/me/exit/object and discover DID/CIDs', '  .inspect @here|@me|@exit <navn>|<objekt>- inspiser rom/meg/utgang/objekt og finn DID/CID');
    appendSystemUi('  .use <object|did> [as alias] - set local default target', '  .use <objekt|did> [as alias] - sett lokal standardtarget');
    appendSystemUi('  .unuse @alias              - clear local default target', '  .unuse @alias              - fjern lokal standardtarget');
    appendSystemUi('  .edit [@here|@me|@exit <name>|did:ma:<world>#<room>] - open editor', '  .edit [@here|@me|@exit <navn>|did:ma:<world>#<room>] - åpne editor');
    appendSystemUi('  .eval <cid|alias>          - run script from IPFS CID or alias', '  .eval <cid|alias>          - kjør script fra IPFS CID eller alias');
    appendSystemUi('  .refresh                   - fetch latest room state and events now', '  .refresh                   - hent siste romtilstand og hendelser nå');
    appendSystemUi('  .mail [list|pick|reply|delete|clear] - inspect mailbox queue', '  .mail [list|pick|reply|delete|clear] - inspiser mailbox-kø');
    appendSystemUi('  .invite <did|alias> [note] - allow DID and send invite notice', '  .invite <did|alias> [note] - tillat DID og send invitasjonsmelding');
    appendSystemUi('  .smoke [alias]             - run connectivity smoke test', '  .smoke [alias]             - kjør enkel tilkoblingstest');
    appendMessage('system', '  .lang <tag>                - set primary language (e.g. nb)');
    appendMessage('system', '  .language <a:b:c>          - set preference chain (e.g. nb_NO:nn_NO:en_UK)');
    appendSystemUi('  .block <did|alias|handle>  - block sender DID root', '  .block <did|alias|handle>  - blokker avsenders DID-root');
    appendSystemUi('  .unblock <did|alias|handle>- remove sender from block list', '  .unblock <did|alias|handle>- fjern avsender fra blokkeringslisten');
    appendSystemUi('  .blocks                    - list blocked sender DID roots', '  .blocks                    - list blokkerte avsender-DID-rooter');
    appendSystemUi('  .debug [on|off]            - toggle debug logs', '  .debug [on|off]            - slå debuglogger av/på');
    appendSystemUi('Gameplay (bare, no prefix):', 'Gameplay (bart, uten prefiks):');
    appendSystemUi('  go did:ma:<world>#<room>   - connect when currently disconnected', '  go did:ma:<world>#<room>   - koble til når du er frakoblet');
    appendMessage('system', '  pick up <object>           - pick up object before open/list/accept actions');
    appendSystemUi('  go north                   - navigate (server resolves exit)', '  go north                   - naviger (server løser utgang)');
    appendSystemUi('  look                       - describe current room', '  look                       - beskriv nåværende rom');
    appendSystemUi('  attack goblin              - gameplay verb sent to world', '  attack goblin              - gameplay-verb sendt til world');
    appendSystemUi('  @did:ma:<world>#<room> poll - refresh room metadata on demand', '  @did:ma:<world>#<room> poll - oppdater rommetadata ved behov');
    appendMessage('system', "  'Hello world               - shorthand for @me say Hello world");
    appendSystemUi('  @target command args       - send command to actor', '  @target command args       - send kommando til actor');
    appendMessage('system', "  @target 'message           - whisper to actor (E2E)");
    appendSystemUi('  @@command                  - world-admin command', '  @@command                  - world-admin-kommando');
    return true;
  }

  if (verb === 'identity') {
    if (!state.identity) {
      appendSystemUi('No identity loaded. Create or unlock an identity first.', 'Ingen identitet lastet. Opprett eller lås opp en identitet først.');
      return true;
    }
    const { did, ipns } = state.identity;
    appendMessage('system', `DID:             ${humanizeIdentifier(did)}`);
    appendMessage('system', `IPNS key:        ${ipns}`);
    appendMessage('system', uiText(
      `Alias:           ${state.aliasName || '(none)'}`,
      `Alias:           ${state.aliasName || '(ingen)'}`
    ));
    appendMessage('system', uiText(`Lang:            ${state.lang}`, `Lang:            ${state.lang}`));
    appendMessage('system', uiText(`UI language:     ${state.uiLang}`, `UI-språk:        ${state.uiLang}`));
    appendMessage('system', uiText(`Published field: ma:lang = ${state.lang}`, `Publisert felt:  ma:lang = ${state.lang}`));
    appendMessage('system', uiText(`Published field: ma:language = ${state.languagePreferences}`, `Publisert felt:  ma:language = ${state.languagePreferences}`));
    appendMessage('system', uiText(`DID document at: https://ipfs.io/ipns/${ipns}`, `DID-dokument på: https://ipfs.io/ipns/${ipns}`));
    appendMessage('system', uiText(
      `Current world:   ${state.currentHome ? `${humanizeIdentifier(state.currentHome.endpointId)} (${state.currentHome.room})` : '(none)'}`,
      `Nåværende world: ${state.currentHome ? `${humanizeIdentifier(state.currentHome.endpointId)} (${state.currentHome.room})` : '(ingen)'}`
    ));
    return true;
  }

  if (verb === 'aliases') {
    const entries = Object.entries(state.aliasBook);
    if (entries.length === 0) {
      appendMessage('system', 'No aliases saved yet.');
      return true;
    }
    for (const [name, address] of entries) {
      appendMessage('system', `${name} => ${address}`);
    }
    return true;
  }

  if (verb === 'lang') {
    if (args.length !== 1) {
      appendMessage('system', 'Usage: .lang <tag>');
      return true;
    }
    const lang = normalizeLanguageTag(args[0]);
    const language = normalizeLanguagePreferences(byId('actor-language-list')?.value || lang);
    applyLanguageChange(lang, language)
      .then(() => {
        appendMessage('system', `Primary language is now ${state.lang}.`);
      })
      .catch((error) => {
        appendMessage('system', `Language update failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    return true;
  }

  if (verb === 'language') {
    if (args.length !== 1) {
      appendMessage('system', 'Usage: .language <a:b:c>');
      return true;
    }
    const lang = normalizeLanguageTag(byId('actor-language').value);
    const language = normalizeLanguagePreferences(args[0]);
    applyLanguageChange(lang, language)
      .then(() => {
        appendMessage('system', `Language preferences are now ${state.languagePreferences}.`);
      })
      .catch((error) => {
        appendMessage('system', `Language update failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    return true;
  }

  if (verb === 'alias') {
    if (args.length < 2) {
      appendMessage('system', 'Usage: .alias <name> <address>');
      return true;
    }
    const [name, ...addressParts] = args;
    const address = addressParts.join(' ');
    if (!isPrintableAliasLabel(name)) {
      appendMessage('system', 'Alias name must be printable UTF-8 (no spaces/control chars), up to 64 chars.');
      return true;
    }
    state.aliasBook[name] = address;
    saveAliasBook();
    appendMessage('system', `Alias saved: ${name} => ${address}`);
    return true;
  }

  if (verb === 'set') {
    const key = String(args[0] || '').toLowerCase();
    if (key === 'lang') {
      if (args.length !== 2) {
        appendSystemUi('Usage: .set lang <nb|en|se|da>', 'Bruk: .set lang <nb|en|se|da>');
        return true;
      }

      const candidate = normalizeUiLang(args[1]);
      if (!candidate) {
        appendSystemUi('Usage: .set lang <nb|en|se|da>', 'Bruk: .set lang <nb|en|se|da>');
        return true;
      }

      setUiLanguage(candidate);
      if (state.editSession) {
        updateYamlEditorControls();
      }
      appendMessage('system', uiText(
        `UI language is now ${state.uiLang}.`,
        `UI-språk er nå ${state.uiLang}.`
      ));
      return true;
    }

    if (key !== 'home') {
      appendMessage('system', 'Usage: .set home [did:ma:<world>#<room>] | .set lang <nb|en|se|da>');
      return true;
    }

    let target = args.slice(1).join(' ').trim();
    if (!target) {
      target = resolveCurrentPositionTarget();
      if (!target) {
        appendMessage('system', 'Could not resolve current position as did:ma target. Use .set home did:ma:<world>#<room>.');
        return true;
      }
    }

    if (!target.startsWith('did:ma:')) {
      appendMessage('system', 'Usage: .set home [did:ma:<world>#<room>]');
      return true;
    }

    state.aliasBook.home = target;
    saveAliasBook();
    appendMessage('system', `Home set: home => ${target}`);
    return true;
  }

  if (verb === 'unalias') {
    if (args.length !== 1) {
      appendMessage('system', 'Usage: .unalias <name>');
      return true;
    }
    const [name] = args;
    if (!Object.prototype.hasOwnProperty.call(state.aliasBook, name)) {
      appendMessage('system', `Alias not found: ${name}`);
      return true;
    }
    delete state.aliasBook[name];
    saveAliasBook();
    appendMessage('system', `Alias removed: ${name}`);
    return true;
  }

  if (verb === 'debug') {
    if (args.length === 0) {
      setDebugMode(!state.debug);
    } else {
      const mode = String(args[0] || '').trim().toLowerCase();
      if (mode === 'on' || mode === '1' || mode === 'true') {
        setDebugMode(true);
      } else if (mode === 'off' || mode === '0' || mode === 'false') {
        setDebugMode(false);
      } else {
        appendMessage('system', 'Usage: .debug [on|off]');
        return true;
      }
    }
    return true;
  }

  if (verb === 'blocks') {
    const blocked = Array.from(state.blockedDidRoots || []).sort();
    if (!blocked.length) {
      appendMessage('system', 'No blocked senders.');
      return true;
    }
    appendMessage('system', `Blocked senders (${blocked.length}):`);
    for (const did of blocked) {
      appendMessage('system', `  ${did}`);
    }
    return true;
  }

  if (verb === 'block') {
    if (args.length !== 1) {
      appendMessage('system', 'Usage: .block <did|alias|handle>');
      return true;
    }
    try {
      const root = resolveTargetDidRoot(args[0]);
      if (state.identity && didRoot(state.identity.did) === root) {
        appendMessage('system', 'Refusing to block your own DID root.');
        return true;
      }
      const before = state.blockedDidRoots.size;
      state.blockedDidRoots.add(root);
      if (state.blockedDidRoots.size !== before) {
        saveBlockedDidRoots();
      }
      appendMessage('system', `Blocked sender: ${root}`);
    } catch (error) {
      appendMessage('system', error instanceof Error ? error.message : String(error));
    }
    return true;
  }

  if (verb === 'unblock') {
    if (args.length !== 1) {
      appendMessage('system', 'Usage: .unblock <did|alias|handle>');
      return true;
    }
    try {
      const root = resolveTargetDidRoot(args[0]);
      const removed = state.blockedDidRoots.delete(root);
      if (removed) {
        saveBlockedDidRoots();
        appendMessage('system', `Unblocked sender: ${root}`);
      } else {
        appendMessage('system', `Sender not blocked: ${root}`);
      }
    } catch (error) {
      appendMessage('system', error instanceof Error ? error.message : String(error));
    }
    return true;
  }

  if (verb === 'edit') {
    onDotEdit(tail);
    return true;
  }

  if (verb === 'eval') {
    onDotEval(tail);
    return true;
  }

  if (verb === 'inspect') {
    onDotInspect(tail);
    return true;
  }

  if (verb === 'use') {
    const requirement = 'none';
    const useTail = String(tail || '').trim();
    const didMatch = useTail.match(/^(\S+)(?:\s+as\s+(@?[A-Za-z0-9_-]+))?$/i);
    if (!didMatch) {
      appendMessage('system', uiText('Usage: .use <object|did:ma:...#fragment> [as alias]', 'Bruk: .use <objekt|did:ma:...#fragment> [as alias]'));
      return true;
    }

    const rawTarget = String(didMatch[1] || '').trim();
    const requestedAliasRaw = String(didMatch[2] || '').trim();
    const requestedAlias = requestedAliasRaw
      ? (requestedAliasRaw.startsWith('@') ? requestedAliasRaw : `@${requestedAliasRaw}`)
      : '';
    Promise.resolve()
      .then(async () => {
        const objectDid = rawTarget.startsWith('did:ma:')
          ? rawTarget
          : await lookupDidInCurrentRoom(rawTarget);
        const fragment = objectDid.includes('#') ? objectDid.split('#')[1] : '';
        const autoAlias = fragment ? `@${fragment.replace(/[^A-Za-z0-9_-]/g, '').toLowerCase()}` : '@obj';
        const alias = requestedAlias || autoAlias;
        if (!/^@[A-Za-z0-9_-]+$/.test(alias)) {
          appendMessage('system', uiText('Usage: .use <object|did:ma:...#fragment> [as alias]', 'Bruk: .use <objekt|did:ma:...#fragment> [as alias]'));
          return;
        }
        await sendWorldCommandQuery(`@${objectDid} id`);
        cacheRoomDidLookup(rawTarget, objectDid);
        cacheRoomDidLookup(alias, objectDid);
        setActiveObjectTarget(alias, objectDid, requirement);
        appendMessage('system', `using ${alias} -> ${objectDid}`);
        refillCommandInputWithActiveTarget();
      })
      .catch((error) => {
        appendMessage('system', uiText(
          `Use failed: ${error instanceof Error ? error.message : String(error)}`,
          `Use feilet: ${error instanceof Error ? error.message : String(error)}`
        ));
      });
    return true;
  }

  if (verb === 'unuse') {
    const alias = String(args[0] || '').trim();
    if (!alias || !alias.startsWith('@')) {
      appendMessage('system', uiText('Usage: .unuse @alias', 'Bruk: .unuse @alias'));
      return true;
    }
    dropCachedRoomDidLookup(alias);
    clearActiveObjectTarget(alias);
    appendMessage('system', uiText(`stopped using ${alias}`, `sluttet å bruke ${alias}`));
    refillCommandInputWithActiveTarget();
    return true;
  }

  if (verb === 'refresh') {
    if (!state.currentHome) {
      appendSystemUi('Not connected to a world.', 'Ikke koblet til en world.');
      return true;
    }
    Promise.resolve()
      .then(() => pollDirectInbox())
      .then(() => pollCurrentHomeEvents())
      .then(() => appendSystemUi('Refreshed room state.', 'Oppdatert romtilstand.'))
      .catch((err) => {
        appendMessage('system', uiText(
          `Refresh failed: ${err instanceof Error ? err.message : String(err)}`,
          `Oppdatering feilet: ${err instanceof Error ? err.message : String(err)}`
        ));
      });
    return true;
  }

  if (verb === 'mail' || verb === 'mailbox') {
    const sub = String(args[0] || 'list').toLowerCase();
    const list = Array.isArray(state.mailbox) ? state.mailbox : [];

    if (sub === 'list') {
      if (!list.length) {
        appendSystemUi('Mailbox is empty.', 'Mailbox er tom.');
        return true;
      }
      appendMessage('system', `Mailbox (${list.length}):`);
      for (const entry of list) {
        const preview = String(entry.content_text || '').replace(/\s+/g, ' ').slice(0, 80) || '(binary)';
        appendMessage(
          'system',
          `  #${entry.id} from=${humanizeIdentifier(entry.from_did || '(unknown)')} type=${entry.content_type || '(unknown)'} text=${preview}`
        );
      }
      appendSystemUi('Use .mail pick <id>, .mail reply <id> <text>, or .mail delete <id>.', 'Bruk .mail pick <id>, .mail reply <id> <tekst>, eller .mail delete <id>.');
      return true;
    }

    if (sub === 'pick' || sub === 'show') {
      const idRaw = String(args[1] || '').trim();
      const id = Number(idRaw);
      if (!Number.isFinite(id) || id <= 0) {
        appendMessage('system', 'Usage: .mail pick <id>');
        return true;
      }
      const entry = list.find((item) => Number(item.id) === id);
      if (!entry) {
        appendMessage('system', `Mailbox entry not found: ${id}`);
        return true;
      }
      appendMessage('system', `.mail pick ${id}`);
      appendMessage('system', `  from: ${humanizeIdentifier(entry.from_did || '(unknown)')}`);
      appendMessage('system', `  endpoint: ${humanizeIdentifier(entry.from_endpoint || '(unknown)')}`);
      appendMessage('system', `  type: ${entry.content_type || '(unknown)'}`);
      appendMessage('system', `  text: ${entry.content_text || '(binary)'}`);
      appendMessage('system', `  cbor: ${entry.message_cbor_b64 || '(missing)'}`);
      return true;
    }

    if (sub === 'delete' || sub === 'del' || sub === 'rm') {
      const idRaw = String(args[1] || '').trim();
      const id = Number(idRaw);
      if (!Number.isFinite(id) || id <= 0) {
        appendMessage('system', 'Usage: .mail delete <id>');
        return true;
      }
      const before = list.length;
      state.mailbox = list.filter((item) => Number(item.id) !== id);
      if (state.mailbox.length === before) {
        appendMessage('system', `Mailbox entry not found: ${id}`);
        return true;
      }
      appendMessage('system', `Deleted mailbox entry #${id}.`);
      return true;
    }

    if (sub === 'reply') {
      const idRaw = String(args[1] || '').trim();
      const id = Number(idRaw);
      const replyText = args.slice(2).join(' ').trim();
      if (!Number.isFinite(id) || id <= 0 || !replyText) {
        appendMessage('system', 'Usage: .mail reply <id> <text>');
        return true;
      }
      const entry = list.find((item) => Number(item.id) === id);
      if (!entry) {
        appendMessage('system', `Mailbox entry not found: ${id}`);
        return true;
      }
      const targetDid = String(entry.from_did || '').trim();
      if (!targetDid.startsWith('did:ma:')) {
        appendMessage('system', `Mailbox entry #${id} has no valid sender DID.`);
        return true;
      }
      sendWhisperToDid(targetDid, replyText)
        .then(() => {
          appendMessage('system', `Reply sent to ${humanizeIdentifier(targetDid)} from mailbox #${id}.`);
        })
        .catch((error) => {
          appendMessage('system', `Reply failed: ${error instanceof Error ? error.message : String(error)}`);
        });
      return true;
    }

    if (sub === 'clear') {
      const cleared = list.length;
      state.mailbox = [];
      appendMessage('system', `Mailbox cleared (${cleared} entries).`);
      return true;
    }

    appendMessage('system', 'Usage: .mail [list|pick <id>|reply <id> <text>|delete <id>|clear]');
    return true;
  }

  if (verb === 'invite') {
    if (args.length < 1) {
      appendMessage('system', 'Usage: .invite <did|alias|handle> [note]');
      return true;
    }
    let targetRoot = '';
    try {
      targetRoot = resolveTargetDidRoot(args[0]);
    } catch (error) {
      appendMessage('system', error instanceof Error ? error.message : String(error));
      return true;
    }
    const note = args.slice(1).join(' ').trim();
    const inviteText = note || 'Your knock request was accepted. You may enter now.';
    const command = `@world invite ${targetRoot} ${inviteText}`;
    sendWorldCommandQuery(command)
      .then((message) => {
        appendMessage('system', message || `Invited ${targetRoot}.`);
        return sendWhisperToDid(targetRoot, `invite accepted: ${inviteText}`);
      })
      .then(() => {
        appendMessage('system', `Invite notice sent to ${humanizeIdentifier(targetRoot)}.`);
      })
      .catch((error) => {
        appendMessage('system', `Invite failed: ${error instanceof Error ? error.message : String(error)}`);
      });
    return true;
  }

  if (verb === 'smoke') {
    if (args.length > 1) {
      appendMessage('system', 'Usage: .smoke [alias]');
      return true;
    }
    runSmokeTest(args[0]).catch((err) => {
      appendMessage('system', `Smoke failed: ${err instanceof Error ? err.message : String(err)}`);
    });
    return true;
  }

  appendMessage('system', uiText(
    `Unknown command: .${verb}. Try .help.`,
    `Ukjent kommando: .${verb}. Prøv .help.`
  ));
  return true;
}

function onCommandSubmit(event) {
  event.preventDefault();
  const inputEl = byId('command-input');
  const text = inputEl.value.trim();
  if (!text) return;

  // Readline-like history: keep unique latest entry and reset cursor.
  state.commandHistory.push(text);
  state.historyIndex = -1;
  state.historyDraft = '';

  if (text.startsWith('.')) {
    parseDot(text);
  } else {
    sendWithActiveTargetRequirementsIfNeeded(text).catch((err) => {
      const message = err instanceof Error ? err.message : String(err);
      appendMessage('system', `Send failed: ${message}`);
      if (state.activeObjectTargetAlias && isActiveTargetGoneMessage(message)) {
        const alias = String(state.activeObjectTargetAlias || '').trim();
        dropCachedRoomDidLookup(alias);
        clearActiveObjectTarget(alias);
        refillCommandInputWithActiveTarget();
        reportActiveTargetVanished(alias);
      }
    });
  }

  refillCommandInputWithActiveTarget();
}

function onCommandKeyDown(event) {
  const inputEl = byId('command-input');

  if (event.key === 'ArrowUp') {
    if (state.commandHistory.length === 0) {
      return;
    }
    event.preventDefault();
    if (state.historyIndex === -1) {
      state.historyDraft = inputEl.value;
      state.historyIndex = state.commandHistory.length - 1;
    } else if (state.historyIndex > 0) {
      state.historyIndex -= 1;
    }
    inputEl.value = state.commandHistory[state.historyIndex];
    inputEl.setSelectionRange(inputEl.value.length, inputEl.value.length);
    return;
  }

  if (event.key === 'ArrowDown') {
    if (state.commandHistory.length === 0 || state.historyIndex === -1) {
      return;
    }
    event.preventDefault();
    if (state.historyIndex < state.commandHistory.length - 1) {
      state.historyIndex += 1;
      inputEl.value = state.commandHistory[state.historyIndex];
    } else {
      state.historyIndex = -1;
      inputEl.value = state.historyDraft;
    }
    inputEl.setSelectionRange(inputEl.value.length, inputEl.value.length);
  }
}

function restoreSavedValues() {
  scrubStoredRecoveryPhrases();

  const savedApi = localStorage.getItem(API_KEY) || localStorage.getItem(LEGACY_API_KEY);
  const savedAlias = resolveInitialAlias();

  if (savedApi) byId('kubo-api').value = savedApi;

  if (savedAlias) {
    byId('alias-name').value = savedAlias;
    loadAliasDraft(savedAlias);
  } else {
    byId('bundle-text').value = '';
    setLanguageSelection(DEFAULT_LANG, DEFAULT_LANGUAGE_PREFERENCES);
    onNewPhrase();
  }

  state.aliasBook = loadAliasBook();
  state.debug = readStoredDebugFlag();
  setCurrentPublishInfo();
}

function shouldAutoCheckKubo() {
  return isLocalhostLikeHost(window.location.hostname);
}

async function main() {
  await init();
  await updateAppVersionFooter();
  installBridgeMonitor();
  applyProperName();
  restoreSavedValues();
  hideLockOverlay();

  if (shouldAutoCheckKubo()) {
    checkKubo().catch(() => {});
  } else {
    setKuboStatus('not checked (remote origin)', 'idle');
    setSetupStatus('Local Kubo API is optional for create/unlock, but required for IPNS publish and edit publish flows.');
  }

  byId('btn-kubo-check').addEventListener('click', () => {
    checkKubo().catch(() => {});
  });
  byId('btn-create').addEventListener('click', onCreateIdentity);
  byId('btn-unlock').addEventListener('click', onUnlockIdentity);
  byId('btn-new-phrase').addEventListener('click', onNewPhrase);
  byId('btn-export').addEventListener('click', exportBundle);
  byId('btn-lock').addEventListener('click', lockSession);
  byId('lock-overlay').addEventListener('click', hideLockOverlay);
  byId('lock-overlay').addEventListener('keydown', onLockOverlayKeydown);
  byId('actor-language').addEventListener('change', onLanguageChange);
  const languageListInput = byId('actor-language-list');
  if (languageListInput) {
    languageListInput.addEventListener('change', onLanguageChange);
  }
  let aliasDraftTimer = null;
  byId('alias-name').addEventListener('input', (event) => {
    if (aliasDraftTimer) {
      clearTimeout(aliasDraftTimer);
    }
    const value = event.target.value;
    aliasDraftTimer = setTimeout(() => {
      loadAliasDraft(value, { persistActive: false });
    }, 120);
  });
  byId('alias-name').addEventListener('change', (event) => {
    loadAliasDraft(event.target.value, { persistActive: true });
  });
  byId('command-form').addEventListener('submit', onCommandSubmit);
  byId('command-input').addEventListener('keydown', onCommandKeyDown);
  byId('yaml-editor-cancel').addEventListener('click', closeYamlEditorModal);
  byId('yaml-editor-reload').addEventListener('click', () => {
    if (!state.editSession) {
      setYamlEditorStatus('No loaded source to reload.', 'error');
      return;
    }
    if (state.editSession.mode === 'script') {
      const textEl = byId('yaml-editor-text');
      const currentText = String(textEl?.value || '');
      const storedText = String(localStorage.getItem(LOCAL_EDIT_SCRIPT_KEY) || '');
      if (currentText !== storedText) {
        const shouldDiscard = window.confirm('Discard unsaved local script changes and reload from local storage?');
        if (!shouldDiscard) {
          setYamlEditorStatus('Reload canceled. Unsaved text kept.', 'working');
          return;
        }
      }
      loadLocalScriptEditor().catch((err) => {
        appendMessage('system', `Reload failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      return;
    }
    if (state.editSession.mode === 'avatar') {
      loadAvatarEditor().catch((err) => {
        appendMessage('system', `Reload failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      return;
    }
    if (state.editSession.mode === 'exit') {
      const query = String(state.editSession.exitId || '').trim();
      loadExitEditorByQuery(query).catch((err) => {
        appendMessage('system', `Reload failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      return;
    }
    loadYamlEditorForTarget(state.editSession.target, false).catch((err) => {
      appendMessage('system', `Reload failed: ${err instanceof Error ? err.message : String(err)}`);
    });
  });
  byId('yaml-editor-save').addEventListener('click', () => {
    saveYamlEditorChanges();
  });
  byId('yaml-editor-close-eval').addEventListener('click', () => {
    closeAndEvalEditorScript().catch((err) => {
      appendMessage('system', `Close and Eval failed: ${err instanceof Error ? err.message : String(err)}`);
    });
  });
  byId('yaml-editor-modal').addEventListener('click', (event) => {
    if (event.target === byId('yaml-editor-modal')) {
      closeYamlEditorModal();
    }
  });
  byId('yaml-editor-modal').addEventListener('keydown', onYamlEditorModalKeyDown);

  byId('passphrase').addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      const hasBundle = byId('bundle-text').value.trim().length > 0;
      if (hasBundle) {
        onUnlockIdentity();
      } else {
        onCreateIdentity();
      }
    }
  });

  showSetup();
}

main().catch((error) => {
  setSetupStatus(`Fatal error: ${error instanceof Error ? error.message : String(error)}`);
});
