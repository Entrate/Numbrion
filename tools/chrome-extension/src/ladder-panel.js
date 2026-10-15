/* A persistent official ladder card in Showdown's main menu. No bot server is needed. */
(function (root) {
  'use strict';
  const L = root.NumbrionLadder;
  const ID = 'numbrion-ladder-host';
  const CSS = `
:host { display:block; box-sizing:border-box; width:100%; padding:0 12px 14px; --text:#333; --muted:#69757d; --bg:#f8f9fa; --head:#e9edef; --line:#c4cdd2; --accent:#285c91; }
:host([data-theme="dark"]) { --text:#ddd; --muted:#aab3b8; --bg:#25292c; --head:#303639; --line:#525e65; --accent:#9ac7f4; }
* { box-sizing:border-box; }
.card { font:12px/1.4 Verdana,Arial,Helvetica,sans-serif; color:var(--text); background:var(--bg); border:1px solid var(--line); border-radius:5px; overflow:hidden; }
.head { display:flex; gap:6px; justify-content:space-between; align-items:center; padding:7px 9px; background:var(--head); border-bottom:1px solid var(--line); }
h3 { font-size:12px; margin:0; }
.official { color:var(--muted); font-size:9px; }
.body { padding:9px; }
.account { display:flex; gap:5px; margin-bottom:6px; }
input,select,button { font:11px Verdana,Arial,Helvetica,sans-serif; color:var(--text); border:1px solid var(--line); background:var(--bg); border-radius:3px; padding:5px; min-width:0; }
input { flex:1; width:0; }
select { width:100%; }
button { cursor:pointer; }
button:hover { border-color:var(--accent); }
button:disabled { opacity:.55; cursor:default; }
input:focus-visible,select:focus-visible,button:focus-visible,summary:focus-visible { outline:2px solid var(--accent); outline-offset:1px; }
.rating { display:flex; gap:8px; align-items:baseline; margin:8px 0 1px; }
.elo { font-size:28px; font-weight:700; font-variant-numeric:tabular-nums; color:var(--accent); }
.label,.muted { color:var(--muted); font-size:10px; }
.delta { margin-left:auto; font-weight:700; font-size:12px; color:var(--accent); }
.stats { display:flex; justify-content:space-between; gap:5px; font-size:10px; color:var(--muted); }
.history-head { display:flex; justify-content:space-between; align-items:center; gap:5px; margin-top:10px; }
.history-head strong { font-size:10px; }
.history-head select { width:auto; padding:2px; font-size:10px; }
svg { display:block; width:100%; height:auto; margin-top:4px; overflow:visible; }
.grid { stroke:var(--line); stroke-width:1; }
.trend { fill:none; stroke:var(--accent); stroke-width:2; vector-effect:non-scaling-stroke; }
.point { fill:var(--accent); }
.axis { fill:var(--muted); font:14px Verdana,Arial,sans-serif; }
.dates { display:flex; justify-content:space-between; font-size:9px; color:var(--muted); }
.note { color:var(--muted); font-size:10px; margin:8px 0 0; }
.error { color:#a84d41; }
details { margin-top:8px; border-top:1px solid var(--line); padding-top:5px; }
summary { cursor:pointer; color:var(--accent); font-size:10px; }
table { width:100%; border-collapse:collapse; margin-top:4px; font-size:10px; }
td,th { padding:3px 0; text-align:left; }
td:last-child,th:last-child { text-align:right; font-variant-numeric:tabular-nums; }
`;

  function el(tag, attrs, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs || {})) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'text') node.textContent = value;
      else if (key === 'class') node.className = value;
      else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
      else node.setAttribute(key, value === true ? '' : value);
    }
    for (const child of children.flat()) if (child !== null && child !== undefined) node.append(child instanceof Node ? child : String(child));
    return node;
  }

  function svg(tag, attrs, ...children) {
    const node = document.createElementNS('http://www.w3.org/2000/svg', tag);
    for (const [key, value] of Object.entries(attrs || {})) node.setAttribute(key, value);
    for (const child of children) node.append(child instanceof Node ? child : String(child));
    return node;
  }

  const date = (at) => new Date(at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  const elo = (value) => Math.round(value).toLocaleString();

  class LadderPanel {
    constructor(callbacks) {
      this.cb = callbacks || {};
      this.host = null;
      this.view = null;
      this.range = 'all';
      this.recordsOpen = false;
    }

    ensure() {
      if (this.host) return;
      document.getElementById(ID)?.remove();
      this.host = el('div', { id: ID });
      this.shadow = this.host.attachShadow({ mode: 'open' });
      this.shadow.append(el('style', { text: CSS }));
      this.card = el('section', { class: 'card', 'aria-label': 'Official Showdown ladder progress' });
      this.shadow.append(this.card);
      this.observer = new MutationObserver((records) => {
        if (records.every((r) => this.host.contains(r.target))) return;
        this.mount();
      });
      this.observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] });
      this.observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
    }

    mount() {
      if (!this.host || !this.view || !this.view.visible) return;
      const main = document.getElementById('room-');
      const menu = main && main.querySelector('.mainmenu');
      if (!menu) { this.host.style.display = 'none'; return; }
      const battle = menu.querySelector('.battleform');
      const group = battle && battle.closest('.menugroup');
      if (group) {
        if (group.nextElementSibling !== this.host) group.after(this.host);
      } else if (this.host.parentElement !== menu) menu.prepend(this.host);
      this.host.style.display = '';
      this.host.dataset.theme = document.documentElement.classList.contains('dark') || document.body.classList.contains('dark') ? 'dark' : 'light';
    }

    destroy() {
      this.observer?.disconnect();
      this.host?.remove();
      this.host = null;
    }

    render(view) {
      this.view = view;
      if (!view.visible) { if (this.host) this.host.style.display = 'none'; return; }
      this.ensure();
      const active = this.shadow.activeElement;
      const editing = active && active.name === 'ladderUsername' ? { value: active.value, start: active.selectionStart, end: active.selectionEnd } : null;
      const settings = view.settings || {};
      const username = view.username || '';
      const format = settings.ladderFormat || view.format || 'gen9randomdoublesbattle';
      const data = view.data || {};
      const rating = data.ratings && data.ratings[format];
      const history = data.history && data.history[format] || [];
      const cutoff = this.range === 'all' ? 0 : Date.now() - Number(this.range) * 86400000;
      const points = history.filter((p) => p.at >= cutoff);
      const user = el('input', { name: 'ladderUsername', 'aria-label': 'Showdown player', placeholder: 'Showdown username', autocomplete: 'off', spellcheck: 'false', value: username,
        onchange: (event) => this.cb.onUsername?.(event.target.value),
        onkeydown: (event) => { if (event.key === 'Enter') { event.preventDefault(); this.cb.onUsername?.(event.target.value); } },
      });
      const formatSelect = el('select', { 'aria-label': 'Ladder format', onchange: (event) => this.cb.onFormat?.(event.target.value) },
        el('option', { value: '', text: 'Battle format: ' + L.formatName(view.format || 'gen9randomdoublesbattle') }));
      const formats = new Set([format, ...Object.keys(data.ratings || {}), ...Object.keys(data.history || {})]);
      for (const id of [...formats].sort()) formatSelect.append(el('option', { value: id, text: L.formatName(id) }));
      formatSelect.value = settings.ladderFormat || '';
      const body = el('div', { class: 'body' }, el('div', { class: 'account' }, user,
        el('button', { text: view.loading ? 'Checking…' : 'Refresh', disabled: view.loading || !username, onclick: () => this.cb.onRefresh?.() })), formatSelect);

      if (rating) {
        const change = points.length > 1 ? Math.round(rating.elo - points[0].elo) : null;
        body.append(el('div', { class: 'rating' }, el('span', { class: 'elo', text: elo(rating.elo) }), el('span', { class: 'label', text: 'Elo' }),
          change !== null ? el('span', { class: 'delta', title: 'Change since the first recorded rating in this view', text: (change > 0 ? '+' : '') + change }) : null));
        const peak = Math.max(rating.elo, ...history.map((p) => p.elo));
        body.append(el('div', { class: 'stats' }, el('span', { text: rating.wins + 'W · ' + rating.losses + 'L' + (rating.ties ? ' · ' + rating.ties + 'T' : '') }), el('span', { text: 'Recorded peak ' + elo(peak) })));
      } else body.append(el('p', { class: 'note', text: username ? view.loading ? 'Checking the official ladder…' : 'No rating for this format yet. Play a rated match on the official server.' : 'Choose a name or enter your Showdown username to see your Elo.' }));

      const period = el('select', { 'aria-label': 'History period', onchange: (event) => { this.range = event.target.value; this.render(this.view); } });
      for (const [value, label] of [['all', 'All time'], ['30', '30 days'], ['7', '7 days']]) period.append(el('option', { value, text: label }));
      period.value = this.range;
      body.append(el('div', { class: 'history-head' }, el('strong', { text: 'Elo progress' }), period));
      const plot = L.chart(points);
      if (plot) {
        const graphic = svg('svg', { viewBox: '0 0 440 108', role: 'img', 'aria-label': 'Elo progress from ' + elo(points[0].elo) + ' to ' + elo(points[points.length - 1].elo) });
        for (const [value, y] of [[plot.high, 8], [(plot.high + plot.low) / 2, 52], [plot.low, 96]]) graphic.append(svg('line', { class: 'grid', x1: 36, x2: 426, y1: y, y2: y }), svg('text', { class: 'axis', x: 0, y: y + 3 }, String(Math.round(value))));
        graphic.append(svg('polyline', { class: 'trend', points: plot.points.map((p) => p.x + ',' + p.y).join(' ') }));
        const dots = plot.points.length <= 20 ? plot.points : [plot.points[0], plot.points[plot.points.length - 1]];
        for (const point of dots) graphic.append(svg('circle', { class: 'point', cx: point.x, cy: point.y, r: 3 }, svg('title', {}, elo(point.elo) + ' Elo · ' + new Date(point.at).toLocaleString())));
        body.append(graphic, el('div', { class: 'dates' }, el('span', { text: date(plot.first) }), el('span', { text: date(plot.last) })));
      }
      if (points.length < 2) body.append(el('p', { class: 'note', text: points.length ? 'Tracking started. Your next rating change will extend this chart.' : 'Rating history starts when this tracker first sees your ladder rating.' }));
      if (points.length > 1) {
        const table = el('table', null, el('thead', null, el('tr', null, el('th', { text: 'Recorded' }), el('th', { text: 'Elo' }))));
        const rows = el('tbody');
        for (const point of points.slice(-5).reverse()) rows.append(el('tr', null, el('td', { text: new Date(point.at).toLocaleString() }), el('td', { text: elo(point.elo) })));
        table.append(rows);
        const details = el('details', { ontoggle: (event) => { if (event.target.isConnected) this.recordsOpen = event.target.open; } }, el('summary', { text: 'Recent ratings' }), table);
        details.open = this.recordsOpen;
        body.append(details);
      }
      if (view.error) body.append(el('p', { class: 'note error', role: 'status', text: view.error }));
      if (data.updatedAt) body.append(el('p', { class: 'note', text: 'Updated ' + new Date(data.updatedAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }) }));
      this.card.replaceChildren(el('div', { class: 'head' }, el('h3', { text: 'Your ladder' }), el('span', { class: 'official', text: 'Official Showdown' })), body);
      if (editing) {
        user.value = editing.value;
        user.focus();
        user.setSelectionRange(editing.start, editing.end);
      }
      this.mount();
    }
  }
  root.NumbrionLadderPanel = LadderPanel;
})(typeof globalThis !== 'undefined' ? globalThis : this);
