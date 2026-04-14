import test from 'node:test';
import assert from 'node:assert/strict';

import { createDotCommands } from './dot-commands.js';

function createHarness() {
  const messages = [];
  const useBindings = [];
  const resolverCalls = [];
  const state = {
    identity: {
      did: 'did:ma:test#hero',
      document_json: '{"id":"did:ma:test#hero"}',
    },
    aliasBook: {},
    aliasRewriteEnabled: true,
    mailbox: [
      {
        id: 1,
        from_did: 'did:ma:test#friend',
        from_endpoint: 'abcdef',
        content_type: 'text/plain',
        content_text: 'hello there',
        message_cbor_b64: 'YWJj',
      },
    ],
    debug: false,
    logEnabled: true,
    logLevel: 'info',
    dialogIdStyle: 'alias',
  };

  const commandApi = createDotCommands({
    state,
    appendSystemUi: (enText) => messages.push(enText),
    appendMessage: (_role, message) => messages.push(String(message)),
    uiText: (enText) => enText,
    humanizeIdentifier: (value) => String(value),
    isPrintableAliasLabel: (value) => !String(value).includes(' '),
    saveAliasBook: () => {},
    setDebugMode: () => true,
    setLogEnabled: () => true,
    setLogLevel: () => true,
    setDialogIdStyle: () => true,
    setAliasRewriteEnabled: (enabled) => {
      state.aliasRewriteEnabled = Boolean(enabled);
      return true;
    },
    setMessageTtl: () => true,
    getMessageTtl: () => 60,
    setTemporaryMessageTtlOverride: () => true,
    clearTemporaryMessageTtlOverride: () => {},
    getTemporaryMessageTtlOverride: () => null,
    setBatchTimeoutSeconds: () => true,
    setBatchRetryCount: () => true,
    runBatchCommands: async () => {},
    batchStatusLine: () => 'batch ok',
    onAliasBookChanged: () => {},
    onDotEdit: () => {},
    onDotEval: () => {},
    onDotInspect: () => {},
    resolveCommandTargetDidOrToken: async (target) => {
      resolverCalls.push(String(target));
      if (String(target) === '@my.world2mailbox') {
        return 'did:ma:test#mailbox';
      }
      return 'did:ma:test#thing';
    },
    lookupDidInCurrentRoom: async () => 'did:ma:test#thing',
    sendWorldCommandQuery: async () => 'ok',
    cacheRoomDidLookup: () => {},
    setActiveObjectTarget: (alias, did) => {
      useBindings.push({ alias, did });
    },
    refillCommandInputWithActiveTarget: () => {},
    dropCachedRoomDidLookup: () => {},
    clearActiveObjectTarget: () => {},
    pollDirectInbox: async () => {},
    pollCurrentHomeEvents: async () => {},
    prepareIdentityDocumentForSend: async () => {},
    publishIdentityToWorldDid: async () => ({ ok: true, message: 'published' }),
    sendWhisperToDid: async () => {},
    runSmokeTest: async () => {},
  });

  return { state, messages, useBindings, resolverCalls, ...commandApi };
}

test('my.home stores explicit room DID as home alias', () => {
  const harness = createHarness();

  assert.equal(harness.parseLocalCommand('my.home did:ma:testworld#lobby'), true);
  assert.equal(harness.state.aliasBook.home, 'did:ma:testworld#lobby');
  assert.equal(harness.messages.at(-1), 'Home set: home => did:ma:testworld#lobby');
});

test('my.aliases supports dotted and spaced forms', () => {
  const harness = createHarness();

  assert.equal(harness.parseLocalCommand('my.aliases.add friend did:ma:test#friend'), true);
  assert.equal(harness.state.aliasBook.friend, 'did:ma:test#friend');

  assert.equal(harness.parseLocalCommand('my.aliases del friend'), true);
  assert.equal(Object.prototype.hasOwnProperty.call(harness.state.aliasBook, 'friend'), false);
});

test('my.mail list is handled locally', () => {
  const harness = createHarness();

  assert.equal(harness.parseLocalCommand('my.mail'), true);
  assert.equal(harness.messages[0], 'Mailbox (1):');
  assert.match(harness.messages[1], /#1 from=did:ma:test#friend/);
});

test('legacy dot identity command is rejected', () => {
  const harness = createHarness();

  assert.equal(harness.parseDot('.identity'), true);
  assert.equal(harness.messages.at(-1), 'Unknown command: .identity. Try .help.');
});

test('dot use supports explicit @my alias source', async () => {
  const harness = createHarness();

  assert.equal(harness.parseDot('.use @my.world2mailbox as @innkasse'), true);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(harness.resolverCalls, ['@my.world2mailbox']);
  assert.equal(harness.useBindings.length, 1);
  assert.deepEqual(harness.useBindings[0], { alias: '@innkasse', did: 'did:ma:test#mailbox' });
});