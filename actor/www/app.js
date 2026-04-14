import init, {
  create_identity,
  unlock_identity,
  ensure_bundle_iroh_secret,
  set_bundle_language,
  set_bundle_transports,
  set_bundle_updated_for_send,
  generate_bip39_phrase,
  normalize_bip39_phrase,
  connect_world,
  connect_world_with_relay,
  ping_world,
  poll_world_events,
  send_world_chat,
  send_world_chat_with_ttl,
  send_world_whisper,
  send_world_whisper_with_ttl,
  send_direct_message,
  send_direct_message_with_ttl,
  send_world_cmd,
  send_world_cmd_with_ttl,
  publish_did_document_via_world_ipfs,
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
  disconnect_world
} from './pkg/ma_actor.js';
import { createInboundDispatcher, createInboxTransport } from './inbox.js';
import { createAliasFlow, isPrintableAliasLabel, isValidAliasName } from './alias.js';
import { createDialogWriter } from './dialog-writer.js';
import { createIdentityFlow, createIdentityLineFlow, createIdentityStore } from './identity.js';
import { createEditorUi } from './editor.js';
import { createUiFlow } from './ui.js';
import { createDidTargetMetaPollHandler } from './meta-poll.js';
import {
  createRoomInspectFlow,
  createRoomPresencePayloadFlow,
  createRoomPresenceFlow,
  createRoomStorage,
  humanRoomTitle,
  parseExitYamlSummary,
  sanitizeRoomYamlForEdit,
} from './room.js';
import {
  normalizeLanguageOrder as normalizeLanguageOrderValue,
  normalizeUiLang,
  roomLanguageKey,
  uiLangFromLanguage,
} from './language.js';
import {
  createDidDocFlow,
  createDidRoot,
  createDidRuntimeHelpers,
  didWithFragment,
  isMaDid,
  isMaDidTarget,
  isUnconfiguredDidTarget,
  parseDidDocument as parseDidDocumentUtil,
  resolveEndpointWithTypePolicy,
} from './did.js';
import { createDotCommands } from './dot-commands.js';
import { createWhisperFlow } from './whisper-flow.js';
import {
  createWorldDispatchFlow,
  createWorldFlow,
  createWorldResponseFlow,
  createWorldTitleFlow,
  extractWorldEndpointFromDidDoc,
  isLikelyIrohAddress,
  normalizeIrohAddress,
  parseEnterDirective,
} from './world.js';
import { byId, isLocalhostLikeHost } from './dom-env-utils.js';
import { resolveAppVersionLabel, semverCore } from './version-utils.js';
import {
  extractDidFromLookupResponse,
  extractRoomCidFromShowResponse,
  parseAvatarDescriptionFromShowMessage,
  parseAvatarShowMeta,
  parseDescriptionFromEditorText,
  parseKeyValuePairs,
  parseRoomShowMeta,
} from './show-meta-parsers.js';
import {
  delay,
  normalizeRelayUrl,
  toSequenceBigInt,
  toSequenceNumber,
  withTimeout,
} from './runtime-utils.js';
import {
  asIpfsGatewayPath,
  fetchGatewayTextByPath as fetchGatewayTextByPathRaw,
  normalizeIpfsGatewayBase,
} from './gateway-client.js';

const STORAGE_PREFIX = 'ma.identity.v3';
const PROPER_NAME = '間';
const BRAND_SUBTITLE_STATIC = 'A text-first world for literate play';
const API_KEY = `${STORAGE_PREFIX}.gatewayApi`;
const ALIAS_BOOK_KEY = `${STORAGE_PREFIX}.aliasBook`;
const LAST_ALIAS_KEY = `${STORAGE_PREFIX}.lastAlias`;
const TAB_ALIAS_KEY = `${STORAGE_PREFIX}.tabAlias`;
const DEBUG_KEY = `${STORAGE_PREFIX}.debug`;
const LOG_ENABLED_KEY = `${STORAGE_PREFIX}.logEnabled`;
const LOG_LEVEL_KEY = `${STORAGE_PREFIX}.logLevel`;
const DIALOG_ID_STYLE_KEY = `${STORAGE_PREFIX}.dialogIdStyle`;
const ALIAS_REWRITE_ENABLED_KEY = `${STORAGE_PREFIX}.aliasRewriteEnabled`;
const ALIAS_RENDER_ENABLED_KEY = `${STORAGE_PREFIX}.aliasRenderEnabled`;
const MSG_CHAT_TTL_KEY = `${STORAGE_PREFIX}.actor.msg.chat.ttl`;
const MSG_CMD_TTL_KEY = `${STORAGE_PREFIX}.actor.msg.cmd.ttl`;
const MSG_WHISPER_TTL_KEY = `${STORAGE_PREFIX}.actor.msg.whisper.ttl`;
const LEGACY_BUNDLE_KEY = 'ma.identity.v2.bundle';
const LAST_ROOM_KEY_PREFIX = `${STORAGE_PREFIX}.lastRoom`;
const LAST_ACTIVE_HOME_KEY_PREFIX = `${STORAGE_PREFIX}.lastActiveHome`;
const LAST_PUBLISHED_IPNS_KEY = `${STORAGE_PREFIX}.lastPublishedIpns`;
const LAST_PUBLISHED_CID_KEY = `${STORAGE_PREFIX}.lastPublishedCid`;
const IPNS_PRIVATE_KEY_B64_KEY = `${STORAGE_PREFIX}.ipnsPrivateKeyB64`;
const LEGACY_ALIAS_KEY = 'ma.identity.v2.alias';
const DEFAULT_UI_LANG = 'en';
const DEFAULT_LANGUAGE_ORDER = 'nb_NO:en_UK';
const DID_PUBLISH_PENDING_TTL_MS = 5 * 60 * 1000;
const KNOWN_IPFS_HELLO_WORLD_CID = 'bafkreidfdrlkeq4m4xnxuyx6iae76fdm4wgl5d4xzsb77ixhyqwumhz244';
const IPFS_GATEWAY_FALLBACKS = [
  'http://localhost:8080',
  'https://ipfs.io',
  'https://dweb.link',
  'https://w3s.link',
];
const LOCAL_EDIT_SCRIPT_KEY = `${STORAGE_PREFIX}.localEditScript`;
const LEGACY_LOCAL_EDIT_SCRIPT_CID_KEY = `${STORAGE_PREFIX}.localEditScriptCid`;

const ROOM_POLL_INTERVAL_MS = 5_000;
const AVATAR_PING_INTERVAL_MS = 5_000;
const MIN_WORLD_PING_INTERVAL_MS = 3_000;
const MAX_WORLD_PING_INTERVAL_MS = 30_000;
const PING_FIB_START = 3000;
const PING_WORLD_GONE_THRESHOLD = 60_000;
const DID_DOC_CACHE_TTL_MS = 60_000;
const DEFAULT_CHAT_TTL_SECONDS = 60;
const DEFAULT_CMD_TTL_SECONDS = 60;
const DEFAULT_WHISPER_TTL_SECONDS = 3660;

const didRoot = createDidRoot(alias_did_root);

function getApiBase() {
  return normalizeIpfsGatewayBase(byId('gateway-api').value);
}

async function fetchGatewayTextByPath(contentPath, options = {}) {
  const localOnly = Boolean(options && options.localOnly);
  const timeoutMs = Number(options && options.timeoutMs);
  return await fetchGatewayTextByPathRaw(contentPath, {
    getApiBase,
    fallbackBases: localOnly ? [] : IPFS_GATEWAY_FALLBACKS,
    timeoutMs: Number.isFinite(timeoutMs) && timeoutMs > 0 ? timeoutMs : undefined,
  });
}


async function updateAppVersionFooter() {
  const versionEl = byId('app-version');
  if (!versionEl) return;
  const localVersion = await resolveAppVersionLabel();
  versionEl.textContent = `Version: ${localVersion}`;
}

const state = {
  identity: null,
  encryptedBundle: '',
  aliasName: '',
  languageOrder: DEFAULT_LANGUAGE_ORDER,
  uiLang: DEFAULT_UI_LANG,
  debug: false,
  logEnabled: true,
  logLevel: 'info',
  dialogIdStyle: 'alias',
  aliasRewriteEnabled: true,
  aliasRenderEnabled: true,
  messageTtl: {
    chat: DEFAULT_CHAT_TTL_SECONDS,
    cmd: DEFAULT_CMD_TTL_SECONDS,
    whisper: DEFAULT_WHISPER_TTL_SECONDS,
  },
  temporaryMessageTtlOverride: null,
  batch: {
    collecting: false,
    running: false,
    timeoutSeconds: 10,
    retryCount: 0,
    commands: [],
  },
  aliasBook: {},
  currentHome: null,
  roomPollTimer: null,
  avatarPingTimer: null,
  roomPollInFlight: false,
  inboxPollInFlight: false,
  consecutivePollFailures: 0,
  pollErrorShown: false,
  passphrase: '',
  handleDidMap: {},
  roomDidLookupCache: new Map(),
  roomDidLookupInFlight: new Map(),
  didEndpointMap: {},
  didDocCache: new Map(),
  inboxEndpointId: '',
  mailbox: [],
  mailboxSeq: 0,
  commandHistory: [],
  commandQueue: Promise.resolve(),
  historyIndex: -1,
  historyDraft: '',
  roomPresence: new Map(),
  activeObjectTargetAlias: '',
  activeObjectTargetDid: '',
  activeObjectTargetRequirement: 'none',
  didPublishPromise: null,
  didPublishError: '',
  didPublishPendingCache: new Map(),
  worldPingIntervalMs: null,
  reentryInProgress: null,
  reentryFibPrev: 3000,
  reentryFibCurr: 3000,
  pingFibPrev: PING_FIB_START,
  pingFibCurr: PING_FIB_START,
  pingRetryTimer: null,
  pingInFlight: false,
  editSession: null,
  editBusy: false,
  lockOverlayAnimationId: 0,
  lockOverlayStarDrift: 0,
  lockOverlayShownAtMs: 0,
  matrixLog: [],
};

let worldDispatchFlow = null;
let didDocFlow = null;

const aliasFlowBridge = {
  saveAliasBook: (book) => book,
  loadAliasBook: () => ({}),
  setActiveAlias: () => {},
  resolveInitialAlias: () => '',
  loadAliasDraft: () => '',
  roomDidLookupCacheKey: (value) => String(value || '').trim(),
  getCachedRoomDidLookup: () => '',
  cacheRoomDidLookup: () => {},
  dropCachedRoomDidLookup: () => {},
  normalizeEndpointId: (value) => String(value || '').trim(),
  findDidByEndpoint: () => '',
  findAliasForAddress: () => '',
  resolveAliasInput: (value) => String(value || '').trim(),
  humanizeIdentifier: (value) => String(value || ''),
  humanizeText: (value) => String(value || ''),
};

function saveAliasBook(...args) { return aliasFlowBridge.saveAliasBook(...args); }
function loadAliasBook(...args) { return aliasFlowBridge.loadAliasBook(...args); }
function setActiveAlias(...args) { return aliasFlowBridge.setActiveAlias(...args); }
function resolveInitialAlias(...args) { return aliasFlowBridge.resolveInitialAlias(...args); }
function loadAliasDraft(...args) { return aliasFlowBridge.loadAliasDraft(...args); }
function roomDidLookupCacheKey(...args) { return aliasFlowBridge.roomDidLookupCacheKey(...args); }
function getCachedRoomDidLookup(...args) { return aliasFlowBridge.getCachedRoomDidLookup(...args); }
function cacheRoomDidLookup(...args) { return aliasFlowBridge.cacheRoomDidLookup(...args); }
function dropCachedRoomDidLookup(...args) { return aliasFlowBridge.dropCachedRoomDidLookup(...args); }
function normalizeEndpointId(...args) { return aliasFlowBridge.normalizeEndpointId(...args); }
function findDidByEndpoint(...args) { return aliasFlowBridge.findDidByEndpoint(...args); }
function findAliasForAddress(...args) { return aliasFlowBridge.findAliasForAddress(...args); }
function resolveAliasInput(...args) { return aliasFlowBridge.resolveAliasInput(...args); }
function humanizeIdentifier(...args) { return aliasFlowBridge.humanizeIdentifier(...args); }
function humanizeText(...args) { return aliasFlowBridge.humanizeText(...args); }

let updateIdentityLineImpl = () => {};
function updateIdentityLine(...args) {
  return updateIdentityLineImpl(...args);
}

const ROOM_DID_CACHE_TTL_MS = 30000;

const { saveLastRoom, loadLastRoom } = createRoomStorage({
  state,
  lastRoomKeyPrefix: LAST_ROOM_KEY_PREFIX,
});

function activeHomeKey(identityDid) {
  const did = String(identityDid || '').trim();
  if (!isMaDid(did)) return '';
  return `${LAST_ACTIVE_HOME_KEY_PREFIX}.${did}`;
}

function buildCurrentHomeResumeTarget() {
  if (!state.currentHome) {
    return '';
  }

  const roomDid = String(state.currentHome.roomDid || '').trim();
  if (isMaDid(roomDid) && !isUnconfiguredDidTarget(roomDid)) {
    return roomDid;
  }

  const worldDid = currentWorldDid();
  if (worldDid) {
    const room = String(state.currentHome.room || 'lobby').trim() || 'lobby';
    return didWithFragment(worldDid, room);
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

function persistActiveHomeSnapshot(identityDid, snapshot) {
  const key = activeHomeKey(identityDid);
  if (!key || !snapshot || typeof snapshot !== 'object') {
    return;
  }
  try {
    localStorage.setItem(key, JSON.stringify(snapshot));
  } catch (_) {
    // Ignore storage write failures.
  }
}

function clearActiveHomeSnapshot(identityDid) {
  const key = activeHomeKey(identityDid);
  if (!key) {
    return;
  }
  try {
    localStorage.removeItem(key);
  } catch (_) {
    // Ignore storage write failures.
  }
}

function syncSpecialAliasesFromCurrentHome() {
  const SPECIAL_KEYS = ['@world', '@here', '@me', '@avatar'];
  const next = {};

  const identityDid = String(state.identity?.did || '').trim();
  if (isMaDidTarget(identityDid)) {
    next['@me'] = identityDid;
  }

  const roomDid = String(state.currentHome?.roomDid || '').trim();
  const room = String(state.currentHome?.room || '').trim();
  const worldDid = String(state.currentHome?.worldDid || '').trim();

  if (isMaDidTarget(worldDid) && worldDid.includes('#') && !isUnconfiguredDidTarget(worldDid)) {
    next['@world'] = worldDid;
  }

  const handle = String(state.currentHome?.handle || '').trim().replace(/^@+/, '');
  if (isMaDidTarget(worldDid) && handle && !/\s/u.test(handle)) {
    next['@avatar'] = didWithFragment(worldDid, handle);
  }

  let hereDid = '';
  if (isMaDidTarget(roomDid) && !isUnconfiguredDidTarget(roomDid)) {
    hereDid = roomDid;
  } else if (isMaDidTarget(worldDid) && room) {
    hereDid = didWithFragment(worldDid, room);
  }
  if (hereDid) {
    next['@here'] = hereDid;
  }

  let changed = false;
  for (const key of SPECIAL_KEYS) {
    const expected = String(next[key] || '').trim();
    const current = String(state.aliasBook?.[key] || '').trim();
    if (expected) {
      if (current !== expected) {
        state.aliasBook[key] = expected;
        changed = true;
      }
    } else if (current) {
      delete state.aliasBook[key];
      changed = true;
    }
  }

  if (changed) {
    saveAliasBook();
    refreshDialogAndPresenceFormatting();
  }
}

function normalizeLegacySnapshotTarget(snapshot) {
  const targetRaw = String(snapshot?.target || '').trim();
  if (!targetRaw) {
    return '';
  }
  const hashIdx = targetRaw.indexOf('#');
  const token = hashIdx === -1 ? targetRaw : targetRaw.slice(0, hashIdx);
  const fragment = hashIdx === -1 ? '' : targetRaw.slice(hashIdx + 1);

  // Accept canonical values as-is.
  if (isMaDid(token) || isLikelyIrohAddress(normalizeIrohAddress(token))) {
    return targetRaw;
  }

  // Legacy snapshots may contain alias-like targets (e.g. @panteia#lobby).
  const resolved = String(resolveAliasInput(token) || '').trim();
  if (!resolved || resolved === token) {
    return '';
  }

  if (isMaDid(resolved)) {
    const room = String(fragment || '').trim();
    return room ? didWithFragment(resolved, room) : resolved;
  }

  const endpoint = normalizeIrohAddress(resolved);
  if (isLikelyIrohAddress(endpoint)) {
    return endpoint;
  }

  return '';
}

async function restoreActiveHomeAfterUnlock() {
  if (!state.identity?.did) {
    return;
  }

  const snapshot = loadActiveHomeSnapshot(state.identity.did);
  if (!snapshot?.target) {
    return;
  }

  const normalizedLegacyTarget = normalizeLegacySnapshotTarget(snapshot);
  if (!normalizedLegacyTarget) {
    appendSystemUi(
      'Skipping stale saved location target. Use go/home to reconnect manually.',
      'Hopper over utdatert lagret lokasjon. Bruk go/home for manuell reconnect.'
    );
    clearActiveHomeSnapshot(state.identity.did);
    return;
  }
  if (normalizedLegacyTarget !== snapshot.target) {
    snapshot.target = normalizedLegacyTarget;
    persistActiveHomeSnapshot(state.identity.did, {
      ...snapshot,
      target: normalizedLegacyTarget,
      savedAt: Date.now(),
    });
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

function readStoredDebugFlag() {
  const raw = localStorage.getItem(DEBUG_KEY);
  if (!raw) return false;
  const value = raw.trim().toLowerCase();
  return value === '1' || value === 'true' || value === 'on';
}

function normalizeLogLevel(value) {
  const level = String(value || '').trim().toLowerCase();
  if (level === 'error' || level === 'warn' || level === 'info' || level === 'debug') {
    return level;
  }
  return 'info';
}

function readStoredLogEnabledFlag() {
  const raw = String(localStorage.getItem(LOG_ENABLED_KEY) || '').trim().toLowerCase();
  if (!raw) return true;
  return raw === '1' || raw === 'true' || raw === 'on';
}

function readStoredLogLevel() {
  return normalizeLogLevel(localStorage.getItem(LOG_LEVEL_KEY));
}

function normalizeMessageTtl(value, fallback) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return fallback;
  }
  return Math.floor(parsed);
}

function readStoredMessageTtl(key, fallback) {
  const raw = localStorage.getItem(key);
  return normalizeMessageTtl(raw, fallback);
}

function persistMessageTtl() {
  localStorage.setItem(MSG_CHAT_TTL_KEY, String(state.messageTtl.chat));
  localStorage.setItem(MSG_CMD_TTL_KEY, String(state.messageTtl.cmd));
  localStorage.setItem(MSG_WHISPER_TTL_KEY, String(state.messageTtl.whisper));
}

function setMessageTtl(kind, ttlSeconds, announce = true) {
  const normalizedKind = String(kind || '').trim().toLowerCase();
  if (normalizedKind !== 'chat' && normalizedKind !== 'cmd' && normalizedKind !== 'whisper') {
    return false;
  }

  const fallback = normalizedKind === 'chat'
    ? DEFAULT_CHAT_TTL_SECONDS
    : normalizedKind === 'cmd'
      ? DEFAULT_CMD_TTL_SECONDS
      : DEFAULT_WHISPER_TTL_SECONDS;
  const next = normalizeMessageTtl(ttlSeconds, fallback);
  state.messageTtl[normalizedKind] = next;
  persistMessageTtl();
  if (announce) {
    appendMessage('system', `actor.msg.${normalizedKind}.ttl = ${next}`);
  }
  return true;
}

function getMessageTtl(kind) {
  const normalizedKind = String(kind || '').trim().toLowerCase();
  if (normalizedKind === 'chat') return state.messageTtl.chat;
  if (normalizedKind === 'cmd') return state.messageTtl.cmd;
  if (normalizedKind === 'whisper') return state.messageTtl.whisper;
  return 60;
}

function setTemporaryMessageTtlOverride(ttlSeconds, announce = true) {
  const parsed = Number(ttlSeconds);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return false;
  }
  state.temporaryMessageTtlOverride = Math.floor(parsed);
  if (announce) {
    appendMessage('system', `.ttl = ${state.temporaryMessageTtlOverride}`);
  }
  return true;
}

function clearTemporaryMessageTtlOverride(announce = true) {
  state.temporaryMessageTtlOverride = null;
  if (announce) {
    appendMessage('system', '.ttl = unset');
  }
}

function getTemporaryMessageTtlOverride() {
  const value = Number(state.temporaryMessageTtlOverride);
  if (!Number.isFinite(value) || value < 0) {
    return null;
  }
  return Math.floor(value);
}

function batchStatusLine() {
  const mode = state.batch.collecting ? 'collecting' : 'idle';
  return `.batch mode=${mode} timeout=${state.batch.timeoutSeconds}s retry=${state.batch.retryCount} queued=${state.batch.commands.length}`;
}

function setBatchTimeoutSeconds(timeoutSeconds, announce = true) {
  const parsed = Number(timeoutSeconds);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return false;
  }
  state.batch.timeoutSeconds = Math.floor(parsed);
  state.batch.collecting = true;
  state.batch.commands = [];
  if (announce) {
    appendMessage('system', `Batch started with timeout=${state.batch.timeoutSeconds}s.`);
    appendMessage('system', batchStatusLine());
  }
  return true;
}

function setBatchRetryCount(retryCount, announce = true) {
  const parsed = Number(retryCount);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return false;
  }
  state.batch.retryCount = Math.floor(parsed);
  if (announce) {
    appendMessage('system', `Batch retry=${state.batch.retryCount}.`);
    appendMessage('system', batchStatusLine());
  }
  return true;
}

function queueBatchCommand(commandText) {
  state.batch.commands.push(commandText);
}

function yamlScalar(value) {
  if (value === null || value === undefined) {
    return 'null';
  }
  if (typeof value === 'boolean') {
    return value ? 'true' : 'false';
  }
  if (typeof value === 'number' && Number.isFinite(value)) {
    return String(value);
  }
  const text = String(value);
  if (/^[A-Za-z0-9._:\/-]+$/.test(text)) {
    return text;
  }
  const escaped = text
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n');
  return `"${escaped}"`;
}

function maRpcYaml(payload) {
  const lines = ['---', 'ma:'];
  for (const [key, value] of Object.entries(payload || {})) {
    if (value === undefined) {
      continue;
    }
    lines.push(`  ${key}: ${yamlScalar(value)}`);
  }
  return lines.join('\n');
}

function appendMaRpc(payload) {
  const enriched = {
    version: 1,
    ...payload,
  };
  appendMessage('system', maRpcYaml(enriched));
}

function classifyRpcCode(message, status = 'failed') {
  if (status === 'ok') {
    return 'ok';
  }
  const text = String(message || '').trim().toLowerCase();
  if (!text) return 'request_failed';
  if (text.includes('timedout') || text.includes('timed out') || text.includes('timeout')) {
    return 'timeout';
  }
  if (text.includes('not found') || text.includes('unknown actor') || text.includes('unknown actor or object')) {
    return 'not_found';
  }
  if (text.includes('access denied') || text.includes('forbidden')) {
    return 'forbidden';
  }
  if (text.includes('invalid') || text.includes('usage:')) {
    return 'invalid_request';
  }
  if (text.includes("can't publish") || text.includes('publish failed')) {
    return 'publish_failed';
  }
  return 'request_failed';
}

async function runBatchCommands() {
  if (state.batch.running) {
    appendMessage('system', 'Batch is already running.');
    return;
  }

  const commands = state.batch.commands.slice();
  if (!commands.length) {
    appendMessage('system', 'Batch is empty.');
    appendMessage('system', batchStatusLine());
    return;
  }

  state.batch.collecting = false;
  state.batch.running = true;
  state.batch.commands = [];

  const timeoutSeconds = Math.max(1, Number(state.batch.timeoutSeconds || 10));
  const timeoutMs = timeoutSeconds * 1000;
  const retryCount = Math.max(0, Number(state.batch.retryCount || 0));
  const previousTemporaryTtl = getTemporaryMessageTtlOverride();
  setTemporaryMessageTtlOverride(timeoutSeconds, false);

  appendMessage('system', `Running batch: ${commands.length} command(s), timeout=${timeoutSeconds}s, retry=${retryCount}.`);
  appendMaRpc({
    status: 'ok',
    code: 'batch_started',
    content: 'batch started',
    commandCount: commands.length,
    timeout: timeoutSeconds,
    retryMax: retryCount,
  });

  try {
    for (let index = 0; index < commands.length; index += 1) {
      const command = String(commands[index] || '').trim();
      if (!command) {
        continue;
      }

      let succeeded = false;
      let lastError = '';
      for (let attempt = 0; attempt <= retryCount; attempt += 1) {
        const startedAtMs = Date.now();
        try {
          await withTimeout(
            sendWithActiveTargetRequirementsIfNeeded(command),
            timeoutMs,
            'timedout'
          );
          succeeded = true;
          appendMaRpc({
            status: 'ok',
            code: 'ok',
            content: `message received and applied (${command})`,
            command,
            commandIndex: index + 1,
            attempt: attempt + 1,
            elapsedMs: Date.now() - startedAtMs,
            retry: false,
          });
          break;
        } catch (error) {
          lastError = error instanceof Error ? error.message : String(error);
          const willRetry = attempt < retryCount;
          const code = classifyRpcCode(lastError, 'failed');
          appendMaRpc({
            status: 'failed',
            code,
            content: lastError || 'timedout',
            command,
            commandIndex: index + 1,
            attempt: attempt + 1,
            elapsedMs: Date.now() - startedAtMs,
            retry: willRetry,
            retryDelay: willRetry ? 1 : undefined,
            retryMax: retryCount,
          });
          if (attempt < retryCount) {
            appendMessage('system', `Batch retry ${attempt + 1}/${retryCount} for '${command}' after error: ${lastError}`);
            await delay(1000);
          }
        }
      }

      if (!succeeded) {
        throw new Error(`request failed with ${command}: ${lastError || 'timedout'}`);
      }
    }

    appendMessage('system', `Batch complete (${commands.length} command(s)).`);
    appendMaRpc({
      status: 'ok',
      code: 'batch_complete',
      content: 'batch complete',
      commandCount: commands.length,
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    appendMessage('system', message);
    appendMaRpc({
      status: 'failed',
      code: classifyRpcCode(message, 'failed'),
      content: message,
      retry: false,
    });
  } finally {
    if (previousTemporaryTtl === null) {
      clearTemporaryMessageTtlOverride(false);
    } else {
      setTemporaryMessageTtlOverride(previousTemporaryTtl, false);
    }
    state.batch.running = false;
  }
}

function setDebugMode(enabled, announce = true) {
  state.debug = Boolean(enabled);
  localStorage.setItem(DEBUG_KEY, state.debug ? '1' : '0');
  if (announce) {
    appendMessage('system', `Debug mode: ${state.debug ? 'on' : 'off'}`);
  }
}

function setLogEnabled(enabled, announce = true) {
  state.logEnabled = Boolean(enabled);
  localStorage.setItem(LOG_ENABLED_KEY, state.logEnabled ? '1' : '0');
  if (announce) {
    appendMessage('system', `Log enabled: ${state.logEnabled ? 'true' : 'false'}`);
  }
}

function setLogLevel(level, announce = true) {
  state.logLevel = normalizeLogLevel(level);
  localStorage.setItem(LOG_LEVEL_KEY, state.logLevel);
  if (announce) {
    appendMessage('system', `Log level: ${state.logLevel}`);
  }
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

const {
  initEditorEngineFromCdn,
  setEditorText,
  getEditorText,
  focusEditor,
  setEditorBusy,
  setEditorStatus,
  setEditorDisabled,
  onEditorModalVisibility,
  openEditorModal,
  closeEditorModal,
  onEditorModalKeyDown,
  updateEditorContext,
  updateEditorControls,
} = createEditorUi({
  byId,
  state,
  uiText,
  onEditorEngineStatus(message) {
    appendMessage('system', message);
  },
});

async function closeAndEvalEditorScript() {
  if (!state.editSession || state.editSession.mode !== 'script') {
    appendSystemUi(
      'Close and Eval is only available in .edit script mode.',
      'Lukk og Evaluer er kun tilgjengelig i .edit script-modus.'
    );
    return;
  }

  const scriptText = getEditorText();
  if (!scriptText.trim()) {
    setEditorStatus('Refusing to eval empty local script.', 'error');
    return;
  }

  localStorage.setItem(LOCAL_EDIT_SCRIPT_KEY, scriptText);
  localStorage.removeItem(LEGACY_LOCAL_EDIT_SCRIPT_CID_KEY);
  state.editSession.sourceCid = '(not published yet)';
  updateEditorContext();

  closeEditorModal();
  await evaluateScriptText(scriptText, 'local .edit script');
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
  if (isMaDid(target)) {
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


async function appendAmbientProseAfterSpeech() {
  if (!state.currentHome) {
    return;
  }

  const info = await fetchCurrentRoomInspectData();
  const languageKey = roomLanguageKey(state.uiLang);
  const roomDescription = String(state.currentHome.roomDescription || '').trim() || uiText('(no description)', '(ingen beskrivelse)');
  const exits = Object.entries(info.exitCidMap);
  const labels = [];

  for (const [exitId, exitCid] of exits) {
    try {
      const exitYaml = await fetchGatewayTextByPath(asIpfsGatewayPath(exitCid));
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
      const exitYaml = await fetchGatewayTextByPath(asIpfsGatewayPath(exitCid));
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

async function lookupDidInCurrentRoom(query) {
  const token = String(query || '').trim().replace(/^@+/, '');
  if (!token) {
    throw new Error(uiText('Usage: .use <object|did> [as @alias]', 'Bruk: .use <objekt|did> [as @alias]'));
  }
  if (isMaDid(token)) {
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
    const candidates = token.startsWith('#')
      ? [token, token.slice(1)]
      : [token];

    for (const candidate of candidates) {
      const normalized = String(candidate || '').trim();
      if (!normalized) {
        continue;
      }
      const response = await sendWorldCommandQuery(`@here id ${normalized}`);
      const did = extractDidFromLookupResponse(response);
      if (did) {
        cacheRoomDidLookup(token, did);
        cacheRoomDidLookup(normalized, did);
        return did;
      }
    }

    throw new Error(uiText(
      `Could not resolve DID for '${token}'.`,
      `Fant ikke DID for '${token}'.`
    ));
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

function lookupHandleDid(handle) {
  const key = String(handle || '').trim();
  if (!key) {
    return '';
  }
  const direct = String(state.handleDidMap[key] || '').trim();
  if (isMaDid(direct)) {
    return direct;
  }
  const lowered = key.toLowerCase();
  for (const [knownHandle, knownDid] of Object.entries(state.handleDidMap || {})) {
    if (String(knownHandle || '').trim().toLowerCase() === lowered && isMaDid(String(knownDid || '').trim())) {
      return String(knownDid || '').trim();
    }
  }
  return '';
}

async function resolveCommandTargetDidOrToken(targetToken) {
  const raw = String(targetToken || '').trim().replace(/^@+/, '');
  if (!raw) {
    throw new Error('Usage: @target <command>');
  }
  const rawLower = raw.toLowerCase();
  if (rawLower.startsWith('my.')) {
    const aliasName = String(raw.slice(3) || '').trim();
    if (!aliasName) {
      throw new Error('Usage: @my.<alias> <command>');
    }

    const aliasDirect = String(state.aliasBook?.[aliasName] || '').trim();
    const aliasWithAt = String(state.aliasBook?.[`@${aliasName}`] || '').trim();
    const resolvedAlias = String(aliasDirect || aliasWithAt || resolveAliasInput(aliasName) || '').trim();
    if (!resolvedAlias) {
      throw new Error(`@my.${aliasName} is not defined in my.aliases.`);
    }

    if (isMaDid(resolvedAlias)) {
      return resolvedAlias;
    }

    if (resolvedAlias.startsWith('@')) {
      const aliasTarget = String(resolvedAlias || '').trim().replace(/^@+/, '');
      const nestedAliasDirect = String(state.aliasBook?.[aliasTarget] || '').trim();
      const nestedAliasWithAt = String(state.aliasBook?.[`@${aliasTarget}`] || '').trim();
      const nestedResolved = String(
        nestedAliasDirect
        || nestedAliasWithAt
        || resolveAliasInput(aliasTarget)
        || ''
      ).trim();

      if (isMaDid(aliasTarget)) {
        return aliasTarget;
      }
      if (isMaDid(nestedResolved)) {
        return nestedResolved;
      }
    }

    throw new Error(`@my.${aliasName} must resolve to did:ma.`);
  }

  if (raw.toLowerCase() === 'world') {
    const worldDid = currentWorldDid();
    if (isMaDidTarget(worldDid) && worldDid.includes('#') && !isUnconfiguredDidTarget(worldDid)) {
      return worldDid;
    }
    throw new Error('@world requires a valid world did:ma target with #fragment. Reconnect to refresh world DID.');
  }
  const activeAliasRaw = String(state.activeObjectTargetAlias || '').trim().replace(/^@+/, '');
  const activeDid = String(state.activeObjectTargetDid || '').trim();
  if (activeAliasRaw && activeAliasRaw.toLowerCase() === raw.toLowerCase() && isMaDid(activeDid)) {
    cacheRoomDidLookup(raw, activeDid);
    return activeDid;
  }
  if (isMaDid(raw)) {
    return raw;
  }

  if (raw.startsWith('#')) {
    const fragment = raw.slice(1).trim().toLowerCase();
    if (fragment) {
      for (const entry of state.roomPresence.values()) {
        const did = String(entry?.did || '').trim();
        if (!isMaDid(did) || !did.includes('#')) {
          continue;
        }
        const didFragment = String(did.split('#')[1] || '').trim().toLowerCase();
        if (didFragment && didFragment === fragment) {
          cacheRoomDidLookup(raw, did);
          return did;
        }
      }
    }
  }

  const mappedDid = lookupHandleDid(raw) || '';
  if (isMaDid(String(mappedDid))) {
    cacheRoomDidLookup(raw, mappedDid);
    return mappedDid;
  }

  if (isBuiltinTargetToken(raw)) {
    return raw;
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

  if (kv.cid) {
    appendMessage('system', `  cid: ${kv.cid}`);
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

function encodeUtf8Base64(text) {
  const bytes = new TextEncoder().encode(String(text || ''));
  let binary = '';
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

async function sendWorldCommandQuery(commandText) {
  if (!worldDispatchFlow) {
    throw new Error('World dispatch is not initialized yet.');
  }
  return await worldDispatchFlow.sendWorldCommandQuery(commandText);
}

function currentWorldDid() {
  const worldAlias = String(state.aliasBook?.['@world'] || '').trim();
  if (isMaDidTarget(worldAlias) && worldAlias.includes('#') && !isUnconfiguredDidTarget(worldAlias)) {
    return worldAlias;
  }
  const worldDid = String(state.currentHome?.worldDid || '').trim();
  if (isMaDidTarget(worldDid) && worldDid.includes('#') && !isUnconfiguredDidTarget(worldDid)) {
    return worldDid;
  }
  return '';
}

async function sendCurrentHomePresencePing() {
  if (!state.currentHome || !worldDispatchFlow) {
    return;
  }
  if (state.pingInFlight) {
    return;
  }
  state.pingInFlight = true;
  try {
    const localRoomDid = String(state.currentHome?.roomDid || '').trim();
    const pingTarget = localRoomDid || String(state.currentHome?.room || '').trim() || 'lobby';
    const pingRaw = await ping_world(
      state.currentHome.endpointId,
      state.passphrase,
      state.encryptedBundle,
      currentActorFragment(),
      pingTarget
    );
    const pingResult = JSON.parse(String(pingRaw || '{}'));
    const response = pingResult?.response || {};

    const serverRoomDid = String(response.room_did || '').trim();
    const serverRoom = String(response.room || '').trim();

    if (state.currentHome && (serverRoomDid || serverRoom)) {
      const beforeDid = String(state.currentHome.roomDid || '').trim();
      const beforeRoom = String(state.currentHome.room || '').trim();
      const didChanged = Boolean(serverRoomDid && beforeDid && serverRoomDid !== beforeDid);
      const roomChanged = Boolean(serverRoom && beforeRoom && serverRoom !== beforeRoom);

      if (didChanged || roomChanged) {
        logger.log('presence.ping', `room truth update: local=${beforeDid || beforeRoom} server=${serverRoomDid || serverRoom}`);
      }

      if (serverRoom) {
        state.currentHome.room = serverRoom;
      }
      if (serverRoomDid) {
        state.currentHome.roomDid = serverRoomDid;
      }
      if (typeof response.room_title === 'string' && response.room_title) {
        state.currentHome.roomTitle = response.room_title;
      }
      if (typeof response.room_description === 'string') {
        state.currentHome.roomDescription = response.room_description;
      }
      if (typeof response.handle === 'string' && response.handle) {
        state.currentHome.handle = response.handle;
      }

      updateRoomHeading(state.currentHome.roomTitle || '', state.currentHome.roomDescription || '');
      syncSpecialAliasesFromCurrentHome();
      saveActiveHomeSnapshot();

      if (didChanged || roomChanged) {
        try {
          await pollCurrentHomeEvents();
        } catch (error) {
          logger.log('presence.ping', `manual room poll after drift failed: ${error instanceof Error ? error.message : String(error)}`);
        }
      }
    }

    // Ping success — reset fibonacci and clear any retry timer.
    state.pingFibPrev = PING_FIB_START;
    state.pingFibCurr = PING_FIB_START;
    if (state.pingRetryTimer) {
      clearTimeout(state.pingRetryTimer);
      state.pingRetryTimer = null;
    }
  } catch (error) {
    logger.log('presence.ping', `failed: ${error instanceof Error ? error.message : String(error)}`);
    schedulePingRetry();
  } finally {
    state.pingInFlight = false;
  }
}

function schedulePingRetry() {
  if (state.pingRetryTimer) return; // Already scheduled.
  if (state.pingFibCurr >= PING_WORLD_GONE_THRESHOLD) {
    logger.log('presence.ping', `world unresponsive after ${state.pingFibCurr}ms fibonacci — declaring world gone`);
    if (!state.pollErrorShown) {
      appendSystemUi(
        'World appears unreachable. Attempting re-entry...',
        'Verden er utilgjengelig. Forsøker re-entry...'
      );
      state.pollErrorShown = true;
    }
    state.pingFibPrev = PING_FIB_START;
    state.pingFibCurr = PING_FIB_START;
    requestReentry('world gone');
    return;
  }

  const delayMs = state.pingFibCurr;
  const next = Math.min(state.pingFibPrev + state.pingFibCurr, PING_WORLD_GONE_THRESHOLD);
  state.pingFibPrev = state.pingFibCurr;
  state.pingFibCurr = next;

  logger.log('presence.ping', `scheduling retry in ${delayMs}ms (next=${next}ms)`);
  state.pingRetryTimer = setTimeout(() => {
    state.pingRetryTimer = null;
    sendCurrentHomePresencePing().catch(() => {});
  }, delayMs);
}

async function loadLocalScriptEditor() {
  localStorage.removeItem(LEGACY_LOCAL_EDIT_SCRIPT_CID_KEY);
  state.editSession = {
    mode: 'script',
    target: 'local-script',
    sourceCid: '(local only)'
  };

  setEditorText(localStorage.getItem(LOCAL_EDIT_SCRIPT_KEY) || '');

  updateEditorContext();
  setEditorStatus('Local script mode.', 'ok');
  openEditorModal();
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

async function executeScriptLine(line) {
  const text = String(line || '').trim();
  if (!text || text.startsWith('#')) {
    return;
  }

  if (text.startsWith('.')) {
    const dot = text.slice(1).trim();
    const [verbRaw] = dot.split(/\s+/);
    const dotCommand = String(verbRaw || '').toLowerCase();

    if (!dotCommand) {
      return;
    }

    if (dotCommand === 'edit' || dotCommand === 'eval') {
      throw new Error(`Script line '${text}' is not allowed inside .eval script`);
    }
    if (dotCommand === 'help') {
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

async function evaluateScriptText(scriptText, sourceLabel = 'script') {
  appendMessage('system', `Evaluating ${sourceLabel}...`);
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
}

async function onDotEval(rawArgs) {
  try {
    const resolved = resolveEvalSourceToken(rawArgs);
    const scriptText = await fetchGatewayTextByPath(asIpfsGatewayPath(resolved));
    await evaluateScriptText(scriptText, `script from ${resolved}`);
  } catch (error) {
    appendMessage('system', `Eval failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}

async function loadAvatarEditor() {
  if (!state.currentHome) {
    throw new Error('Not connected to a world. Connect first.');
  }

  setEditorBusy(true);
  setEditorStatus('Loading avatar state from @me show...', 'working');

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

    setEditorText(yamlDraft);

    updateEditorContext();
    setEditorStatus('Loaded avatar draft.', 'ok');
    openEditorModal();
  } finally {
    setEditorBusy(false);
  }
}

async function loadEditorForTarget(target, announce = true) {
  setEditorBusy(true);
  setEditorStatus(`Loading source YAML for room '${target}'...`, 'working');

  try {
    const showResponse = await sendWorldCommandQuery(`@world show #${target}`);
    if (showResponse.includes('not found')) {
      throw new Error(showResponse);
    }

    const showMeta = parseRoomShowMeta(showResponse);
    const sourceCid = extractRoomCidFromShowResponse(showResponse);
    const cidMissing = !sourceCid || sourceCid === '(unknown)';

    if (cidMissing) {
      throw new Error(`No room CID available for '${target}'. Response: ${showResponse}`);
    }

    const yamlText = await fetchGatewayTextByPath(asIpfsGatewayPath(sourceCid));
    const safeYamlText = sanitizeRoomYamlForEdit(yamlText);

    state.editSession = {
      mode: 'room',
      target,
      sourceCid: sourceCid || '(runtime)'
    };

    setEditorText(safeYamlText);

    updateEditorContext();
    setEditorStatus(`Loaded room '${target}' from ${sourceCid}.`, 'ok');
    openEditorModal();

    if (announce) {
      appendMessage('system', `Loaded .edit source for room '${target}' from CID ${sourceCid}.`);
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setEditorStatus(`Load failed: ${message}`, 'error');
    throw error;
  } finally {
    setEditorBusy(false);
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

  setEditorBusy(true);
  setEditorStatus(`Loading exit '${query}'...`, 'working');

  try {
    const info = await fetchCurrentRoomInspectData();
    const exits = Object.entries(info.exitCidMap);
    if (!exits.length) {
      throw new Error('No exits found in current room content.');
    }

    const target = query.toLowerCase();
    let matched = null;

    for (const [exitId, exitCid] of exits) {
      const exitYaml = await fetchGatewayTextByPath(asIpfsGatewayPath(exitCid));
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

    setEditorText(matched.exitYaml);

    updateEditorContext();
    setEditorStatus(`Loaded exit '${matched.exitId}' from ${matched.exitCid}.`, 'ok');
    openEditorModal();
    appendMessage('system', `Loaded .edit source for exit '${matched.exitId}' from CID ${matched.exitCid}.`);
  } finally {
    setEditorBusy(false);
  }
}

async function saveEditorChanges() {
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

  const yamlText = getEditorText();
  if (!yamlText.trim()) {
    setEditorStatus('Refusing to save empty content.', 'error');
    return;
  }

  if (state.editSession.mode === 'script') {
    setEditorBusy(true);
    setEditorStatus('Saving local script...', 'working');
    try {
      localStorage.setItem(LOCAL_EDIT_SCRIPT_KEY, yamlText);
      localStorage.removeItem(LEGACY_LOCAL_EDIT_SCRIPT_CID_KEY);
      state.editSession.sourceCid = '(not published yet)';
      updateEditorContext();
      setEditorStatus('Saved local script.', 'ok');
      appendMessage('system', 'Saved local .edit script.');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setEditorStatus(`Script save failed: ${message}`, 'error');
      appendMessage('system', `Script save failed: ${message}`);
    } finally {
      setEditorBusy(false);
    }
    return;
  }

  if (state.editSession.mode === 'avatar') {
    const description = parseDescriptionFromEditorText(yamlText);
    if (!description) {
      setEditorStatus('Avatar description is empty.', 'error');
      return;
    }

    setEditorBusy(true);
    setEditorStatus('Applying avatar update via @me describe ...', 'working');
    try {
      await sendCurrentWorldMessage(`@me describe ${description}`);
      setEditorStatus('Avatar updated.', 'ok');
      appendMessage('system', 'Applied avatar edit from .edit @me.');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setEditorStatus(`Avatar update failed: ${message}`, 'error');
      appendMessage('system', `Avatar edit failed: ${message}`);
    } finally {
      setEditorBusy(false);
    }
    return;
  }

  if (state.editSession.mode === 'exit') {
    const exitId = String(state.editSession.exitId || '').trim();
    if (!exitId) {
      setEditorStatus('Exit edit session is missing exit id.', 'error');
      return;
    }

    if (String(state.currentHome.room || '').trim() !== String(state.editSession.roomTarget || '').trim()) {
      setEditorStatus('Exit edits can only be saved from the room where the exit lives.', 'error');
      appendMessage('system', `Enter room '${state.editSession.roomTarget}' and save again.`);
      return;
    }

    setEditorBusy(true);
    setEditorStatus(`Sending YAML for exit '${exitId}' to world...`, 'working');
    try {
      const payload = encodeUtf8Base64(yamlText);
      const reply = await sendWorldCommandQuery(`@here.exit-content-b64 ${exitId} ${payload}`);
      const exitCidMatch = reply.match(/published as\s+([A-Za-z0-9]+)/i);
      const roomCidMatch = reply.match(/updated to\s+([A-Za-z0-9]+)/i);
      if (exitCidMatch && exitCidMatch[1]) {
        state.editSession.sourceCid = exitCidMatch[1];
      }
      if (roomCidMatch && roomCidMatch[1]) {
        state.editSession.roomCid = roomCidMatch[1];
      }
      updateEditorContext();

      appendMessage('system', reply);
      setEditorStatus(`Exit '${exitId}' saved via world.`, 'ok');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setEditorStatus(`Exit save failed: ${message}`, 'error');
      appendMessage('system', `Exit edit failed: ${message}`);
    } finally {
      setEditorBusy(false);
    }
    return;
  }

  setEditorBusy(true);
  setEditorStatus(`Sending YAML for room '${state.editSession.target}' to world...`, 'working');

  try {
    if (state.currentHome.room !== state.editSession.target) {
      throw new Error(`Enter room '${state.editSession.target}' and save there, because @here applies to your current room.`);
    }

    const safeRoomYaml = sanitizeRoomYamlForEdit(yamlText);
    const payload = encodeUtf8Base64(safeRoomYaml);
    const reply = await sendWorldCommandQuery(`@here.content-b64 ${payload}`);
    const cidMatch = reply.match(/as\s+([A-Za-z0-9]+)/i);
    if (cidMatch && cidMatch[1]) {
      state.editSession.sourceCid = cidMatch[1];
    }
    updateEditorContext();

    appendMessage('system', reply);
    setEditorStatus(`Room '${state.editSession.target}' saved via world.`, 'ok');
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setEditorStatus(`Save failed: ${message}`, 'error');
    appendMessage('system', `Edit publish failed: ${message}`);
  } finally {
    setEditorBusy(false);
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
      await loadEditorForTarget(target);
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

    if (isMaDid(arg)) {
      const target = normalizeEditTarget(arg);
      await loadEditorForTarget(target);
      return;
    }

    throw new Error('Usage: .edit | .edit @here | .edit @me | .edit @exit <name|alias> | .edit did:ma:<world>#<room>');
  } catch (error) {
    appendMessage('system', `Edit failed: ${error instanceof Error ? error.message : String(error)}`);
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

const { updateDocumentTitle, updateLocationContext } = createWorldTitleFlow({
  state,
  properName: PROPER_NAME,
});

const { trackRoomPresence, removeRoomPresence, clearRoomPresence } = createRoomPresenceFlow({
  state,
  cacheRoomDidLookup,
  dropCachedRoomDidLookup,
  renderAvatarPanel,
});

function renderAvatarPanel() {
  const list = byId('avatar-list');
  if (!list) return;
  list.innerHTML = '';
  const sorted = Array.from(state.roomPresence.values()).sort((a, b) => {
    const left = String(a?.did || a?.handle || '').toLowerCase();
    const right = String(b?.did || b?.handle || '').toLowerCase();
    return left.localeCompare(right);
  });
  for (const entry of sorted) {
    const li = document.createElement('li');
    li.className = 'avatar-item';
    const didText = String(entry?.did || '').trim();
    li.textContent = didText ? formatDidForDialog(didText) : String(entry?.handle || '').trim();
    if (didText) {
      li.title = didText;
    }
    list.appendChild(li);
  }
}

const { applyPresencePayload } = createRoomPresencePayloadFlow({
  state,
  updateRoomHeading: (...args) => updateRoomHeading(...args),
  trackRoomPresence,
  removeRoomPresence,
  clearRoomPresence,
  appendMessage: (...args) => appendMessage(...args),
});

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

function setSetupStatus(message) {
  byId('setup-status').textContent = message;
}

function didParts(value) {
  const source = String(value || '').trim();
  if (!source.startsWith('did:ma:')) {
    return { root: '', fragment: '' };
  }
  const hash = source.indexOf('#');
  if (hash === -1) {
    return { root: source, fragment: '' };
  }
  return {
    root: source.slice(0, hash),
    fragment: source.slice(hash + 1),
  };
}

function aliasForDid(did) {
  const target = String(did || '').trim();
  if (!target) {
    return '';
  }

  const entries = Object.entries(state.aliasBook || {});
  const candidates = entries
    .filter(([, address]) => String(address || '').trim() === target)
    .map(([alias]) => String(alias || '').trim())
    .filter(Boolean);

  if (!candidates.length) {
    return '';
  }

  const hashIdx = target.indexOf('#');
  const fragment = hashIdx === -1 ? '' : target.slice(hashIdx + 1).trim().toLowerCase();
  const reserved = new Set(['world', '@world', 'here', '@here', 'me', '@me', 'avatar', '@avatar']);

  const score = (alias) => {
    const lowered = alias.toLowerCase();
    const stripped = lowered.replace(/^@+/, '');
    if (fragment && stripped === fragment) {
      return lowered.startsWith('@') ? 90 : 100;
    }
    if (!reserved.has(lowered)) {
      return lowered.startsWith('@') ? 60 : 70;
    }
    return 10;
  };

  candidates.sort((a, b) => {
    const sa = score(a);
    const sb = score(b);
    if (sa !== sb) {
      return sb - sa;
    }
    if (a.length !== b.length) {
      return a.length - b.length;
    }
    return a.localeCompare(b);
  });

  return candidates[0] || '';
}

function formatDidForDialog(value) {
  const source = String(value || '').trim();
  if (!source) {
    return '';
  }
  if (!state.aliasRenderEnabled) {
    return source;
  }

  // Alias mode: exact full-DID match only.
  if (state.dialogIdStyle === 'alias') {
    const alias = aliasForDid(source);
    return alias || source;
  }

  const parts = didParts(source);
  if (!parts.root) {
    return source;
  }
  if (state.dialogIdStyle === 'did') {
    return source;
  }
  if (state.dialogIdStyle === 'fragment') {
    if (parts.fragment) {
      return `#${parts.fragment}`;
    }
    return parts.root;
  }
  return source;
}

function dialogText(text) {
  const source = String(text || '');
  const lowered = source.trim().toLowerCase();

  // Never rewrite DID values in document/JSON output.
  if (lowered.startsWith('document.')
    || lowered.startsWith('{')
    || lowered.startsWith('[')
    || source.includes('\n{')
    || source.includes('\n[')
    || source.includes('"verificationMethod"')
    || source.includes('"assertionMethod"')
    || source.includes('"keyAgreement"')
    || source.includes('"proof"')
    || source.includes('"id": "did:ma:')) {
    return source;
  }

  if (lowered.startsWith('my.aliases')
    || lowered.startsWith('alias saved:')
    || lowered.startsWith('alias removed:')
    || lowered.startsWith('alias not found:')
    || lowered.startsWith('usage: my.aliases')
    || lowered.startsWith('resolving did target ')
    || lowered.startsWith('connecting to ')
    || lowered.startsWith('send failed:')
    || lowered.startsWith('could not restore last location:')
    || lowered.includes('unknown actor or object')
    || lowered.startsWith('did resolve note:')) {
    return source;
  }
  return source.replace(/did:ma:[A-Za-z0-9]+(?:#[A-Za-z0-9._:-]+)?/g, (match) => formatDidForDialog(match));
}

function setAliasRewriteEnabled(enabled) {
  const next = Boolean(enabled);
  state.aliasRewriteEnabled = next;
  localStorage.setItem(ALIAS_REWRITE_ENABLED_KEY, next ? 'true' : 'false');
  return true;
}

function setAliasRenderEnabled(enabled) {
  const next = Boolean(enabled);
  state.aliasRenderEnabled = next;
  localStorage.setItem(ALIAS_RENDER_ENABLED_KEY, next ? 'true' : 'false');
  updateAliasToggleButton();
  refreshDialogAndPresenceFormatting();
  return true;
}

function toggleAliasRenderEnabled() {
  setAliasRenderEnabled(!state.aliasRenderEnabled);
}

function updateAliasToggleButton() {
  const btn = byId('btn-alias');
  if (!btn) return;
  btn.textContent = 'Aliasing';
  btn.setAttribute('aria-pressed', state.aliasRenderEnabled ? 'true' : 'false');
  btn.classList.toggle('aliasing-inverted', !state.aliasRenderEnabled);
}

function displayActor(senderDid, senderHandle) {
  const handle = String(senderHandle || '').trim();
  const did = String(senderDid || '').trim();

  if (did) {
    return did;
  }

  if (handle) {
    return handle;
  }

  return 'unknown';
}

function actorFragmentFromDid(did) {
  const value = String(did || '').trim();
  const idx = value.indexOf('#');
  if (idx === -1 || idx >= value.length - 1) {
    return '';
  }
  return value.slice(idx + 1).trim();
}

function currentActorFragment() {
  const fragment = actorFragmentFromDid(state.identity?.did).replace(/^@+/, '');
  if (fragment) {
    return fragment;
  }
  return String(state.aliasName || 'actor').trim().replace(/^@+/, '');
}

function currentAvatarDid() {
  const aliasAvatar = String(state.aliasBook?.['@avatar'] || state.aliasBook?.avatar || '').trim();
  if (isMaDidTarget(aliasAvatar)) {
    return aliasAvatar;
  }

  const worldDid = currentWorldDid();
  const handle = String(state.currentHome?.handle || '').trim().replace(/^@+/, '');
  if (worldDid && handle && !/\s/u.test(handle)) {
    return didWithFragment(worldDid, handle);
  }
  return '';
}

function setDialogIdStyle(style) {
  const normalized = String(style || '').trim().toLowerCase();
  if (normalized !== 'alias' && normalized !== 'fragment' && normalized !== 'did') {
    return false;
  }
  state.dialogIdStyle = normalized;
  localStorage.setItem(DIALOG_ID_STYLE_KEY, normalized);
  refreshDialogAndPresenceFormatting();
  return true;
}

function recordMatrixLine(role, message) {
  const entry = {
    at: new Date().toISOString(),
    role: String(role || 'world'),
    message: String(message || ''),
  };
  state.matrixLog.push(entry);
  if (state.matrixLog.length > 5000) {
    state.matrixLog.splice(0, state.matrixLog.length - 5000);
  }
  if (!byId('matrix-modal')?.classList.contains('hidden')) {
    renderMatrixView();
  }
}

function recordCommandIo(direction, text) {
  const dir = String(direction || '').toLowerCase();
  const label = dir === 'out' ? 'out' : 'in';
  recordMatrixLine(label, text);
}

function renderTranscriptFromMatrix() {
  const transcript = byId('transcript');
  if (!transcript) return;
  transcript.innerHTML = '';
  for (const entry of state.matrixLog) {
    const row = document.createElement('div');
    row.className = `msg ${entry.role}`;
    const text = document.createElement('p');
    text.textContent = dialogText(entry.message);
    row.appendChild(text);
    transcript.appendChild(row);
  }
  transcript.scrollTop = transcript.scrollHeight;
}

function renderMatrixView() {
  const pre = byId('matrix-text');
  if (!pre) return;
  const lines = state.matrixLog.map((entry) => {
    const at = String(entry.at || '');
    const role = String(entry.role || 'in');
    const message = String(entry.message || '');
    return `[${at}] ${role}: ${message}`;
  });
  pre.textContent = lines.length ? lines.join('\n') : '(empty)';
  pre.scrollTop = pre.scrollHeight;
}

function openMatrixModal() {
  const modal = byId('matrix-modal');
  if (!modal) return;
  renderMatrixView();
  modal.classList.remove('hidden');
  modal.setAttribute('aria-hidden', 'false');
}

function closeMatrixModal() {
  const modal = byId('matrix-modal');
  if (!modal) return;
  modal.classList.add('hidden');
  modal.setAttribute('aria-hidden', 'true');
}

function openHelpModal() {
  const modal = byId('help-modal');
  if (!modal) return;
  modal.classList.remove('hidden');
  modal.setAttribute('aria-hidden', 'false');
}

function closeHelpModal() {
  const modal = byId('help-modal');
  if (!modal) return;
  modal.classList.add('hidden');
  modal.setAttribute('aria-hidden', 'true');
}

function refreshDialogAndPresenceFormatting() {
  renderTranscriptFromMatrix();
  renderAvatarPanel();
  renderMatrixView();
}

function showLockOverlayTracked() {
  state.lockOverlayShownAtMs = Date.now();
  showLockOverlay();
}

function onLockOverlayClick(event) {
  const elapsed = Date.now() - Number(state.lockOverlayShownAtMs || 0);
  if (elapsed < 250) {
    return;
  }
  if (event.target !== event.currentTarget) {
    return;
  }
  hideLockOverlay();
}

function renderLocalBroadcastMessage(text) {
  const payload = String(text || '').trim();
  if (!payload) return;
  const actor = displayActor(state.identity?.did, '');
  appendMessage('world', `${actor}: ${payload}`);
}

const {
  hideLockOverlay,
  showLockOverlay,
  onLockOverlayKeydown,
  setGatewayStatus,
  setGatewayInstallNoteVisible,
} = createUiFlow({
  byId,
  state,
  setSetupStatus,
  ipfsGatewayFallbacks: IPFS_GATEWAY_FALLBACKS,
});

const dialogWriter = createDialogWriter({ byId, displayActor, formatDialogText: dialogText });
const appendMessage = (role, message) => {
  recordCommandIo('in', `${String(role || 'world')}: ${String(message || '')}`);
  dialogWriter.appendMessage(role, message);
};

const { fetchCurrentRoomInspectData, inspectExitByQuery } = createRoomInspectFlow({
  state,
  sendWorldCommandQuery,
  parseRoomShowMeta,
  extractRoomCidFromShowResponse,
  fetchGatewayTextByPath,
  asIpfsGatewayPath,
  uiText,
  appendMessage,
});

// Logging system: always visible in browser console; mirrored in chat when debug mode is enabled.
const logger = {
  shouldLog(level) {
    if (!state.logEnabled) return false;
    const rank = { debug: 10, info: 20, warn: 30, error: 40 };
    const configured = rank[normalizeLogLevel(state.logLevel)] || rank.info;
    const incoming = rank[normalizeLogLevel(level)] || rank.info;
    return incoming >= configured;
  },
  log(scope, ...args) {
    if (!this.shouldLog('info')) return;
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
    const line = `[${scope}] ${message}`;
    console.info(`[ma] ${line}`);
    if (state.debug) {
      appendMessage('system', line);
    }
  },
  warn(scope, ...args) {
    if (!this.shouldLog('warn')) return;
    const message = args.map(arg => String(arg)).join(' ');
    console.warn(`[ma] [${scope}] ${message}`);
    if (state.debug) {
      appendMessage('system', `[${scope}] ${message}`);
    }
  },
  error(scope, ...args) {
    if (!this.shouldLog('error')) return;
    const message = args.map(arg => String(arg)).join(' ');
    console.error(`[ma] [${scope}] ${message}`);
    if (state.debug) {
      appendMessage('system', `[${scope}] ${message}`);
    }
  }
};

didDocFlow = createDidDocFlow({
  state,
  didRoot,
  fetchGatewayTextByPath,
  logger,
  parseEnterDirective,
  extractWorldEndpointFromDidDoc,
  appendMessage,
  enterHome,
  didDocCacheTtlMs: DID_DOC_CACHE_TTL_MS,
});

const worldFlow = createWorldFlow({
  state,
  appendMessage,
  sendWorldCommandQuery,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  refillCommandInputWithActiveTarget,
  logger,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
  buildCurrentHomeResumeTarget,
  enterHome,
  requestReentry,
});

const {
  isNotRegisteredInRoomMessage,
  isActiveTargetGoneMessage,
  reportActiveTargetVanished,
  performTransparentReentry,
} = worldFlow;

const whisperFlow = createWhisperFlow({
  state,
  resolveAliasInput,
  findDidByEndpoint,
  fetchDidDocumentJsonByDid,
  sendWorldWhisper: send_world_whisper,
  sendWorldWhisperWithTtl: send_world_whisper_with_ttl,
  sendDirectMessage: send_direct_message,
  sendDirectMessageWithTtl: send_direct_message_with_ttl,
  getMessageTtl,
});

const { sendWhisperToDid, sendMessageToDid } = whisperFlow;

function stopHomeEventPolling() {
  if (state.roomPollTimer) {
    clearInterval(state.roomPollTimer);
    state.roomPollTimer = null;
  }
  if (state.avatarPingTimer) {
    clearInterval(state.avatarPingTimer);
    state.avatarPingTimer = null;
  }
  if (state.pingRetryTimer) {
    clearTimeout(state.pingRetryTimer);
    state.pingRetryTimer = null;
  }
  state.roomPollInFlight = false;
  state.inboxPollInFlight = false;
  state.pingInFlight = false;
  state.pollErrorShown = false;
}

const inboundDispatcher = createInboundDispatcher({
  state,
  logger,
  appendMessage,
  displayActor,
  fetchDidDocumentJsonByDid,
  decodeChatEventMessage: decode_chat_event_message,
  decodeWhisperEventMessage: decode_whisper_event_message,
  onPresenceEvent: async (payload) => {
    const kind = String(payload?.kind || '').trim();
    if (kind === 'presence.room_state' && state.currentHome) {
      const room = String(payload?.room || '').trim();
      const roomDid = String(payload?.room_did || '').trim();

      if (room) {
        state.currentHome.room = room;
      }
      if (roomDid) {
        state.currentHome.roomDid = roomDid;
      }
      if (typeof payload?.room_title === 'string' && payload.room_title) {
        state.currentHome.roomTitle = payload.room_title;
      }
      if (typeof payload?.room_description === 'string') {
        state.currentHome.roomDescription = payload.room_description;
      }
      if (typeof payload?.latest_event_sequence === 'number') {
        state.currentHome.lastEventSequence = Math.max(
          toSequenceNumber(state.currentHome.lastEventSequence || 0),
          toSequenceNumber(payload.latest_event_sequence)
        );
      }

      clearRoomPresence();
      if (Array.isArray(payload?.avatars)) {
        for (const avatar of payload.avatars) {
          const handle = String(avatar?.handle || '').trim();
          const did = String(avatar?.did || '').trim();
          if (handle) {
            trackRoomPresence(handle, did);
          }
        }
      }

      if (payload?.room_object_dids && typeof payload.room_object_dids === 'object') {
        primeDidLookupCacheFromRoomObjectDids(payload.room_object_dids);
      }

      updateRoomHeading(state.currentHome.roomTitle || '', state.currentHome.roomDescription || '');
      syncSpecialAliasesFromCurrentHome();
      saveLastRoom(state.currentHome.endpointId, state.currentHome.room || 'lobby');
      saveActiveHomeSnapshot();
      updateIdentityLine();
      logger.log('presence.room_state', `applied forced room-state for room=${state.currentHome.room} did=${state.currentHome.roomDid}`);
      return;
    }

    applyPresencePayload(payload);
  },
  onPresenceRefreshRequest: async () => {
    try {
      await sendCurrentHomePresencePing();
    } catch (error) {
      logger.log('presence.refresh', `ping response failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  },
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
        currentActorFragment(),
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
    state.consecutivePollFailures = 0;
    state.pollErrorShown = false;
  } catch (error) {
    const elapsed = Date.now() - pollStart;
    logger.log('poll.events', `failed after ${elapsed}ms: ${error instanceof Error ? error.message : String(error)}`);
    throw error;
  } finally {
    state.roomPollInFlight = false;
  }
}

function requestReentry(reason) {
  if (state.reentryInProgress) {
    return state.reentryInProgress;
  }

  if (!state.currentHome) {
    return Promise.resolve();
  }

  const home = state.currentHome;
  const fallbackTarget = buildCurrentHomeResumeTarget() || home.endpointId;
  const room = String(home.room || '').trim() || 'lobby';
  const delayMs = state.reentryFibCurr;

  stopHomeEventPolling();
  logger.log('reconnect', `reentry requested (${reason || 'unknown'}) delay=${delayMs}ms endpoint=${home.endpointId.slice(0, 8)}...`);

  if (!state.pollErrorShown) {
    appendSystemUi(
      'Connection lost. Attempting re-entry...',
      'Mistet forbindelse. Forsøker re-entry...'
    );
    state.pollErrorShown = true;
  }

  const work = delay(delayMs)
    .then(() => enterHome(fallbackTarget, room, { silent: true }))
    .then(() => {
      state.reentryFibPrev = 3000;
      state.reentryFibCurr = 3000;
      state.pingFibPrev = PING_FIB_START;
      state.pingFibCurr = PING_FIB_START;
      state.consecutivePollFailures = 0;
      state.pollErrorShown = false;
      logger.log('reconnect', 're-entry complete');
      appendSystemUi('Re-entry complete.', 'Re-entry fullført.');
    })
    .catch((err) => {
      const next = Math.min(state.reentryFibPrev + state.reentryFibCurr, 30000);
      state.reentryFibPrev = state.reentryFibCurr;
      state.reentryFibCurr = next;
      logger.log('reconnect', `re-entry failed (next delay=${next}ms): ${err instanceof Error ? err.message : String(err)}`);
      startHomeEventPolling();
    })
    .finally(() => {
      if (state.reentryInProgress === work) {
        state.reentryInProgress = null;
      }
    });

  state.reentryInProgress = work;
  return work;
}

function startHomeEventPolling() {
  stopHomeEventPolling();

  state.avatarPingTimer = setInterval(() => {
    sendCurrentHomePresencePing().catch((error) => {
      logger.log('presence.ping', `failed: ${error instanceof Error ? error.message : String(error)}`);
    });
  }, state.worldPingIntervalMs ?? AVATAR_PING_INTERVAL_MS);

  state.roomPollTimer = setInterval(() => {
    pollDirectInbox().catch((error) => {
      logger.log('inbox.poll', `non-fatal inbox poll failure: ${error instanceof Error ? error.message : String(error)}`);
    });
  }, ROOM_POLL_INTERVAL_MS);
}

const { updateIdentityLine: updateIdentityLineFromFlow } = createIdentityLineFlow({ updateLocationContext });
updateIdentityLineImpl = updateIdentityLineFromFlow;

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
        currentActorFragment(),
        state.currentHome.room,
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

function setUiLanguage(value) {
  const normalized = normalizeUiLang(value) || uiLangFromLanguage('en', DEFAULT_UI_LANG);
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

function updateRoomHeading(title, description) {
  const headingEl = byId('room-heading');
  const descriptionEl = byId('room-description');
  if (headingEl) {
    headingEl.textContent = String(title || '').trim() || 'Welcome';
  }
  if (descriptionEl) {
    descriptionEl.textContent = String(description || '').trim();
  }
}

const identityStore = createIdentityStore({
  storagePrefix: STORAGE_PREFIX,
  legacy: {
    aliasKey: LEGACY_ALIAS_KEY,
    bundleKey: LEGACY_BUNDLE_KEY,
    recoveryPhraseKey: 'ma.identity.v2.recoveryPhrase'
  },
  isValidAliasName
});

const aliasFlow = createAliasFlow({
  state,
  identityStore,
  aliasBookKey: ALIAS_BOOK_KEY,
  tabAliasKey: TAB_ALIAS_KEY,
  lastAliasKey: LAST_ALIAS_KEY,
  aliasNormalizeEndpointId: alias_normalize_endpoint_id,
  aliasFindDidByEndpoint: alias_find_did_by_endpoint,
  aliasFindAliasForAddress: alias_find_alias_for_address,
  aliasResolveInput: alias_resolve_input,
  aliasHumanizeIdentifier: alias_humanize_identifier,
  aliasHumanizeText: alias_humanize_text,
  roomDidCacheTtlMs: ROOM_DID_CACHE_TTL_MS,
});
Object.assign(aliasFlowBridge, {
  saveAliasBook: aliasFlow.saveAliasBook,
  loadAliasBook: aliasFlow.loadAliasBook,
  setActiveAlias: aliasFlow.setActiveAlias,
  resolveInitialAlias: aliasFlow.resolveInitialAlias,
  loadAliasDraft: aliasFlow.loadAliasDraft,
  roomDidLookupCacheKey: aliasFlow.roomDidLookupCacheKey,
  getCachedRoomDidLookup: aliasFlow.getCachedRoomDidLookup,
  cacheRoomDidLookup: aliasFlow.cacheRoomDidLookup,
  dropCachedRoomDidLookup: aliasFlow.dropCachedRoomDidLookup,
  normalizeEndpointId: aliasFlow.normalizeEndpointId,
  findDidByEndpoint: aliasFlow.findDidByEndpoint,
  findAliasForAddress: aliasFlow.findAliasForAddress,
  resolveAliasInput: aliasFlow.resolveAliasInput,
  humanizeIdentifier: aliasFlow.humanizeIdentifier,
  humanizeText: aliasFlow.humanizeText,
});

const {
  primeDidLookupCacheFromWorldMessage,
  primeDidLookupCacheFromRoomObjectDids,
} = createDidRuntimeHelpers({
  state,
  didRoot,
  resolveAliasInput,
  findDidByEndpoint,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
});

const { parseDot, parseLocalCommand } = createDotCommands({
  state,
  appendSystemUi,
  appendMessage,
  uiText,
  humanizeIdentifier,
  isPrintableAliasLabel,
  saveAliasBook,
  setDebugMode,
  setLogEnabled,
  setLogLevel,
  setDialogIdStyle,
  setAliasRewriteEnabled,
  setMessageTtl,
  getMessageTtl,
  setTemporaryMessageTtlOverride,
  clearTemporaryMessageTtlOverride,
  getTemporaryMessageTtlOverride,
  setBatchTimeoutSeconds,
  setBatchRetryCount,
  runBatchCommands,
  batchStatusLine,
  onAliasBookChanged: refreshDialogAndPresenceFormatting,
  didRoot,
  onDotEdit,
  onDotEval,
  onDotInspect,
  resolveCommandTargetDidOrToken,
  lookupDidInCurrentRoom,
  sendWorldCommandQuery,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  refillCommandInputWithActiveTarget,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
  pollDirectInbox,
  pollCurrentHomeEvents,
  prepareIdentityDocumentForSend,
  publishIdentityToWorldDid,
  sendWhisperToDid,
  sendMessageToDid,
  runSmokeTest,
});

async function prepareIdentityDocumentForSend() {
  if (!state.passphrase || !state.encryptedBundle) {
    return;
  }

  if (!state.inboxEndpointId) {
    state.inboxEndpointId = await start_inbox_listener(state.passphrase, state.encryptedBundle);
  }

  if (state.inboxEndpointId) {
    const withTransports = JSON.parse(
      set_bundle_transports(state.passphrase, state.encryptedBundle, state.inboxEndpointId)
    );
    state.identity = withTransports;
    state.encryptedBundle = withTransports.encrypted_bundle;
  }

  const updated = JSON.parse(
    set_bundle_updated_for_send(state.passphrase, state.encryptedBundle)
  );
  state.identity = updated;
  state.encryptedBundle = updated.encrypted_bundle;

  const bundleEl = byId('bundle-text');
  if (bundleEl) {
    bundleEl.value = updated.encrypted_bundle;
  }
  if (isValidAliasName(state.aliasName || '')) {
    saveIdentityRecord(state.aliasName, updated.encrypted_bundle);
  }
}

async function publishIdentityToWorldDid(worldDid) {
  if (!state.passphrase || !state.encryptedBundle) {
    throw new Error('Identity is locked. Unlock it first.');
  }

  await prepareIdentityDocumentForSend();

  const endpointId = await withTimeout(
    resolveWorldEndpointForDid(worldDid),
    5000,
    `endpoint resolve timed out for ${worldDid}`
  );

  const relayHint = await lookupWorldRelayHint(endpointId).catch(() => '');
  const fragment = currentActorFragment();

  const raw = await withTimeout(
    publish_did_document_via_world_ipfs(
      endpointId,
      relayHint || '',
      state.passphrase,
      state.encryptedBundle,
      fragment,
      '',
      fragment
    ),
    20000,
    'ma/ipfs/1 publish request timed out'
  );

  return JSON.parse(String(raw || '{}'));
}

function saveIdentityRecord(aliasName, encryptedBundle) {
  identityStore.saveIdentityRecord(aliasName, encryptedBundle, state.languageOrder);
}

function resolveIdentityRecord(aliasName) {
  return identityStore.resolveIdentityRecord(aliasName);
}

function scrubStoredRecoveryPhrases() {
  identityStore.scrubStoredRecoveryPhrases();
}

const identityFlow = createIdentityFlow({
  byId,
  state,
  isValidAliasName,
  saveIdentityRecord,
  setSetupStatus,
  setActiveAlias,
  getApiBase,
  normalizeIpfsGatewayBase,
  apiStorageKey: API_KEY,
  createIdentity: create_identity,
  unlockIdentity: unlock_identity,
  ensureBundleIrohSecret: ensure_bundle_iroh_secret,
  setBundleLanguage: set_bundle_language,
  normalizeLanguageOrder: (value) => normalizeLanguageOrderValue(value, DEFAULT_LANGUAGE_ORDER),
  generateBip39Phrase: generate_bip39_phrase,
  normalizeBip39Phrase: normalize_bip39_phrase,
  defaultLanguageOrder: DEFAULT_LANGUAGE_ORDER,
  defaultUiLang: DEFAULT_UI_LANG,
  setUiLanguage,
  setCurrentPublishInfo,
  showChat,
  restoreActiveHomeAfterUnlock,
  appendMessage,
  saveActiveHomeSnapshot,
  stopHomeEventPolling,
  disconnectWorld: disconnect_world,
  clearRoomPresence,
  showSetup,
  showLockOverlay: showLockOverlayTracked,
});

const {
  normalizeLanguageOrder,
  applyBundleLanguagePreference,
  onCreateIdentity,
  onUnlockIdentity,
  onNewPhrase,
  lockSession,
} = identityFlow;

const aliasDraftOptionsBase = {
  byId,
  onNewPhrase,
  normalizeLanguageOrder,
  setUiLanguage,
  defaultLanguageOrder: DEFAULT_LANGUAGE_ORDER,
  defaultUiLang: DEFAULT_UI_LANG,
};

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

async function checkGateway() {
  const configuredBase = getApiBase();
  setGatewayStatus(`Checking IPFS gateway (${configuredBase})...`, 'working');

  const applyGatewaySelection = (base) => {
    const normalized = normalizeIpfsGatewayBase(base);
    const input = byId('gateway-api');
    if (input) {
      input.value = normalized;
    }
    try {
      localStorage.setItem(API_KEY, normalized);
    } catch (_) {}
    return normalized;
  };

  const probeGateway = async (base) => {
    const normalized = normalizeIpfsGatewayBase(base);
    const probeUrl = `${normalized}/ipfs/${KNOWN_IPFS_HELLO_WORLD_CID}`;
    try {
      const response = await fetch(probeUrl, {
        method: 'GET',
        mode: 'no-cors',
        cache: 'no-store',
      });
      return response.type === 'opaque' || response.ok || response.status < 500;
    } catch {
      return false;
    }
  };

  if (await probeGateway(configuredBase)) {
    const activeBase = applyGatewaySelection(configuredBase);
    setGatewayStatus(`Gateway reachable (${activeBase}).`, 'ok');
    setSetupActionsEnabled(true);
    setGatewayInstallNoteVisible(false);
    setSetupStatus('IPFS gateway reachable. You can create or unlock an identity bundle.');
    return [];
  }

  const fallbacks = IPFS_GATEWAY_FALLBACKS
    .map((entry) => normalizeIpfsGatewayBase(entry))
    .filter((entry, idx, list) => list.indexOf(entry) === idx && entry !== configuredBase);

  for (const fallback of fallbacks) {
    if (await probeGateway(fallback)) {
      const activeBase = applyGatewaySelection(fallback);
      setGatewayStatus(`Configured gateway failed. Switched to ${activeBase}.`, 'ok');
      setSetupActionsEnabled(true);
      setGatewayInstallNoteVisible(true, 'gateway-fallback');
      setSetupStatus(`Configured gateway is not reachable. Switched to working fallback: ${activeBase}`);
      return [];
    }
  }

  setGatewayStatus('No tested gateway reachable from browser.', 'error');
  setSetupActionsEnabled(false);
  setGatewayInstallNoteVisible(true, 'gateway-fallback');
  setSetupStatus('Could not reach localhost gateway or public fallbacks. Check network/local node, then try again.');
  throw new Error('IPFS gateway connectivity test failed');
}

async function fetchDidDocumentJsonByDid(did, options) {
  if (!didDocFlow) {
    throw new Error('DID document flow is not initialized yet.');
  }
  return await didDocFlow.fetchDidDocumentJsonByDid(did, options);
}

async function resolveWorldEndpointForDid(did) {
  const targetRoot = didRoot(String(did || '').trim());
  if (!isMaDid(targetRoot)) {
    throw new Error(`Expected did:ma root, got: ${did}`);
  }

  const targetDocJson = await withTimeout(
    fetchDidDocumentJsonByDid(targetRoot, {
      forceRefresh: true,
      timeoutMs: 3500,
    }),
    4500,
    `gateway DID resolve timed out for ${targetRoot}`
  );

  const targetDoc = parseDidDocumentUtil(targetDocJson);
  if (!targetDoc || typeof targetDoc !== 'object') {
    throw new Error(`Resolved DID document is invalid JSON for ${targetRoot}.`);
  }

  return await withTimeout(
    resolveEndpointWithTypePolicy({
      targetRoot,
      targetDoc,
      fetchDidDocumentJsonByDid,
      didRoot,
      parseDidDocument: parseDidDocumentUtil,
      extractWorldEndpointFromDidDoc,
    }),
    4000,
    `endpoint extraction timed out for ${targetRoot}`
  );
}

async function autoFollowEnterDirective(message) {
  if (!didDocFlow) {
    throw new Error('DID document flow is not initialized yet.');
  }
  return await didDocFlow.autoFollowEnterDirective(message);
}

function syncBundleTransportsFromEndpoint(endpointId) {
  const inboxEndpointId = String(endpointId || '').trim();
  if (!inboxEndpointId || !state.passphrase || !state.encryptedBundle) {
    return;
  }

  try {
    const updated = JSON.parse(
      set_bundle_transports(state.passphrase, state.encryptedBundle, inboxEndpointId)
    );
    state.identity = updated;
    state.encryptedBundle = updated.encrypted_bundle;
    const bundleEl = byId('bundle-text');
    if (bundleEl) {
      bundleEl.value = updated.encrypted_bundle;
    }
    if (isValidAliasName(state.aliasName || '')) {
      saveIdentityRecord(state.aliasName, updated.encrypted_bundle);
    }
  } catch (error) {
    logger.log('did.transports', `failed to sync ma.transports: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function refillCommandInputWithActiveTarget() {
  const alias = String(state.activeObjectTargetAlias || '').trim().replace(/^@+/, '');
  const inputEl = byId('command-input');
  if (!inputEl) {
    return;
  }
  if (!alias) {
    inputEl.value = '';
    return;
  }
  inputEl.value = `@${alias} `;
  inputEl.setSelectionRange(inputEl.value.length, inputEl.value.length);
}

function normalizeUseRequirement(value) {
  const normalized = String(value || 'none').trim().toLowerCase();
  return normalized === 'held' ? 'held' : 'none';
}

function setActiveObjectTarget(aliasOrDid, explicitDid = '', requirement = 'none') {
  let alias = String(aliasOrDid || '').trim();
  let did = String(explicitDid || '').trim();

  if (!did && isMaDid(alias)) {
    did = alias;
    alias = '';
  }

  if (!did && alias) {
    const resolved = String(resolveAliasInput(alias) || '').trim();
    const mappedDid = String(findDidByEndpoint(resolved) || '').trim();
    if (isMaDid(mappedDid)) {
      did = mappedDid;
    } else if (isMaDid(resolved)) {
      did = resolved;
    }
  }

  alias = alias.replace(/^@+/, '');

  state.activeObjectTargetAlias = alias;
  state.activeObjectTargetDid = isMaDid(did) ? did : '';
  state.activeObjectTargetRequirement = normalizeUseRequirement(requirement);
  updateLocationContext();
}

function clearActiveObjectTarget(expectedAlias = '') {
  const currentAlias = String(state.activeObjectTargetAlias || '').trim().replace(/^@+/, '');
  const normalizedExpected = String(expectedAlias || '').trim().replace(/^@+/, '');
  if (normalizedExpected) {
    if (currentAlias && currentAlias !== normalizedExpected) {
      return;
    }
  }

  state.activeObjectTargetAlias = '';
  state.activeObjectTargetDid = '';
  state.activeObjectTargetRequirement = 'none';
  updateLocationContext();
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
  const target = isMaDid(did) ? did : alias;
  const normalizedTarget = String(target || '').trim().replace(/^@+/, '');
  if (!normalizedTarget) return String(text || '');
  return `@${normalizedTarget} ${String(text || '').trim()}`;
}

async function ensureHeldRequirementSatisfied(alias, objectDid) {
  const normalizedDid = String(objectDid || '').trim();
  if (!isMaDid(normalizedDid)) return;
  const response = await sendWorldCommandQuery(`@${normalizedDid} show`);
  const kv = parseKeyValuePairs(response);
  const holder = String(kv.holder || '').trim();
  const currentDid = String(state.identity?.did || '').trim();
  const aliasLabel = String(alias || '').trim() || '@object';
  if (!holder || holder === '(none)' || !currentDid || holder !== currentDid) {
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

const { applyWorldResponse } = createWorldResponseFlow({
  state,
  saveLastRoom,
  updateIdentityLine,
  updateRoomHeading: (...args) => updateRoomHeading(...args),
  syncSpecialAliases: syncSpecialAliasesFromCurrentHome,
  clearActiveObjectTarget,
  clearRoomPresence,
  trackRoomPresence,
  saveActiveHomeSnapshot,
  toSequenceNumber,
  primeDidLookupCacheFromRoomObjectDids,
  primeDidLookupCacheFromWorldMessage,
  appendMessage,
  autoFollowEnterDirective,
  refillCommandInputWithActiveTarget,
});

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
  let announcedConnectivity = false;
  logger.log('connect.world', `starting connect sequence for endpoint=${endpointId.slice(0, 8)}... actor=${actorName} room=${room}`);

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const attemptStart = Date.now();
    logger.log(`connect.attempt.${attempt}`, `starting attempt`);
    
    try {
      // Phase 1: Relay discovery and connection
      logger.log(`connect.attempt.${attempt}`, `phase 1/2: relay discovery and connect`);
      const relayHint = await lookupWorldRelayHint(endpointId);
      
      if (relayHint) {
        logger.log(`connect.attempt.${attempt}`, `using relay hint: ${relayHint}`);
      } else {
        logger.log(`connect.attempt.${attempt}`, `no relay hint found, falling back to discovery-only`);
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
      logger.log(`connect.attempt.${attempt}`, `connected in ${connectElapsed}ms`);
      if (!announcedConnectivity) {
        appendMessage('system', `iroh node discovered at ${humanizeIdentifier(endpointId)}. Requesting avatar/session state...`);
        announcedConnectivity = true;
      }

      // Phase 2: World ping (initial avatar/session sync)
      logger.log(`connect.attempt.${attempt}`, `phase 2/2: sending ping request`);
      const requestStart = Date.now();
      const response = await withTimeout(
        ping_world(endpointId, state.passphrase, state.encryptedBundle, actorName, room),
        12000,
        'ping request timed out'
      );
      const requestElapsed = Date.now() - requestStart;
      logger.log(`connect.attempt.${attempt}`, `ping request succeeded in ${requestElapsed}ms`);
      
      const result = JSON.parse(response);
      logger.log(`connect.attempt.${attempt}`, `response: ok=${result.ok} room=${result.room} latest_seq=${result.latest_event_sequence || 0} endpoint=${result.endpoint_id?.slice(0, 8)}...`);
      logger.log(`connect.world`, `success after ${Date.now() - attemptStart}ms total on attempt ${attempt}/${maxAttempts}`);
      
      return response;
    } catch (error) {
      lastError = error;
      const message = error instanceof Error ? error.message : String(error);
      const elapsedTotal = Date.now() - attemptStart;
      const isTimeout = message.includes('timed out');
      const isConnectionLost = message.includes('connection lost');
      const isRetryable = isTimeout || isConnectionLost;
      
      logger.log(`connect.attempt.${attempt}`, `failed after ${elapsedTotal}ms: ${message} (retryable=${isRetryable})`);

      if (!isRetryable || attempt === maxAttempts) {
        logger.log('connect.world', `giving up after attempt ${attempt}/${maxAttempts}: ${message}`);
        throw error;
      }

      const backoffMs = 1500 * attempt;
      appendMessage(
        'system',
        `iroh attempt ${attempt}/${maxAttempts} failed (${message}). Retrying...`
      );
      logger.log(`connect.attempt.${attempt}`, `waiting ${backoffMs}ms before attempt ${attempt + 1}`);
      await delay(backoffMs);
    }
  }

  logger.log('connect.world', `failed: all ${maxAttempts} attempts exhausted`);
  throw lastError || new Error('iroh connect failed');
}

async function enterHome(target, preferredRoom = null) {
  const options = (typeof arguments[2] === 'object' && arguments[2] !== null) ? arguments[2] : {};
  const silent = Boolean(options.silent);
  if (!state.identity) {
    throw new Error('Load or create an identity before entering a home.');
  }

  if (state.didPublishPromise) {
    appendMessage('system', 'DID publish is still running in background. Continuing world connect attempt.');
  } else if (state.didPublishError) {
    appendMessage('system', `DID publish is pending/retrying in background (${state.didPublishError}). Continuing world connect attempt.`);
  }

  const alias = String(target || '').trim();
  if (!alias) {
    throw new Error('go requires a target (did:ma:<world>#<room> or alias).');
  }

  const hashIdx = alias.indexOf('#');
  const aliasBaseToken = hashIdx === -1 ? alias : alias.slice(0, hashIdx);
  const aliasRoomFragment = hashIdx === -1 ? '' : alias.slice(hashIdx + 1);
  const aliasAltToken = aliasBaseToken.startsWith('@') ? aliasBaseToken.slice(1) : `@${aliasBaseToken}`;

  let resolvedBase = String(resolveAliasInput(aliasBaseToken) || '').trim();
  if (!resolvedBase || resolvedBase === aliasBaseToken) {
    const altResolved = String(resolveAliasInput(aliasAltToken) || '').trim();
    if (altResolved && altResolved !== aliasAltToken) {
      resolvedBase = altResolved;
    }
  }
  if (!resolvedBase) {
    resolvedBase = aliasBaseToken;
  }

  let resolvedInput = resolvedBase;
  if (aliasRoomFragment) {
    const room = String(aliasRoomFragment || '').trim();
    if (isMaDid(resolvedBase)) {
      resolvedInput = didWithFragment(resolvedBase, room);
    }
  }

  const resolvedDid = isMaDidTarget(String(resolvedInput)) ? String(resolvedInput).trim() : '';
  const resolvedDidFragment = aliasRoomFragment || (String(resolvedInput).includes('#') ? String(resolvedInput).split('#')[1] : '');
  let endpointId = '';
  if (!isMaDid(String(resolvedInput))) {
    endpointId = normalizeIrohAddress(resolvedBase);
  }

  if (resolvedDid) {
    if (!silent) {
      appendMessage('system', `Resolving DID target ${resolvedDid}...`);
    }

    let targetDocJson = '';
    let localResolveError = '';
    try {
      targetDocJson = await withTimeout(
        fetchDidDocumentJsonByDid(resolvedDid, {
          localOnly: true,
          timeoutMs: 2500,
        }),
        3000,
        `local DID resolve timed out for ${resolvedDid}`
      );
    } catch (localError) {
      localResolveError = localError instanceof Error ? localError.message : String(localError);
      targetDocJson = await withTimeout(
        fetchDidDocumentJsonByDid(resolvedDid, {
          forceRefresh: true,
          timeoutMs: 3500,
        }),
        4500,
        `gateway DID resolve timed out for ${resolvedDid}`
      );
    }

    const targetDoc = parseDidDocumentUtil(targetDocJson);
    if (!targetDoc || typeof targetDoc !== 'object') {
      throw new Error(`Resolved DID document is invalid JSON for ${resolvedDid}.`);
    }

    const rawPingInterval = targetDoc?.ma?.pingIntervalSecs;
    if (typeof rawPingInterval === 'number' && rawPingInterval > 0) {
      const rawPingIntervalMs = rawPingInterval * 1000;
      state.worldPingIntervalMs = Math.min(
        MAX_WORLD_PING_INTERVAL_MS,
        Math.max(MIN_WORLD_PING_INTERVAL_MS, rawPingIntervalMs)
      );
    } else {
      state.worldPingIntervalMs = null;
    }

    endpointId = await withTimeout(
      resolveEndpointWithTypePolicy({
        targetRoot: resolvedDid,
        targetDoc,
        fetchDidDocumentJsonByDid,
        didRoot,
        parseDidDocument: parseDidDocumentUtil,
        extractWorldEndpointFromDidDoc,
      }),
      4000,
      `endpoint extraction timed out for ${resolvedDid}`
    );

    if (!endpointId && localResolveError) {
      appendMessage('system', `DID resolve note: local gateway lookup failed first (${localResolveError}).`);
    }
  }

  const effectivePreferredRoom = String(preferredRoom || '').trim() || resolvedDidFragment;
  logger.log('connect.home', `alias=${alias} resolved=${resolvedInput} endpoint=${endpointId.slice(0, 8)}...`);
  
  if (!isLikelyIrohAddress(endpointId)) {
    if (resolvedDid) {
      throw new Error(
        `DID ${resolvedDid} did not resolve to a valid iroh endpoint. Ensure its DID document has ma.transports, ma.currentInbox, or ma.presenceHint with /ma-iroh/<endpoint-id>/... or /iroh/<endpoint-id>.`
      );
    }
    throw new Error(
      `Alias ${alias} is not a valid endpoint id (expected 64 hex chars, got ${endpointId.length}).`
    );
  }

  if (!silent) {
    appendMessage('system', `Connecting to ${endpointId}...`);
  }

  const requestedRoom = effectivePreferredRoom;
  const savedRoom = requestedRoom || loadLastRoom(endpointId);
  let result;
  try {
    if (savedRoom && savedRoom !== 'lobby') {
      try {
        result = JSON.parse(await enterWorldWithRetry(endpointId, currentActorFragment(), savedRoom));
        if (!result.ok) {
          logger.log('connect.home', `last room '${savedRoom}' denied (${result.message}), falling back to lobby`);
          result = JSON.parse(await enterWorldWithRetry(endpointId, currentActorFragment(), 'lobby'));
        }
      } catch (_) {
        result = JSON.parse(await enterWorldWithRetry(endpointId, currentActorFragment(), 'lobby'));
      }
    } else {
      result = JSON.parse(await enterWorldWithRetry(endpointId, currentActorFragment(), 'lobby'));
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw error;
  }
  logger.log('connect.home', `result ok=${result.ok} room=${result.room} endpoint=${result.endpoint_id?.slice(0, 8)}... latest_seq=${result.latest_event_sequence || 0}`);

  if (!result.ok) {
    logger.warn('connect.home', `world enter returned ok=false: ${String(result.message || '(empty message)')}`);
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
    worldDid: result.world_did || '',
    lastEventSequence: toSequenceNumber(result.latest_event_sequence || 0),
    handle: result.handle || state.aliasName
  };
  syncSpecialAliasesFromCurrentHome();
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

  const inboxEndpointId = await ensureInboxListener();
  syncBundleTransportsFromEndpoint(inboxEndpointId);
  updateIdentityLine();

  startHomeEventPolling();

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

worldDispatchFlow = createWorldDispatchFlow({
  state,
  appendMessage,
  enterHome,
  resolveWorldEndpointForDid,
  isLikelyIrohAddress,
  normalizeIrohAddress,
  parseDot,
  parseLocalCommand,
  resolveCommandTargetDidOrToken,
  logger,
  sendWorldChat: send_world_chat,
  sendWorldChatWithTtl: send_world_chat_with_ttl,
  sendWorldCmd: send_world_cmd,
  sendWorldCmdWithTtl: send_world_cmd_with_ttl,
  getMessageTtl,
  pollCurrentHomeEvents,
  appendAmbientProseAfterSpeech,
  renderLocalBroadcastMessage,
  applyWorldResponse,
  tryHandleDidTargetMetaPoll,
  sendWhisperToDid,
  sendMessageToDid,
  isNotRegisteredInRoomMessage,
  performTransparentReentry,
});

async function sendCurrentWorldMessage(text) {
  if (!worldDispatchFlow) {
    throw new Error('World dispatch is not initialized yet.');
  }
  return await worldDispatchFlow.sendCurrentWorldMessage(text, arguments[1] || {});
}

function onCommandSubmit(event) {
  event.preventDefault();
  const inputEl = byId('command-input');
  const text = inputEl.value.trim();
  if (!text) return;

  enqueueCommandText(text);
}

function enqueueCommandText(text) {
  const commandText = String(text || '').trim();
  if (!commandText) {
    return;
  }

  recordCommandIo('out', commandText);

  // Readline-like history: keep unique latest entry and reset cursor.
  state.commandHistory.push(commandText);
  state.historyIndex = -1;
  state.historyDraft = '';

  const queuedText = commandText;
  state.commandQueue = state.commandQueue
    .catch(() => {})
    .then(async () => {
      if (queuedText.startsWith('.')) {
        parseDot(queuedText);
        return;
      }

      if (parseLocalCommand(queuedText)) {
        return;
      }

      if (state.batch.collecting && !state.batch.running) {
        queueBatchCommand(queuedText);
        appendMessage('system', `Batch +1 (${state.batch.commands.length}): ${queuedText}`);
        return;
      }

      await sendWithActiveTargetRequirementsIfNeeded(queuedText);
    })
    .catch((err) => {
      const message = err instanceof Error ? err.message : String(err);
      logger.error('command.send', message);
      appendMessage('system', `Send failed: ${message}`);
      appendMaRpc({
        status: 'failed',
        code: classifyRpcCode(message, 'failed'),
        content: message,
        retry: false,
      });
      if (state.activeObjectTargetAlias && isActiveTargetGoneMessage(message)) {
        const alias = String(state.activeObjectTargetAlias || '').trim();
        dropCachedRoomDidLookup(alias);
        clearActiveObjectTarget(alias);
        refillCommandInputWithActiveTarget();
        reportActiveTargetVanished(alias);
      }
    });

  refillCommandInputWithActiveTarget();
}

function onCommandPaste(event) {
  const pasted = String(event.clipboardData?.getData('text') || '');
  if (!pasted.includes('\n') && !pasted.includes('\r')) {
    return;
  }

  event.preventDefault();
  const lines = pasted
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  if (lines.length === 0) {
    return;
  }

  for (const line of lines) {
    enqueueCommandText(line);
  }

  if (lines.length > 1) {
    appendMessage('system', `Queued ${lines.length} pasted commands.`);
  }
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

  const rawSavedApi = localStorage.getItem(API_KEY);
  const savedApi = normalizeIpfsGatewayBase(rawSavedApi);
  let savedAlias = resolveInitialAlias();
  if (!savedAlias) {
    const fallbackRecord = identityStore.findAnyIdentityRecord();
    if (fallbackRecord?.aliasName) {
      savedAlias = fallbackRecord.aliasName;
    }
  }

  if (savedApi) {
    byId('gateway-api').value = savedApi;
    try {
      localStorage.setItem(API_KEY, savedApi);
    } catch (_) {}
  }

  if (savedAlias) {
    byId('alias-name').value = savedAlias;
    loadAliasDraft(savedAlias, { ...aliasDraftOptionsBase });
  } else {
    byId('bundle-text').value = '';
    const languageInput = byId('language-order');
    if (languageInput) {
      languageInput.value = DEFAULT_LANGUAGE_ORDER;
    }
    state.languageOrder = DEFAULT_LANGUAGE_ORDER;
    setUiLanguage(DEFAULT_UI_LANG);
    onNewPhrase();
  }

  state.aliasBook = loadAliasBook();
  state.debug = readStoredDebugFlag();
  state.messageTtl.chat = readStoredMessageTtl(MSG_CHAT_TTL_KEY, DEFAULT_CHAT_TTL_SECONDS);
  state.messageTtl.cmd = readStoredMessageTtl(MSG_CMD_TTL_KEY, DEFAULT_CMD_TTL_SECONDS);
  state.messageTtl.whisper = readStoredMessageTtl(MSG_WHISPER_TTL_KEY, DEFAULT_WHISPER_TTL_SECONDS);
  persistMessageTtl();
  setDialogIdStyle(localStorage.getItem(DIALOG_ID_STYLE_KEY) || 'alias');
  setAliasRewriteEnabled(localStorage.getItem(ALIAS_REWRITE_ENABLED_KEY) !== 'false');
  setAliasRenderEnabled(localStorage.getItem(ALIAS_RENDER_ENABLED_KEY) !== 'false');
  setCurrentPublishInfo();
}

function shouldAutoCheckIpfsRpc() {
  return isLocalhostLikeHost(window.location.hostname);
}

async function main() {
  await init();
  await updateAppVersionFooter();
  applyProperName();
  restoreSavedValues();
  state.logEnabled = readStoredLogEnabledFlag();
  state.logLevel = readStoredLogLevel();
  hideLockOverlay();

  if (shouldAutoCheckIpfsRpc()) {
    checkGateway().catch(() => {});
  } else {
    setGatewayStatus('not checked (remote origin)', 'idle');
    setSetupStatus('Gateway is not auto-checked on this origin. Use Test IPFS Connection.');
  }

  byId('btn-gateway-check').addEventListener('click', () => {
    checkGateway().catch(() => {});
  });
  byId('btn-create').addEventListener('click', onCreateIdentity);
  byId('btn-unlock').addEventListener('click', onUnlockIdentity);
  byId('btn-new-phrase').addEventListener('click', onNewPhrase);
  byId('btn-alias').addEventListener('click', toggleAliasRenderEnabled);
  byId('btn-log').addEventListener('click', openMatrixModal);
  byId('btn-export').addEventListener('click', exportBundle);
  byId('btn-help').addEventListener('click', openHelpModal);
  byId('help-close').addEventListener('click', closeHelpModal);
  byId('help-modal').addEventListener('click', (event) => {
    if (event.target === byId('help-modal')) {
      closeHelpModal();
    }
  });
  byId('matrix-close').addEventListener('click', closeMatrixModal);
  byId('matrix-refresh').addEventListener('click', renderMatrixView);
  byId('matrix-modal').addEventListener('click', (event) => {
    if (event.target === byId('matrix-modal')) {
      closeMatrixModal();
    }
  });
  byId('btn-lock').addEventListener('click', lockSession);
  byId('lock-overlay').addEventListener('click', onLockOverlayClick);
  byId('lock-overlay').addEventListener('keydown', onLockOverlayKeydown);
  let aliasDraftTimer = null;
  byId('alias-name').addEventListener('input', (event) => {
    if (aliasDraftTimer) {
      clearTimeout(aliasDraftTimer);
    }
    const value = event.target.value;
    aliasDraftTimer = setTimeout(() => {
      loadAliasDraft(value, { ...aliasDraftOptionsBase, persistActive: false });
    }, 120);
  });
  byId('alias-name').addEventListener('change', (event) => {
    loadAliasDraft(event.target.value, { ...aliasDraftOptionsBase, persistActive: true });
  });
  const languageInput = byId('language-order');
  if (languageInput) {
    languageInput.addEventListener('change', (event) => {
      const value = normalizeLanguageOrder(event.target?.value || '');
      event.target.value = value;

      try {
        applyBundleLanguagePreference(value);
      } catch (err) {
        appendMessage('system', `Failed to save language preference: ${err instanceof Error ? err.message : String(err)}`);
      }

      const aliasName = byId('alias-name').value.trim();
      const bundle = byId('bundle-text').value.trim();
      if (isValidAliasName(aliasName)) {
        saveIdentityRecord(aliasName, bundle);
      }
    });
  }
  byId('command-form').addEventListener('submit', onCommandSubmit);
  byId('command-input').addEventListener('keydown', onCommandKeyDown);
  byId('command-input').addEventListener('paste', onCommandPaste);
  byId('yaml-editor-cancel').addEventListener('click', closeEditorModal);
  byId('yaml-editor-reload').addEventListener('click', () => {
    if (!state.editSession) {
      setEditorStatus('No loaded source to reload.', 'error');
      return;
    }
    if (state.editSession.mode === 'script') {
      const currentText = getEditorText();
      const storedText = String(localStorage.getItem(LOCAL_EDIT_SCRIPT_KEY) || '');
      if (currentText !== storedText) {
        const shouldDiscard = window.confirm('Discard unsaved local script changes and reload from local storage?');
        if (!shouldDiscard) {
          setEditorStatus('Reload canceled. Unsaved text kept.', 'working');
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
    loadEditorForTarget(state.editSession.target, false).catch((err) => {
      appendMessage('system', `Reload failed: ${err instanceof Error ? err.message : String(err)}`);
    });
  });
  byId('yaml-editor-save').addEventListener('click', () => {
    saveEditorChanges();
  });
  byId('yaml-editor-close-eval').addEventListener('click', () => {
    closeAndEvalEditorScript().catch((err) => {
      appendMessage('system', `Close and Eval failed: ${err instanceof Error ? err.message : String(err)}`);
    });
  });
  byId('yaml-editor-modal').addEventListener('click', (event) => {
    if (event.target === byId('yaml-editor-modal')) {
      closeEditorModal();
    }
  });
  byId('yaml-editor-modal').addEventListener('keydown', onEditorModalKeyDown);

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
  logger.error('app.main', error instanceof Error ? error.stack || error.message : String(error));
  setSetupStatus(`Fatal error: ${error instanceof Error ? error.message : String(error)}`);
});
