/* Model identity and search labels shared by the popup and battle panel. */
(function (root) {
  'use strict';

  function configuration(status, settings, botName) {
    const name = botName || settings.bot || status && status.advisor || '';
    const bot = (status && status.bots || []).find((item) => item.name === name);
    const checkpoint = bot && bot.checkpoint ? String(bot.checkpoint) : '';
    const parts = checkpoint.replace(/\\/g, '/').split('/').filter(Boolean);
    const file = parts.pop();
    if (parts[parts.length - 1] === 'snapshots') parts.pop();
    const model = file ? (parts.length ? parts[parts.length - 1] + ' / ' : '') + file
      : bot ? 'No checkpoint reported' : status ? 'Unavailable' : 'Checking model…';
    const searchEnabled = bot && Object.hasOwn(bot, 'search') ? !!bot.search : null;
    const budgetMs = settings.budgetMs || bot && bot.search && bot.search.budget_ms || null;
    const search = searchEnabled === true ? 'Search enabled' : searchEnabled === false ? 'Search off' : 'Search status unknown';
    return {
      bot: name || 'Server default', model, modelTitle: checkpoint || model,
      searchEnabled, budgetMs, search,
      detail: searchEnabled === true ? (budgetMs ? budgetMs + ' ms budget per turn' : 'Server default budget')
        : searchEnabled === false ? 'Policy only. This advisor does not run search.' : 'Check the API to confirm the search settings.',
      error: status && name && !bot ? 'The selected advisor is not available on this API.' : bot && (bot.error || bot.describe_error) || '',
      loading: !!(bot && (bot.loading || bot.ready === false && !bot.error)),
    };
  }

  function searchState(config, entry, options) {
    const opts = options || {};
    const analysis = entry && entry.result && entry.result.decision && entry.result.decision.analysis || {};
    const note = String(analysis.note || '');
    const fallback = analysis.source === 'default' ? 'Using the default choice.' : 'Using policy advice.';
    if (entry && (entry.status === 'error' || entry.status === 'offline')) {
      return { kind: 'error', text: config.searchEnabled === false ? 'Advisor failed' : 'Search failed',
        detail: (entry.error || 'No answer from the advisor.') + ' No recommendation is available. Check the API settings, then click Advise now to retry.' };
    }
    if (entry && entry.status === 'thinking') {
      if (opts.historyNote && config.searchEnabled !== false) {
        return { kind: 'warn', text: 'Search unavailable', detail: 'Preparing policy advice. ' + opts.historyNote };
      }
      return { kind: 'busy', text: config.searchEnabled === false ? 'Evaluating policy' : config.searchEnabled === true ? 'Search running' : 'Advisor running',
        detail: opts.elapsed || 'Waiting for the advisor.' };
    }
    if (analysis.source === 'search') {
      return { kind: 'ok', text: 'Search used', detail: 'This recommendation includes search.' +
        (Number.isFinite(analysis.time_ms) ? ' ' + Math.round(analysis.time_ms) + ' ms.' : '') };
    }
    if (analysis.source && /^(only one legal choice|one candidate)$/.test(note)) {
      return { kind: 'idle', text: 'Search not needed', detail: note === 'one candidate' ? 'Only one candidate for this request.' : 'Only one legal choice for this request.' };
    }
    if (analysis.source && (config.searchEnabled === false || note === 'search off')) {
      return { kind: 'idle', text: 'Search off', detail: fallback };
    }
    if (analysis.source && (config.searchEnabled === true || note && note !== 'search off')) {
      const reason = note ? note + (/[.!?]$/.test(note) ? '' : '.') : 'The advisor returned no search result or failure reason.';
      return { kind: 'error', text: 'Search failed', detail: fallback + ' ' +
        reason +
        (opts.historyNote ? ' Reload before turn 1 of your next battle to restore search.' : ' Click Advise now to retry.') };
    }
    if (analysis.source) return { kind: 'warn', text: 'Search not used', detail: fallback + ' ' + (note || 'No search result was returned.') };
    if (entry && entry.status === 'nochoice') return { kind: 'idle', text: 'Search not needed', detail: 'No choice is needed for this request.' };
    if (opts.historyNote && config.searchEnabled !== false) return { kind: 'warn', text: 'Search unavailable', detail: opts.historyNote };
    if (config.error) return { kind: 'error', text: 'Advisor unavailable', detail: config.error };
    if (config.loading) return { kind: 'busy', text: 'Model loading', detail: 'Waiting for the advisor to load the model.' };
    return { kind: config.searchEnabled === true ? 'ok' : 'idle', text: config.search,
      detail: opts.idleText || config.detail };
  }

  root.NumbrionAdvisorStatus = { configuration, searchState };
})(typeof globalThis !== 'undefined' ? globalThis : this);
