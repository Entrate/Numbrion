/*
 * Service worker: the only part that talks to the advisor API (docs/play/ADVISOR-API.md). Content scripts send
 * {type: "advise" | "botLog" | "status"} messages; the worker adds the API URL, budget and bot from the settings
 * and answers {ok, data} or {ok: false, error, offline}. Fetching from here (with host permissions) avoids the
 * page's CORS and mixed-content rules.
 */
importScripts('settings.js');

async function callApi(path, init, timeoutMs) {
  const settings = await NumbrionSettings.load();
  const url = settings.apiUrl + path;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const started = Date.now();
  try {
    const response = await fetch(url, Object.assign({ cache: 'no-store', signal: controller.signal }, init));
    let data = null;
    try {
      data = await response.json();
    } catch (e) {
      data = null;
    }
    const elapsedMs = Date.now() - started;
    if (!data || typeof data !== 'object') return { ok: false, error: 'HTTP ' + response.status + ' (not JSON)', elapsedMs };
    if (!response.ok || data.ok === false) {
      return { ok: false, error: data.error || 'HTTP ' + response.status, data, elapsedMs };
    }
    return { ok: true, data, elapsedMs };
  } catch (e) {
    const aborted = e && e.name === 'AbortError';
    return {
      ok: false,
      offline: !aborted,
      error: aborted ? 'timed out after ' + Math.round(timeoutMs / 1000) + ' s' : 'API offline (' + settings.apiUrl + ')',
      elapsedMs: Date.now() - started,
    };
  } finally {
    clearTimeout(timer);
  }
}

async function advise(body) {
  const settings = await NumbrionSettings.load();
  const payload = Object.assign({}, body);
  if (settings.budgetMs) payload.budget_ms = settings.budgetMs;
  if (settings.bot) payload.bot = settings.bot;
  const timeout = (settings.budgetMs || 1500) + 30000;
  return callApi('/api/advise', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  }, timeout);
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!message || typeof message !== 'object') return false;
  let work = null;
  if (message.type === 'advise') work = advise(message.body);
  else if (message.type === 'botLog') work = callApi('/api/bot-log?room=' + encodeURIComponent(message.room), {}, 8000);
  else if (message.type === 'status') work = callApi('/api/status', {}, 5000);
  if (!work) return false;
  work.then(sendResponse, (e) => sendResponse({ ok: false, error: String((e && e.message) || e) }));
  return true; // async response
});

/** After install or update, attach to Showdown tabs that are already open (their battles are backfilled). */
chrome.runtime.onInstalled.addListener(async () => {
  const manifest = chrome.runtime.getManifest();
  let tabs = [];
  try {
    tabs = await chrome.tabs.query({ url: manifest.content_scripts[0].matches });
  } catch (e) {
    return;
  }
  const mainFiles = manifest.content_scripts.find((entry) => entry.world === 'MAIN').js;
  const isolatedFiles = manifest.content_scripts.find((entry) => entry.world !== 'MAIN').js;
  for (const tab of tabs) {
    try {
      await chrome.scripting.executeScript({ target: { tabId: tab.id }, world: 'MAIN', files: mainFiles });
      await chrome.scripting.executeScript({ target: { tabId: tab.id }, files: isolatedFiles });
    } catch (e) {
      // Tabs that cannot be scripted (discarded, error pages) attach on their next load.
    }
  }
});
