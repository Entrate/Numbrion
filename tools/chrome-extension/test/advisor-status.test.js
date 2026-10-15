'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
require('../src/advisor-status.js');
const { configuration, searchState } = globalThis.NumbrionAdvisorStatus;
const status = {
  advisor: 'SearchBot',
  bots: [
    { name: 'SearchBot', checkpoint: '/models/teacher/snapshots/m240.pt', search: { budget_ms: 1500 } },
    { name: 'PolicyBot', checkpoint: '/models/distill/m535.pt', search: null },
  ],
};
const config = configuration(status, {});
const done = (source, note, extra) => ({ status: 'done', result: { decision: { analysis: { source, note, ...extra } } } });

test('model identity includes the training run and checkpoint, with the full path available', () => {
  assert.equal(config.model, 'teacher / m240.pt');
  assert.equal(config.modelTitle, '/models/teacher/snapshots/m240.pt');
  assert.equal(config.bot, 'SearchBot');
  assert.equal(config.searchEnabled, true);
  assert.equal(config.budgetMs, 1500);
});

test('the selected advisor and budget override the defaults; the response bot identifies existing advice', () => {
  const selected = configuration(status, { bot: 'PolicyBot', budgetMs: 600 });
  assert.equal(selected.model, 'distill / m535.pt');
  assert.equal(selected.searchEnabled, false);
  assert.equal(selected.budgetMs, 600);
  assert.equal(configuration(status, { bot: 'PolicyBot' }, 'SearchBot').model, config.model);
});

test('missing or unknown API metadata never invents a model or claims search is off', () => {
  assert.equal(configuration(null, {}).searchEnabled, null);
  assert.equal(configuration(status, { bot: 'MissingBot' }).model, 'Unavailable');
  assert.equal(configuration(status, { bot: 'MissingBot' }).searchEnabled, null);
  assert.equal(configuration({ advisor: 'PlainBot', bots: [{ name: 'PlainBot' }] }, {}).model, 'No checkpoint reported');
});

test('search running and search used are distinct from policy evaluation', () => {
  assert.equal(searchState(config, { status: 'thinking' }).text, 'Search running');
  assert.equal(searchState(configuration(status, { bot: 'PolicyBot' }), { status: 'thinking' }).text, 'Evaluating policy');
  assert.equal(searchState(config, done('search', '', { time_ms: 823 })).text, 'Search used');
  assert.match(searchState(config, done('search', '', { time_ms: 823 })).detail, /823 ms/);
});

test('failed search with a playable policy fallback has a visible error and the actual reason', () => {
  for (const note of ['no world: team could not be rebuilt', 'out of time after building the worlds', 'no candidate is legal in the sampled worlds', '']) {
    const result = searchState(config, done('policy', note));
    assert.equal(result.kind, 'error');
    assert.equal(result.text, 'Search failed');
    assert.match(result.detail, /Using policy advice/);
    if (note) assert.ok(result.detail.includes(note));
    else assert.match(result.detail, /no search result or failure reason/);
  }
});

test('forced choices and disabled search are never reported as failures', () => {
  for (const note of ['only one legal choice', 'one candidate']) {
    assert.equal(searchState(config, done('policy', note)).text, 'Search not needed');
  }
  assert.equal(searchState(config, done('policy', 'search off')).text, 'Search off');
  assert.equal(searchState(configuration(status, { bot: 'PolicyBot' }), done('policy', '')).text, 'Search off');
  assert.equal(searchState(config, { status: 'nochoice' }).text, 'Search not needed');
});

test('missing battle history explains why search is unavailable and how to restore it', () => {
  const options = { historyNote: 'Search needs the battle from its start.' };
  assert.equal(searchState(config, null, options).text, 'Search unavailable');
  assert.equal(searchState(config, { status: 'thinking' }, options).text, 'Search unavailable');
  assert.match(searchState(config, done('policy', 'missing past choices'), options).detail, /Reload before turn 1/);
});

test('API errors make the absence of a recommendation and the retry action explicit', () => {
  for (const status of ['error', 'offline']) {
    const result = searchState(config, { status, error: 'timed out after 31 s' });
    assert.equal(result.kind, 'error');
    assert.match(result.detail, /timed out after 31 s/);
    assert.match(result.detail, /No recommendation is available/);
    assert.match(result.detail, /Advise now to retry/);
  }
});
