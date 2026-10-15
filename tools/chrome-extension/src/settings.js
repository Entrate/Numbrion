/* Settings shared by the service worker, the content script and the options page (chrome.storage.local). */
(function (root) {
  'use strict';
  const DEFAULTS = {
    apiUrl: 'http://127.0.0.1:8765',
    budgetMs: null, // null: the server's default search budget
    bot: '', // '': the server's advisor bot
    enabled: true,
    autoAdvise: true,
    showBotLog: true,
    ladderUsername: '', // empty: use the player's current Showdown name
    ladderFormat: '', // empty: follow the main-menu battle format
  };
  const KEY = 'settings';

  function normalize(raw) {
    const s = Object.assign({}, DEFAULTS, raw || {});
    s.apiUrl = String(s.apiUrl || DEFAULTS.apiUrl).trim().replace(/\/+$/, '') || DEFAULTS.apiUrl;
    const budget = parseInt(s.budgetMs, 10);
    s.budgetMs = Number.isFinite(budget) && budget > 0 ? Math.min(budget, 120000) : null;
    s.bot = String(s.bot || '').trim();
    s.enabled = s.enabled !== false;
    s.autoAdvise = s.autoAdvise !== false;
    s.showBotLog = s.showBotLog !== false;
    s.ladderUsername = String(s.ladderUsername || '').replace(/[|,;\r\n]/g, '').trim().slice(0, 30);
    s.ladderFormat = /^[a-z0-9]+$/.test(s.ladderFormat || '') ? s.ladderFormat : '';
    return s;
  }

  async function load() {
    const stored = await chrome.storage.local.get(KEY);
    return normalize(stored[KEY]);
  }

  async function save(patch) {
    const current = await load();
    const next = normalize(Object.assign({}, current, patch));
    await chrome.storage.local.set({ [KEY]: next });
    return next;
  }

  root.NumbrionSettings = { DEFAULTS, KEY, normalize, load, save };
})(typeof globalThis !== 'undefined' ? globalThis : this);
