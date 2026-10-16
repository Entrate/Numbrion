'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const SRC = path.join(__dirname, '..', 'src');

test('the popup loads, saves, and resets the layer-2 details setting', async () => {
  let ready;
  const stored = { settings: { showDepth2: false } };
  const nodes = new Map();
  const element = (id) => {
    if (!nodes.has(id)) nodes.set(id, { value: '', checked: false, options: [], listeners: {},
      addEventListener(type, fn) { this.listeners[type] = fn; }, append(option) { this.options.push(option); } });
    return nodes.get(id);
  };
  const context = { setTimeout: () => 1, clearTimeout() {},
    document: { getElementById: element, addEventListener(type, fn) { if (type === 'DOMContentLoaded') ready = fn; } },
    chrome: { storage: { local: {
      get: async () => stored, set: async (values) => Object.assign(stored, values),
    } }, runtime: { sendMessage: async () => ({ ok: true, data: { bots: [] } }) } },
  };
  vm.createContext(context);
  for (const file of ['settings.js', 'advisor-status.js', 'options.js']) {
    vm.runInContext(fs.readFileSync(path.join(SRC, file), 'utf8'), context);
  }
  await ready();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(element('showDepth2').checked, false);
  element('showDepth2').checked = true;
  await element('showDepth2').listeners.change();
  assert.equal(stored.settings.showDepth2, true);
  element('showDepth2').checked = false;
  await element('showDepth2').listeners.change();
  assert.equal(stored.settings.showDepth2, false);
  await element('reset').listeners.click();
  assert.equal(element('showDepth2').checked, true);
  assert.equal(stored.settings.showDepth2, true);
});
