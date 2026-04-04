import init, {
  create_identity,
  unlock_identity,
  ensure_bundle_iroh_secret,
  set_bundle_language,
  set_bundle_transports,
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
  disconnect_world
} from './pkg/ma_actor.js';
import { createInboundDispatcher, createInboxTransport } from './inbox.js';
import { createAliasFlow, isPrintableAliasLabel, isValidAliasName } from './alias.js';
import { createClosetFlow } from './closet.js';
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
  roomLanguageKey,
  sanitizeRoomYamlForEdit,
} from './room.js';
import {
  createDidDocFlow,
  createDidRoot,
  createDidRuntimeHelpers,
  isMaDid,
  isUnconfiguredDidTarget,
  parseDidDocument as parseDidDocumentUtil,
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
const LEGACY_BUNDLE_KEY = 'ma.identity.v2.bundle';
const LAST_ROOM_KEY_PREFIX = `${STORAGE_PREFIX}.lastRoom`;
const LAST_ACTIVE_HOME_KEY_PREFIX = `${STORAGE_PREFIX}.lastActiveHome`;
const BLOCKLIST_KEY_PREFIX = `${STORAGE_PREFIX}.blockedDidRoots`;
const LAST_PUBLISHED_IPNS_KEY = `${STORAGE_PREFIX}.lastPublishedIpns`;
const LAST_PUBLISHED_CID_KEY = `${STORAGE_PREFIX}.lastPublishedCid`;
const LEGACY_ALIAS_KEY = 'ma.identity.v2.alias';
const DEFAULT_UI_LANG = 'en';
const DEFAULT_LANGUAGE_ORDER = 'nb_NO:en_UK';
const KNOWN_IPFS_HELLO_WORLD_CID = 'bafkreidfdrlkeq4m4xnxuyx6iae76fdm4wgl5d4xzsb77ixhyqwumhz244';
const IPFS_GATEWAY_FALLBACKS = [
  'http://localhost:8080',
  'https://ipfs.io',
  'https://dweb.link',
  'https://w3s.link',
];
const LOCAL_EDIT_SCRIPT_KEY = `${STORAGE_PREFIX}.localEditScript`;
const LEGACY_LOCAL_EDIT_SCRIPT_CID_KEY = `${STORAGE_PREFIX}.localEditScriptCid`;

const ROOM_POLL_INTERVAL_MS = 1500;
const DID_DOC_CACHE_TTL_MS = 60_000;

const didRoot = createDidRoot(alias_did_root);

function getApiBase() {
  return normalizeIpfsGatewayBase(byId('gateway-api').value);
}

async function fetchGatewayTextByPath(contentPath) {
  return await fetchGatewayTextByPathRaw(contentPath, {
    getApiBase,
    fallbackBases: IPFS_GATEWAY_FALLBACKS,
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
  commandQueue: Promise.resolve(),
  historyIndex: -1,
  historyDraft: '',
  roomPresence: new Map(),
  activeObjectTargetAlias: '',
  activeObjectTargetDid: '',
  activeObjectTargetRequirement: 'none',
  closetSessionId: '',
  closetSessionDid: '',
  closetEndpointId: '',
  closetLobbySeq: 0,
  closetPendingIpnsPrivateKeyB64: '',
  didPublishPromise: null,
  didPublishError: '',
  transparentReentryPromise: null,
  editSession: null,
  editBusy: false,
  lockOverlayAnimationId: 0,
  lockOverlayStarDrift: 0
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

const RECONNECT_DELAY_MS = 3000;
const ROOM_DID_CACHE_TTL_MS = 30000;

const { saveLastRoom, loadLastRoom } = createRoomStorage({
  state,
  lastRoomKeyPrefix: LAST_ROOM_KEY_PREFIX,
});

function activeHomeKey(identityDid) {
  const rootDid = didRoot(identityDid || '');
  if (!rootDid) return '';
  return `${LAST_ACTIVE_HOME_KEY_PREFIX}.${rootDid}`;
}

function buildCurrentHomeResumeTarget() {
  if (!state.currentHome) {
    return '';
  }

  const roomDid = String(state.currentHome.roomDid || '').trim();
  if (isMaDid(roomDid) && !isUnconfiguredDidTarget(roomDid)) {
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
        .filter((value) => isMaDid(value))
    );
  } catch {
    state.blockedDidRoots = new Set();
  }
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
  if (activeAliasRaw && activeAliasRaw.toLowerCase() === raw.toLowerCase() && isMaDid(activeDid)) {
    cacheRoomDidLookup(raw, activeDid);
    return activeDid;
  }
  if (isBuiltinTargetToken(raw)) {
    return raw;
  }
  if (isMaDid(raw)) {
    return raw;
  }

  const resolvedAlias = String(resolveAliasInput(raw) || '').trim();
  if (isMaDid(resolvedAlias)) {
    return resolvedAlias;
  }

  const mappedDid = state.handleDidMap[raw]
    || state.handleDidMap[resolvedAlias]
    || '';
  if (isMaDid(String(mappedDid))) {
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
      const reply = await sendWorldCommandQuery(`@here set exit-content-b64 ${exitId} ${payload}`);
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
    const reply = await sendWorldCommandQuery(`@here set content-b64 ${payload}`);
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

const { applyPresencePayload } = createRoomPresencePayloadFlow({
  state,
  updateRoomHeading: (...args) => updateRoomHeading(...args),
  trackRoomPresence,
  removeRoomPresence,
  clearRoomPresence,
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

function displayActor(senderDid, senderHandle) {
  const handle = String(senderHandle || '').trim();
  if (handle) {
    return handle.startsWith('@') ? handle : `@${handle}`;
  }

  const did = String(senderDid || '').trim();
  if (did) {
    return humanizeIdentifier(did);
  }

  return '@unknown';
}

function renderLocalBroadcastMessage(text) {
  const payload = String(text || '').trim();
  if (!payload) return;
  const actor = displayActor(
    state.identity?.did,
    state.currentHome?.handle || state.aliasName || '@you'
  );
  appendMessage('world', humanizeText(`${actor}: ${payload}`));
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

const dialogWriter = createDialogWriter({ byId, displayActor });
const { appendMessage } = dialogWriter;

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

const closetFlow = createClosetFlow({
  state,
  byId,
  appendMessage,
  didRoot,
  isValidAliasName,
  saveIdentityRecord,
  updateIdentityLine,
  ensureBundleIrohSecret: ensure_bundle_iroh_secret,
});

const {
  isClosetRequiredMessage,
  isClosetBootstrapFailureMessage,
  normalizeClosetInput,
  renderClosetResponse,
  closetStartSessionForEndpoint,
  closetCommandForCurrentWorld,
} = closetFlow;

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
});

const { sendWhisperToDid } = whisperFlow;

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
        state.aliasName,
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

function resolveCurrentPositionTarget() {
  if (!state.currentHome) {
    return '';
  }

  const roomDid = String(state.currentHome.roomDid || '').trim();
  if (isMaDid(roomDid) && !isUnconfiguredDidTarget(roomDid)) {
    return roomDid;
  }

  const worldDid = didRoot(findDidByEndpoint(state.currentHome.endpointId) || '');
  if (worldDid) {
    const room = String(state.currentHome.room || 'lobby').trim() || 'lobby';
    return `${worldDid}#${room}`;
  }

  return '';
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
  const lang = String(languageValue || '').trim().replace(/_/g, '-').toLowerCase();
  if (lang.startsWith('nb') || lang.startsWith('nn') || lang === 'no') {
    return 'nb';
  }
  if (lang.startsWith('en')) {
    return 'en';
  }
  return DEFAULT_UI_LANG;
}

function setUiLanguage(value) {
  const normalized = normalizeUiLang(value) || uiLangFromLanguage('en');
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
  saveBlockedDidRoots,
  resolveTargetDidRoot,
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
  blocklistKey,
});

const { parseDot } = createDotCommands({
  state,
  appendSystemUi,
  appendMessage,
  uiText,
  humanizeIdentifier,
  isPrintableAliasLabel,
  saveAliasBook,
  resolveCurrentPositionTarget,
  setDebugMode,
  didRoot,
  resolveTargetDidRoot,
  saveBlockedDidRoots,
  onDotEdit,
  onDotEval,
  onDotInspect,
  lookupDidInCurrentRoom,
  sendWorldCommandQuery,
  cacheRoomDidLookup,
  setActiveObjectTarget,
  refillCommandInputWithActiveTarget,
  dropCachedRoomDidLookup,
  clearActiveObjectTarget,
  pollDirectInbox,
  pollCurrentHomeEvents,
  sendWhisperToDid,
  runSmokeTest,
});

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
  generateBip39Phrase: generate_bip39_phrase,
  normalizeBip39Phrase: normalize_bip39_phrase,
  defaultLanguageOrder: DEFAULT_LANGUAGE_ORDER,
  defaultUiLang: DEFAULT_UI_LANG,
  setUiLanguage,
  loadBlockedDidRootsForIdentity,
  setCurrentPublishInfo,
  showChat,
  restoreActiveHomeAfterUnlock,
  appendMessage,
  saveActiveHomeSnapshot,
  stopHomeEventPolling,
  disconnectWorld: disconnect_world,
  clearRoomPresence,
  showSetup,
  showLockOverlay,
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

async function fetchDidDocumentJsonByDid(did) {
  if (!didDocFlow) {
    throw new Error('DID document flow is not initialized yet.');
  }
  return await didDocFlow.fetchDidDocumentJsonByDid(did);
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
  const alias = String(state.activeObjectTargetAlias || '').trim();
  const inputEl = byId('command-input');
  if (!inputEl) {
    return;
  }
  if (!alias) {
    inputEl.value = '';
    return;
  }
  inputEl.value = `${alias} `;
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

  if (alias && !alias.startsWith('@')) {
    alias = `@${alias.replace(/^@+/, '')}`;
  }

  state.activeObjectTargetAlias = alias;
  state.activeObjectTargetDid = isMaDid(did) ? did : '';
  state.activeObjectTargetRequirement = normalizeUseRequirement(requirement);
  updateLocationContext();
}

function clearActiveObjectTarget(expectedAlias = '') {
  const currentAlias = String(state.activeObjectTargetAlias || '').trim();
  const normalizedExpected = String(expectedAlias || '').trim();
  if (normalizedExpected) {
    const normalizedAt = normalizedExpected.startsWith('@')
      ? normalizedExpected
      : `@${normalizedExpected.replace(/^@+/, '')}`;
    if (currentAlias && currentAlias !== normalizedAt) {
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
  return `@${target} ${String(text || '').trim()}`;
}

async function ensureHeldRequirementSatisfied(alias, objectDid) {
  const normalizedDid = String(objectDid || '').trim();
  if (!isMaDid(normalizedDid)) return;
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

const { applyWorldResponse } = createWorldResponseFlow({
  state,
  saveLastRoom,
  updateIdentityLine,
  updateRoomHeading: (...args) => updateRoomHeading(...args),
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
      if (!announcedConnectivity) {
        appendMessage('system', `iroh node discovered at ${humanizeIdentifier(endpointId)}. Requesting avatar/session state...`);
        announcedConnectivity = true;
      }

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

  if (state.didPublishPromise) {
    await state.didPublishPromise;
  }
  if (state.didPublishError) {
    throw new Error(`DID document is not published yet: ${state.didPublishError}`);
  }

  const alias = String(target || '').trim();
  if (!alias) {
    throw new Error('enterHome() requires a target (did:ma:world[#room] or alias).');
  }

  const resolvedInput = resolveAliasInput(alias);
  const resolvedDidRoot = isMaDid(String(resolvedInput)) ? didRoot(resolvedInput) : '';
  const resolvedDidFragment = String(resolvedInput).includes('#') ? String(resolvedInput).split('#')[1] : '';
  let worldDidForBundle = '';
  let endpointId = '';
  if (!isMaDid(String(resolvedInput))) {
    endpointId = normalizeIrohAddress(resolvedInput);
  }

  if (resolvedDidRoot) {
    const targetDocJson = await fetchDidDocumentJsonByDid(resolvedDidRoot);
    const targetDoc = parseDidDocumentUtil(targetDocJson);
    const hintedWorldDid = typeof targetDoc?.ma?.world === 'string'
      ? targetDoc.ma.world
      : '';
    const worldDid = hintedWorldDid ? didRoot(hintedWorldDid) : resolvedDidRoot;
    worldDidForBundle = worldDid;
    const worldDocJson = await fetchDidDocumentJsonByDid(worldDid);
    const worldDoc = parseDidDocumentUtil(worldDocJson);
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
      appendMessage('system', 'No avatar available for this DID yet, or DID publish is not ready. Entering closet onboarding.');
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
      appendMessage('system', result.message || 'No avatar profile is ready yet. Closet onboarding is required first.');
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
  state.closetSessionDid = '';
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

  const inboxEndpointId = await ensureInboxListener();
  syncBundleTransportsFromEndpoint(inboxEndpointId);
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
  normalizeClosetInput,
  closetCommandForCurrentWorld,
  renderClosetResponse,
  enterHome,
  isLikelyIrohAddress,
  normalizeIrohAddress,
  parseDot,
  resolveCommandTargetDidOrToken,
  logger,
  sendWorldChat: send_world_chat,
  sendWorldMessage: send_world_message,
  sendWorldCmd: send_world_cmd,
  pollCurrentHomeEvents,
  appendAmbientProseAfterSpeech,
  renderLocalBroadcastMessage,
  applyWorldResponse,
  tryHandleDidTargetMetaPoll,
  sendWhisperToDid,
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

      await sendWithActiveTargetRequirementsIfNeeded(queuedText);
    })
    .catch((err) => {
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
  byId('btn-export').addEventListener('click', exportBundle);
  byId('btn-lock').addEventListener('click', lockSession);
  byId('lock-overlay').addEventListener('click', hideLockOverlay);
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
  setSetupStatus(`Fatal error: ${error instanceof Error ? error.message : String(error)}`);
});
