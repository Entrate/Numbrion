/* Official ladder snapshots and chart data. Shared by the worker, UI and Node tests. */
(function (root) {
  'use strict';
  const HISTORY_KEY = 'ladderHistoryV1';
  const MAX_POINTS = 2000;
  const userId = (name) => String(name || '').toLowerCase().replace(/[^a-z0-9]/g, '');
  const number = (value) => value === null || value === undefined || value === '' ? null : Number.isFinite(Number(value)) ? Number(value) : null;

  function ratingsFrom(data) {
    if (!data || typeof data !== 'object' || !data.ratings || typeof data.ratings !== 'object') throw new Error('Showdown returned no ladder data');
    const ratings = {};
    for (const [format, row] of Object.entries(data.ratings)) {
      if (!/^[a-z0-9]+$/.test(format) || !row || typeof row !== 'object') continue;
      const elo = number(row.elo);
      if (elo === null || elo < 0) continue;
      ratings[format] = { elo, wins: number(row.w) || 0, losses: number(row.l) || 0, ties: number(row.t) || 0 };
    }
    return ratings;
  }

  function record(history, username, ratings, at) {
    const next = Object.assign({}, history || {});
    const userid = userId(username);
    if (!userid || !Number.isFinite(at)) return next;
    for (const [format, rating] of Object.entries(ratings)) {
      const key = userid + ':' + format;
      const points = Array.isArray(next[key]) ? next[key] : [];
      const last = points[points.length - 1];
      if (last && ['elo', 'wins', 'losses', 'ties'].every((field) => last[field] === rating[field])) continue;
      next[key] = points.concat(Object.assign({ at }, rating)).slice(-MAX_POINTS);
    }
    return next;
  }

  function historyFor(history, username) {
    const prefix = userId(username) + ':';
    const result = {};
    for (const [key, points] of Object.entries(history || {})) {
      if (key.startsWith(prefix) && Array.isArray(points)) result[key.slice(prefix.length)] = points;
    }
    return result;
  }

  function formatName(id) {
    const match = /^gen(\d+)(.*)$/.exec(id || '');
    if (!match) return id;
    const names = { randombattle: 'Random Battle', randomdoublesbattle: 'Random Doubles Battle', anythinggoes: 'Anything Goes', doublesou: 'Doubles OU', ou: 'OU', uu: 'UU', ru: 'RU', nu: 'NU', pu: 'PU', ubers: 'Ubers', monotype: 'Monotype', lc: 'LC' };
    return '[Gen ' + match[1] + '] ' + (names[match[2]] || match[2]);
  }

  function chart(points) {
    const valid = (points || []).filter((p) => Number.isFinite(p.at) && Number.isFinite(p.elo)).slice().sort((a, b) => a.at - b.at);
    if (!valid.length) return null;
    const values = valid.map((p) => p.elo);
    const low = Math.floor((Math.min(...values) - 15) / 10) * 10;
    const high = Math.ceil((Math.max(...values) + 15) / 10) * 10;
    const first = valid[0].at, last = valid[valid.length - 1].at;
    return { low, high, first, last, points: valid.map((p) => Object.assign({}, p, {
      x: valid.length === 1 ? 225 : 36 + (p.at - first) / Math.max(1, last - first) * 390,
      y: 8 + (high - p.elo) / (high - low) * 88,
    })) };
  }

  const api = { HISTORY_KEY, MAX_POINTS, userId, ratingsFrom, record, historyFor, formatName, chart };
  root.NumbrionLadder = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
