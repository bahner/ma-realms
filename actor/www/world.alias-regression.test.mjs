import test from 'node:test';
import assert from 'node:assert/strict';

import { resolveAliasTargetToken, createWorldDispatchFlow } from './world.js';

function makeDispatchFlow(stateOverrides = {}) {
  const state = {
    identity: { did: 'did:ma:testactor#alice' },
    currentHome: {
      room: 'lobby',
      roomDid: 'did:ma:testroom#lobby',
      worldDid: 'did:ma:testworld#world',
    },
    aliasBook: { '@world': 'did:ma:testworld#world', myfriend: 'did:ma:testfriend#bob' },
    ...stateOverrides,
  };
  const flow = createWorldDispatchFlow({
    state,
    appendMessage: () => {},
    enterHome: async () => {},
    resolveWorldEndpointForDid: async () => '',
    isLikelyIrohAddress: () => false,
    normalizeIrohAddress: (v) => v,
    parseDot: () => null,
    parseLocalCommand: () => null,
    resolveCommandTargetDidOrToken: async (t) => t,
    logger: { log: () => {} },
    sendWorldChat: async () => '{}',
    sendWorldChatWithTtl: async () => '{}',
    sendWorldCmd: async () => '{}',
    sendWorldCmdWithTtl: async () => JSON.stringify({ ok: true, message: 'ok' }),
    getMessageTtl: () => 60,
    pollCurrentHomeEvents: async () => {},
    appendAmbientProseAfterSpeech: () => {},
    renderLocalBroadcastMessage: () => {},
    applyWorldResponse: () => {},
    tryHandleDidTargetMetaPoll: async () => {},
    sendWhisperToDid: async () => {},
    isNotRegisteredInRoomMessage: () => false,
    performTransparentReentry: async () => {},
  });
  return { flow, state };
}

test('resolveAliasTargetToken keeps self-referential handle alias stable', () => {
  const aliases = new Map([
    ['panteia', '@panteia'],
    ['@panteia', '@panteia'],
  ]);

  assert.equal(resolveAliasTargetToken('panteia', aliases), '@panteia');
  assert.equal(resolveAliasTargetToken('@panteia', aliases), '@panteia');
});

test('resolveAliasTargetToken follows chain ending in did:ma', () => {
  const aliases = new Map([
    ['hero', '@friend'],
    ['friend', 'did:ma:abc123#friend'],
    ['@friend', 'did:ma:abc123#friend'],
  ]);

  assert.equal(resolveAliasTargetToken('hero', aliases), 'did:ma:abc123#friend');
});

test('resolveAliasTargetToken returns empty string for unknown alias', () => {
  const aliases = new Map([
    ['known', '@known'],
  ]);

  assert.equal(resolveAliasTargetToken('missing', aliases), '');
});

test('placeholder for @@here.owner syntax is handled in runtime rewrite path', () => {
  // Runtime rewrite depends on current room and alias map in createWorldDispatchFlow.
  // Keep a stable placeholder so this file remains the home for alias-regressions.
  assert.equal(true, true);
});

// normalizeInputAliases tests


// .peek is deprecated; use @avatar or @here for introspection

test('normalizeInputAliases: @me expands to actor DID with @ prefix', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('@me.id'), '@did:ma:testactor#alice.id');
});

test('normalizeInputAliases: @here expands to room DID when standalone', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('@here'), '@did:ma:testroom#lobby');
});


// .peek is deprecated; use @here for introspection

test('normalizeInputAliases: @world expands to world DID when standalone', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('@world'), '@did:ma:testworld#world');
});

test('normalizeInputAliases: @world.rooms left unexpanded (handled by normalizeOutgoingAtTarget)', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('@world.rooms'), '@world.rooms');
});

test('normalizeInputAliases: @my.myfriend expands to aliasBook DID', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('@my.myfriend whisper hello'), '@did:ma:testfriend#bob whisper hello');
});

test('normalizeInputAliases: \\@avatar escapes to literal @avatar', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('\\@avatar'), '@avatar');
});

test('normalizeInputAliases: \\@here escapes to literal @here', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(flow.normalizeInputAliases('\\@here'), '@here');
});

test('normalizeInputAliases: @here.owner @avatar expands only @avatar (not @here)', () => {
  const { flow } = makeDispatchFlow();
  assert.equal(
    flow.normalizeInputAliases('@here.owner @avatar'),
    '@here.owner @did:ma:testactor#alice'
  );
});

test('normalizeInputAliases: throws if @here has no active room', () => {
  const { flow } = makeDispatchFlow({ currentHome: null });
  assert.throws(() => flow.normalizeInputAliases('@here'), /active room/);
});

test('normalizeInputAliases: throws if @my.unknown is not in aliasBook', () => {
  const { flow } = makeDispatchFlow();
  assert.throws(() => flow.normalizeInputAliases('@my.unknown whisper'), /not defined/);
});

// rewriteAliasesToDid tests for @here.owner

test('rewriteAliasesToDid: @here.owner with bare alias rewrites to room DID dot-path', () => {
  const { flow } = makeDispatchFlow();
  const result = flow.rewriteAliasesToDid('@here.owner myfriend');
  assert.equal(result, '@did:ma:testroom#lobby.owner did:ma:testfriend#bob');
});

test('rewriteAliasesToDid: @here.owner with @did:ma: value rewrites to room DID dot-path', () => {
  const { flow } = makeDispatchFlow();
  const result = flow.rewriteAliasesToDid('@here.owner @did:ma:testactor#alice');
  assert.equal(result, '@did:ma:testroom#lobby.owner did:ma:testactor#alice');
});

test('rewriteAliasesToDid: @here.owner with plain DID rewrites to room DID dot-path', () => {
  const { flow } = makeDispatchFlow();
  const result = flow.rewriteAliasesToDid('@here.owner did:ma:testactor#alice');
  assert.equal(result, '@did:ma:testroom#lobby.owner did:ma:testactor#alice');
});

test('full pipeline: @here.owner @avatar resolves to room DID dot-path', () => {
  const { flow } = makeDispatchFlow();
  const preNorm = flow.normalizeInputAliases('@here.owner @avatar');
  const result = flow.rewriteAliasesToDid(preNorm);
  assert.equal(result, '@did:ma:testroom#lobby.owner did:ma:testactor#alice');
});
