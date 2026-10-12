/* Options page and toolbar popup: edits chrome.storage.local settings and checks GET /api/status. */
(function () {
  'use strict';
  const S = globalThis.NumbrionSettings;
  const $ = (id) => document.getElementById(id);
  const fields = ['enabled', 'autoAdvise', 'showBotLog', 'apiUrl', 'budgetMs', 'bot'];
  let savedTimer = null;

  function fill(settings) {
    $('enabled').checked = settings.enabled;
    $('autoAdvise').checked = settings.autoAdvise;
    $('showBotLog').checked = settings.showBotLog;
    $('apiUrl').value = settings.apiUrl;
    $('budgetMs').value = settings.budgetMs || '';
    ensureBotOption(settings.bot);
    $('bot').value = settings.bot;
  }

  function ensureBotOption(name) {
    if (!name) return;
    const select = $('bot');
    if (![...select.options].some((option) => option.value === name)) select.append(new Option(name, name));
  }

  function read() {
    return {
      enabled: $('enabled').checked,
      autoAdvise: $('autoAdvise').checked,
      showBotLog: $('showBotLog').checked,
      apiUrl: $('apiUrl').value,
      budgetMs: $('budgetMs').value,
      bot: $('bot').value,
    };
  }

  async function save() {
    const settings = await S.save(read());
    $('saved').textContent = 'Saved';
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => ($('saved').textContent = ''), 1200);
    return settings;
  }

  function showStatus(kind, text) {
    const box = $('status');
    box.className = kind;
    box.textContent = text;
  }

  async function check() {
    await save();
    showStatus('', 'Checking…');
    const response = await chrome.runtime.sendMessage({ type: 'status' });
    if (!response || !response.ok) {
      showStatus('error', (response && response.error) || 'no answer');
      return;
    }
    const d = response.data || {};
    const lines = ['API online (' + response.elapsedMs + ' ms)'];
    if (d.server) lines.push('Showdown: ' + d.server);
    if (d.advisor) lines.push('Advisor: ' + d.advisor);
    for (const bot of d.bots || []) {
      ensureBotOption(bot.name);
      const budget = bot.search && bot.search.budget_ms ? ', budget ' + bot.search.budget_ms + ' ms' : '';
      lines.push('• ' + bot.name + (bot.connected === false ? ' (offline)' : '') + budget + (bot.battles !== undefined ? ', ' + bot.battles + ' battle(s)' : ''));
    }
    showStatus('ok', lines.join('\n'));
  }

  document.addEventListener('DOMContentLoaded', async () => {
    fill(await S.load());
    for (const id of fields) {
      $(id).addEventListener('change', save);
    }
    $('check').addEventListener('click', check);
    $('reset').addEventListener('click', async () => {
      await chrome.storage.local.set({ [S.KEY]: S.normalize(null) });
      fill(await S.load());
      showStatus('', 'Defaults restored');
    });
    check();
  });
})();
