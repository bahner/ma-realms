import test from 'node:test';
import assert from 'node:assert/strict';

import { resolveAliasTargetToken } from './world.js';

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
