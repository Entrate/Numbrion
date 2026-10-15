/*
 * Service worker: the only part that talks to the advisor API (docs/play/ADVISOR-API.md). Content scripts send
 * {type: "advise" | "botLog" | "status"} messages; the worker adds the API URL, budget and bot from the settings
 * and answers {ok, data} or {ok: false, error, offline}. Fetching from here (with host permissions) avoids the
 * page's CORS and mixed-content rules.
 */
importScripts('settings.js', 'ladder.js');

const ladderCache = new Map();
let ladderQueue = Promise.resolve();

async function fetchLadder(username, force) {
  const userid = NumbrionLadder.userId(username);
  if (!userid || userid.length > 30) return { ok: false, error: 'Enter your Showdown username' };
  try {
    let cached = ladderCache.get(userid);
    if (!cached || force || Date.now() - cached.at > 60000) {
      const response = await fetch('https://pokemonshowdown.com/users/' + userid + '.json', {
        cache: 'no-store', credentials: 'omit', signal: AbortSignal.timeout(15000),
      });
      // Unregistered names can have ladder ratings. Showdown returns their valid JSON with status 404.
      if (!response.ok && response.status !== 404) throw new Error('Showdown ladder returned HTTP ' + response.status);
      const data = await response.json();
      cached = { at: Date.now(), username: data.username || username, ratings: NumbrionLadder.ratingsFrom(data) };
      ladderCache.set(userid, cached);
    }
    const stored = await chrome.storage.local.get(NumbrionLadder.HISTORY_KEY);
    const history = NumbrionLadder.record(stored[NumbrionLadder.HISTORY_KEY], userid, cached.ratings, cached.at);
    await chrome.storage.local.set({ [NumbrionLadder.HISTORY_KEY]: history });
    return { ok: true, data: { username: cached.username, ratings: cached.ratings, history: NumbrionLadder.historyFor(history, userid), updatedAt: cached.at } };
  } catch (error) {
    return { ok: false, error: error.message || 'Could not reach the official ladder' };
  }
}

function ladder(username, force) {
  // Multiple tabs share one history. Serialize writes so a second tab cannot overwrite a new snapshot.
  const result = ladderQueue.then(() => fetchLadder(username, force));
  ladderQueue = result.then(() => {}, () => {});
  return result;
}

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
  else if (message.type === 'ladder') work = ladder(message.username, message.force === true);
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
