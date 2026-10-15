/* Options page and toolbar popup: edits chrome.storage.local settings and checks GET /api/status. */
(function () {
  'use strict';
  const S = globalThis.NumbrionSettings;
  const $ = (id) => document.getElementById(id);
  const fields = ['enabled', 'autoAdvise', 'showBotLog', 'apiUrl', 'budgetMs', 'bot'];
  let savedTimer = null;
  let apiStatus = null;
  let statusUrl = '';
  let apiError = '';
  let checkId = 0;

  function renderAdvisor(settings, error) {
    const status = statusUrl === settings.apiUrl ? apiStatus : null;
    error = error || (statusUrl === settings.apiUrl ? apiError : '');
    const config = globalThis.NumbrionAdvisorStatus.configuration(status, settings);
    $('currentModel').textContent = error ? 'Unavailable' : config.model;
    $('currentModel').title = error ? 'The API could not confirm the current model.' : config.modelTitle;
    $('currentBot').textContent = 'Advisor: ' + config.bot;
    $('searchStatus').textContent = !settings.enabled ? 'Advisor disabled' : error ? 'Search unavailable'
      : config.error ? 'Advisor unavailable' : config.loading ? 'Model loading' : config.search;
    $('searchStatus').className = error || config.error ? 'error' : settings.enabled && config.searchEnabled ? 'ok' : '';
    $('searchDetail').textContent = !settings.enabled ? 'Enable the extension to get battle advice.'
      : error ? error + ' Check the API URL and make sure the play server is running.'
      : config.error || (config.loading ? 'Model loading. Advice will be available when it is ready.' : config.detail);
  }

  function fill(settings) {
    $('enabled').checked = settings.enabled;
    $('autoAdvise').checked = settings.autoAdvise;
    $('showBotLog').checked = settings.showBotLog;
    $('apiUrl').value = settings.apiUrl;
    $('budgetMs').value = settings.budgetMs || '';
    ensureBotOption(settings.bot);
    $('bot').value = settings.bot;
    renderAdvisor(settings);
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
    renderAdvisor(settings);
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
    const id = ++checkId;
    const settings = await save();
    showStatus('', 'Checking…');
    let response;
    try {
      response = await chrome.runtime.sendMessage({ type: 'status' });
    } catch (error) {
      response = { ok: false, error: error.message || 'No answer from the extension.' };
    }
    if (id !== checkId || settings.apiUrl !== S.normalize(read()).apiUrl) return;
    if (!response || !response.ok) {
      apiStatus = null;
      const error = response && response.error || 'No answer from the API.';
      apiError = error;
      statusUrl = settings.apiUrl;
      renderAdvisor(S.normalize(read()), error);
      showStatus('error', 'API check failed. ' + error);
      return;
    }
    const d = response.data || {};
    apiStatus = d;
    apiError = '';
    statusUrl = settings.apiUrl;
    const lines = ['API online (' + response.elapsedMs + ' ms)'];
    if (d.server) lines.push('Showdown: ' + d.server);
    if (d.advisor) lines.push('Advisor: ' + d.advisor);
    for (const bot of d.bots || []) {
      ensureBotOption(bot.name);
      const budget = bot.search && bot.search.budget_ms ? ', budget ' + bot.search.budget_ms + ' ms' : '';
      const connection = bot.showdown === false ? ' (advisor only)' : bot.connected === false ? ' (Showdown offline)' : '';
      lines.push('• ' + bot.name + connection + budget + (bot.battles !== undefined ? ', ' + bot.battles + ' battle(s)' : ''));
    }
    showStatus('ok', lines.join('\n'));
    renderAdvisor(S.normalize(read()));
  }

  document.addEventListener('DOMContentLoaded', async () => {
    fill(await S.load());
    for (const id of fields) {
      $(id).addEventListener('change', id === 'apiUrl' ? check : save);
    }
    $('check').addEventListener('click', check);
    $('reset').addEventListener('click', async () => {
      await chrome.storage.local.set({ [S.KEY]: S.normalize(null) });
      fill(await S.load());
      check();
    });
    check();
  });
})();
