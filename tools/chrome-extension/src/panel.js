/*
 * The advisor panel sits at the top of Showdown's room-list sidebar beside the active battle.
 * When that sidebar is hidden, it sits below the battle's move and switch buttons instead.
 * A shadow root keeps its styles isolated.
 * render(view) redraws it from a plain view model built by content.js; user actions go to the callbacks.
 */
(function (root) {
  'use strict';

  const HOST_ID = 'numbrion-advisor-host';
  const CSS = `
:host {
  display: block; width: 100%; box-sizing: border-box; padding: 8px 6px 6px;
  --text: #333; --muted: #667078; --surface: #f8f9fa; --head: #e9edef; --border: #b4bec4;
  --line: #d6dde1; --best: #edf7e8; --best-border: #87ad77; --best-text: #31552a;
  --button-top: #ddf0d3; --button-bottom: #bedba9; --track: #dce3e6; --link: #285c91;
}
:host([data-theme="dark"]) {
  --text: #ddd; --muted: #aaa; --surface: #25292c; --head: #303639; --border: #596269;
  --line: #414a50; --best: #293c28; --best-border: #719461; --best-text: #d7edcd;
  --button-top: #426e34; --button-bottom: #305525; --track: #424a50; --link: #a0c7ef;
}
:host([data-layout="standalone"]) { max-width: 640px; margin: 12px auto; }
:host([data-layout="sidebar"]) {
  position: sticky; top: 0; z-index: 2; padding: 8px; background: var(--surface);
  max-height: var(--numbrion-sidebar-height); overflow-y: auto; scrollbar-width: thin; overscroll-behavior: contain;
}
* { box-sizing: border-box; }
.panel {
  width: 100%; font: 12px/1.4 Verdana, Arial, Helvetica, sans-serif; color: var(--text);
  background: var(--surface); border: 1px solid var(--border); border-radius: 5px; overflow: hidden;
}
.head {
  display: flex; align-items: center; gap: 8px; padding: 4px 9px; background: var(--head);
  border-bottom: 1px solid var(--line);
}
.brand { font-weight: 700; }
.room { flex: 1; min-width: 0; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.iconbtn {
  cursor: pointer; width: 24px; height: 22px; padding: 0; text-align: center; border-radius: 3px;
  color: var(--muted); border: 1px solid transparent; background: transparent; font: inherit;
}
.iconbtn:hover { border-color: var(--border); }
.body { padding: 8px 9px; }
.collapsed .body, .collapsed .foot { display: none; }
.status { display: flex; align-items: center; gap: 6px; color: var(--muted); margin-bottom: 6px; font-size: 11px; }
.status:last-child { margin-bottom: 0; }
.dot { width: 7px; height: 7px; border-radius: 50%; background: #929a9e; flex: none; }
.dot.ok { background: #5b9148; } .dot.busy { background: #bb8c2f; animation: pulse 1s infinite; }
.dot.warn { background: #bb802d; } .dot.error { background: #b94d43; }
@keyframes pulse { 50% { opacity: 0.35; } }
.status .txt { flex: 1; }
.muted { color: var(--muted); }
.decision { display: grid; grid-template-columns: minmax(0, 1fr); gap: 6px; }
.recommended { padding: 7px 9px; border: 1px solid var(--best-border); border-left-width: 4px; border-radius: 4px; background: var(--best); }
.recommendation-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 4px; }
.badge { font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: .5px; color: var(--best-text); }
.win { display: flex; align-items: baseline; gap: 4px; white-space: nowrap; color: var(--best-text); }
.win .pct { font-size: 14px; font-weight: 700; font-variant-numeric: tabular-nums; }
.win .label { font-size: 10px; }
.head .win .pct { font-size: 12px; }
.slot { display: grid; grid-template-columns: minmax(90px, 130px) 1fr; gap: 8px; padding: 3px 0; }
.slot:first-child { border-top: 0; }
.slot .mon { font-weight: 700; color: var(--best-text); overflow-wrap: anywhere; }
.slot .act { font-size: 13px; font-weight: 700; overflow-wrap: anywhere; }
.decision .slot { grid-template-columns: minmax(75px, 100px) minmax(0, 1fr); gap: 6px; }
.decision .slot .act { font-size: 12px; }
.forecast { min-width: 0; padding-top: 6px; border-top: 1px solid var(--line); }
.forecast h4 { margin: 0 0 3px; color: var(--muted); font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: .4px; }
.forecast-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: baseline; gap: 7px; padding: 3px 0; }
.forecast-actions { display: flex; flex-wrap: wrap; gap: 1px 12px; min-width: 0; font-size: 10px; line-height: 1.35; overflow-wrap: anywhere; }
.forecast-actions > span { min-width: 0; }
.forecast-actions strong { font-weight: 700; }
.forecast-pct { color: var(--link); font-size: 11px; font-weight: 700; font-variant-numeric: tabular-nums; white-space: nowrap; }
.forecast-row:first-of-type .forecast-actions { color: var(--text); font-weight: 600; }
.forecast-other, .forecast-empty { color: var(--muted); font-size: 10px; margin: 3px 0 0; }
.forecast-other { display: flex; justify-content: space-between; gap: 7px; }
.choice { margin-top: 7px; display: flex; justify-content: flex-end; }
code { display: block; margin: 5px 0; font: 11px ui-monospace, SFMono-Regular, Menlo, monospace; overflow-wrap: anywhere; color: var(--muted); }
.play {
  cursor: pointer; padding: 6px 15px; border-radius: 4px; border: 1px solid var(--best-border);
  background: linear-gradient(var(--button-top), var(--button-bottom)); color: var(--best-text);
  font: bold 12px Verdana, Arial, Helvetica, sans-serif; box-shadow: 0 1px 1px #0001; min-height: 30px;
}
.play:hover:not([disabled]) { filter: brightness(1.05); border-color: var(--best-text); }
.play[disabled] { opacity: .55; cursor: default; }
button:focus-visible, summary:focus-visible, input:focus-visible { outline: 2px solid var(--link); outline-offset: 2px; }
.msg { font-size: 11px; margin: 2px 0 4px; }
.msg.ok { color: var(--best-text); } .msg.error { color: #b6473d; } .msg.info { color: var(--muted); }
.analysis { display: flex; flex-wrap: wrap; gap: 4px 14px; border-top: 1px solid var(--line); margin-top: 8px; padding-top: 5px; }
details { min-width: 0; }
details[open] { flex-basis: 100%; order: 1; }
summary { cursor: pointer; color: var(--link); font-size: 11px; padding: 2px 0; }
h4 { margin: 9px 0 3px; font-size: 11px; color: var(--muted); }
.cand { padding: 5px 0; border-top: 1px solid var(--line); }
.ctext { color: var(--text); }
.bars { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; margin-top: 2px; }
.bars.two { grid-template-columns: repeat(2, 1fr); }
.m { font-size: 10px; color: var(--muted); display: flex; flex-direction: column; gap: 1px; }
.m .lab { display: flex; justify-content: space-between; }
.bar { height: 4px; border-radius: 2px; background: var(--track); position: relative; overflow: hidden; }
.bar > i { position: absolute; top: 0; bottom: 0; left: 0; }
.bar.prior > i { background: #9782bd; } .bar.weight > i { background: #4b9ba1; }
.bar.value > i.pos { background: #6b9b56; } .bar.value > i.neg { background: #be726d; }
.bar.value::after { content: ""; position: absolute; left: 50%; top: 0; bottom: 0; width: 1px; background: var(--muted); }
.note { margin-top: 6px; color: var(--muted); font-size: 11px; }
.note.history { margin: -2px 0 6px; }
.log { padding: 4px 0; border-top: 1px solid var(--line); }
.log .lh { color: var(--muted); font-size: 11px; }
.log .lc { color: var(--text); }
.foot {
  display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 5px 9px;
  border-top: 1px solid var(--line); color: var(--muted); font-size: 10px;
}
.foot label { display: flex; align-items: center; gap: 4px; cursor: pointer; }
.foot input { margin: 0; accent-color: #5a8348; }
.foot .sp { flex: 1; }
.link { cursor: pointer; color: var(--link); background: transparent; border: 0; padding: 0; font: inherit; }
.link:hover { text-decoration: underline; }
@media (max-width: 480px) {
  .slot { grid-template-columns: 100px 1fr; }
  .slot .act { font-size: 12px; }
}
@media (prefers-reduced-motion: reduce) { .dot.busy { animation: none; } }
`;

  function el(tag, attrs, ...children) {
    const node = document.createElement(tag);
    if (attrs) {
      for (const [key, value] of Object.entries(attrs)) {
        if (value === null || value === undefined || value === false) continue;
        if (key === 'class') node.className = value;
        else if (key === 'text') node.textContent = value;
        else if (key === 'style') node.setAttribute('style', value);
        else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
        else node.setAttribute(key, value === true ? '' : String(value));
      }
    }
    for (const child of children.flat()) {
      if (child === null || child === undefined || child === false) continue;
      node.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return node;
  }

  const clamp01 = (x) => Math.max(0, Math.min(1, Number(x) || 0));
  const pct = (x) => Math.round(clamp01(x) * 100) + '%';
  const signed = (x) => (x >= 0 ? '+' : '') + Number(x).toFixed(2);

  function winBadge(probability) {
    const win = Number.isFinite(probability) ? pct(probability) : null;
    return el('div', { class: 'win', title: win === null ? 'Win probability unavailable for this turn' : 'Estimated chance of winning the battle with the recommended play', 'aria-label': win === null ? 'Win probability unavailable' : 'Estimated win probability: ' + win },
      el('span', { class: 'pct', text: win === null ? '—' : win }), el('span', { class: 'label', text: 'win' }));
  }

  function meter(kind, label, value, shown) {
    const bar = el('div', { class: 'bar ' + kind });
    if (kind === 'value') {
      const v = Math.max(-1, Math.min(1, Number(value) || 0));
      const fill = el('i', { class: v >= 0 ? 'pos' : 'neg' });
      fill.style.left = (v >= 0 ? 50 : 50 + v * 50) + '%';
      fill.style.width = Math.abs(v) * 50 + '%';
      bar.append(fill);
    } else {
      const fill = el('i');
      fill.style.width = clamp01(value) * 100 + '%';
      bar.append(fill);
    }
    return el('div', { class: 'm' }, el('div', { class: 'lab' }, el('span', { text: label }), el('span', { text: shown })), bar);
  }

  class Panel {
    constructor(callbacks) {
      this.cb = callbacks || {};
      this.host = null;
      this.view = null;
      this.collapsed = false;
      this.expanded = new Set();
      this.controls = null;
      this.mountQueued = false;
      this.onResize = () => this.mount();
    }

    ensure() {
      if (this.host) return;
      const old = document.getElementById(HOST_ID);
      if (old) old.remove(); // left by a previous version of the extension
      this.host = el('div', { id: HOST_ID });
      const shadow = this.host.attachShadow({ mode: 'open' });
      shadow.append(el('style', { text: CSS }));
      this.root = el('section', { class: 'panel', 'aria-label': 'Numbrion advisor' });
      this.roomEl = el('span', { class: 'room' });
      this.headerWin = el('span', { style: 'display: none' });
      this.collapseBtn = el('button', { class: 'iconbtn', title: 'Collapse', onclick: () => this.setCollapsed(!this.collapsed) });
      this.head = el('div', { class: 'head' }, el('span', { class: 'brand', text: 'Numbrion advisor' }), this.roomEl, this.headerWin, this.collapseBtn);
      this.body = el('div', { class: 'body' });
      this.foot = el('div', { class: 'foot' });
      this.root.append(this.head, this.body, this.foot);
      shadow.append(this.root);
      this.layoutStyle = el('style', { id: HOST_ID + '-layout', text: '.battle-controls[data-numbrion-controls] { max-height: var(--numbrion-controls-height); overflow-y: auto; overflow-x: hidden; scrollbar-width: thin; }' });
      document.head.append(this.layoutStyle);
      // Showdown replaces the room list and controls as they update. Reattach the same host so
      // open disclosures and event handlers survive. Ignore our own rendering to avoid observer loops.
      this.observer = new MutationObserver((records) => {
        if (records.every((record) => this.host.contains(record.target))) return;
        if (this.mountQueued) return;
        this.mountQueued = true;
        queueMicrotask(() => {
          this.mountQueued = false;
          if (this.host) this.mount();
        });
      });
      this.observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
      this.observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'style'] });
      window.addEventListener('resize', this.onResize);
      this.setCollapsed(this.collapsed, true);
    }

    destroy() {
      if (this.host) this.host.remove();
      if (this.observer) this.observer.disconnect();
      if (this.layoutStyle) this.layoutStyle.remove();
      window.removeEventListener('resize', this.onResize);
      this.releaseControls();
      this.host = null;
    }

    releaseControls() {
      if (!this.controls) return;
      this.controls.removeAttribute('data-numbrion-controls');
      this.controls.style.removeProperty('--numbrion-controls-height');
      this.controls = null;
    }

    mount() {
      if (!this.host || !this.view || !this.view.visible) return;
      const room = document.getElementById('room-' + this.view.roomid) || document.getElementById(this.view.roomid);
      const controls = room && room.querySelector('.battle-controls');
      const sidebar = this.findSidebar(room);
      if (sidebar) {
        this.releaseControls();
        this.host.dataset.layout = 'sidebar';
        if (this.host.parentElement !== sidebar || sidebar.firstElementChild !== this.host) sidebar.prepend(this.host);
        const rect = sidebar.getBoundingClientRect();
        const height = Math.max(120, Math.floor(Math.min(window.innerHeight, rect.bottom) - Math.max(0, rect.top))) + 'px';
        if (this.host.style.getPropertyValue('--numbrion-sidebar-height') !== height) this.host.style.setProperty('--numbrion-sidebar-height', height);
      } else if (controls) {
        if (this.controls !== controls) this.releaseControls();
        this.controls = controls;
        this.host.dataset.layout = 'battle';
        if (this.host.parentElement !== controls) controls.append(this.host);
        controls.setAttribute('data-numbrion-controls', '');
        const bottom = Math.min(window.innerHeight, room.getBoundingClientRect().bottom);
        const height = Math.max(120, Math.floor(bottom - controls.getBoundingClientRect().top - 4)) + 'px';
        if (controls.style.getPropertyValue('--numbrion-controls-height') !== height) controls.style.setProperty('--numbrion-controls-height', height);
      } else if (!document.querySelector('.ps-room')) {
        this.releaseControls();
        // A standalone transport page has no Showdown UI to integrate with. Keep the advisor in page flow.
        this.host.dataset.layout = 'standalone';
        if (this.host.parentElement !== document.body) document.body.append(this.host);
      } else {
        this.releaseControls();
        this.host.style.display = 'none';
        return; // The client's battle controls have not been created yet.
      }
      this.host.style.display = '';
      this.host.dataset.theme = document.documentElement.classList.contains('dark') || document.body.classList.contains('dark') ? 'dark' : 'light';
    }

    findSidebar(room) {
      const sidebar = document.getElementById('room-rooms');
      if (!room || !sidebar || !room.getClientRects().length || !sidebar.getClientRects().length) return null;
      const battle = room.getBoundingClientRect();
      const side = sidebar.getBoundingClientRect();
      return battle.width > 0 && side.width >= 260 && side.height >= 120 && side.left >= battle.right - 2 && side.left < window.innerWidth
        ? sidebar : null;
    }

    setCollapsed(collapsed, silent) {
      this.collapsed = !!collapsed;
      if (!this.root) return;
      this.root.classList.toggle('collapsed', this.collapsed);
      this.collapseBtn.textContent = this.collapsed ? '▸' : '–';
      this.collapseBtn.title = this.collapsed ? 'Expand' : 'Collapse';
      this.collapseBtn.setAttribute('aria-label', this.collapseBtn.title + ' advisor');
      this.collapseBtn.setAttribute('aria-expanded', String(!this.collapsed));
      if (!silent && this.cb.onCollapse) this.cb.onCollapse(this.collapsed);
      if (this.view) this.renderHead(this.view);
    }

    render(view) {
      this.view = view;
      if (!view || !view.visible) {
        if (this.host) this.host.style.display = 'none';
        this.releaseControls();
        return;
      }
      this.ensure();
      this.host.style.display = '';
      this.root.dataset.request = view.status && view.status.right || '';
      this.renderHead(view);
      this.body.replaceChildren(...this.renderBody(view));
      this.foot.replaceChildren(...this.renderFoot(view));
      this.mount();
    }

    renderHead(view) {
      let text = view.turn ? 'Turn ' + view.turn : '';
      if (this.collapsed && view.advice && view.advice.summary) text = view.advice.summary;
      this.roomEl.textContent = text;
      this.roomEl.title = view.header || view.roomid || '';
      this.headerWin.style.display = this.collapsed && view.advice ? '' : 'none';
      this.headerWin.replaceChildren(...(this.collapsed && view.advice ? [winBadge(view.advice.winProbability)] : []));
    }

    disclosure(key, label, children) {
      const details = el('details', { class: key }, el('summary', { text: label }), children);
      details.open = this.expanded.has(key);
      details.addEventListener('toggle', () => {
        if (!details.isConnected) return;
        if (details.open) this.expanded.add(key);
        else this.expanded.delete(key);
      });
      return details;
    }

    renderForecast(opponent) {
      const probability = (reply) => Number.isFinite(reply.weight) ? reply.weight : Number.isFinite(reply.prior) ? reply.prior : null;
      const replies = (opponent || []).filter((reply) => probability(reply) !== null)
        .slice().sort((a, b) => probability(b) - probability(a)).slice(0, 3);
      const forecast = el('aside', { class: 'forecast', 'aria-label': 'Likely opponent actions', title: 'Each percentage estimates a complete opponent turn, including both Pokémon. Uses search weights when available, otherwise policy probabilities.' },
        el('h4', { text: 'Opponent likely' }));
      if (!replies.length) {
        forecast.append(el('p', { class: 'forecast-empty', text: 'No opponent estimate for this turn.' }));
        return forecast;
      }
      let shown = 0;
      for (const reply of replies) {
        const chance = clamp01(probability(reply));
        shown += chance;
        const actions = el('div', { class: 'forecast-actions' });
        for (const part of String(reply.text || '').split(';')) {
          const action = part.trim().replace(/\s*->\s*/g, ' → ');
          const colon = action.indexOf(':');
          actions.append(colon > 0 ? el('span', null, el('strong', { text: action.slice(0, colon) }), action.slice(colon)) : el('span', { text: action }));
        }
        forecast.append(el('div', { class: 'forecast-row', title: reply.text || '' }, actions,
          el('span', { class: 'forecast-pct', text: pct(chance), 'aria-label': pct(chance) + ' estimated chance of this opponent turn' })));
      }
      const other = Math.max(0, 1 - shown);
      if (other >= 0.005) forecast.append(el('div', { class: 'forecast-other' }, el('span', { text: 'Other replies' }), el('span', { text: pct(other) })));
      return forecast;
    }

    renderBody(view) {
      const out = [];
      const analysis = [];
      const status = view.status || { kind: 'idle', text: '' };
      const readyText = view.play && view.play.enabled ? 'Ready to play' : view.play && view.play.label === 'Sending…' ? 'Submitting…' : 'Choice sent';
      out.push(el('div', { class: 'status', role: 'status', 'aria-live': 'polite', title: status.text }, el('span', { class: 'dot ' + (status.kind || '') }), el('span', { class: 'txt', text: view.advice && status.kind === 'ok' ? readyText : status.text })));
      if (view.historyNote) out.push(el('div', { class: 'note history', text: view.historyNote }));

      const advice = view.advice;
      if (advice) {
        const best = el('section', { class: 'recommended', 'aria-label': 'Recommended play' });
        best.append(el('div', { class: 'recommendation-head' }, el('span', { class: 'badge', text: 'Recommended play' }),
          winBadge(advice.winProbability)));
        for (const slot of advice.slots || []) {
          best.append(el('div', { class: 'slot' }, el('span', { class: 'mon', text: slot.pokemon || '' }), el('span', { class: 'act', text: (slot.text || '').replace(/\s*->\s*/g, ' → ') })));
        }
        if (!(advice.slots || []).length) best.append(el('div', { class: 'slot' }, el('span', { class: 'act', text: advice.summary || advice.choice || 'No choice available' })));
        const play = view.play || {};
        best.append(el('div', { class: 'choice' }, el('button', {
          class: 'play',
          disabled: !play.enabled,
          title: play.title || 'Submit this choice in the client',
          text: play.label || 'Play recommended',
          onclick: () => this.cb.onPlay && this.cb.onPlay(),
        })));
        out.push(el('div', { class: 'decision' }, best, this.renderForecast(advice.opponent)));
        if (play.message) out.push(el('div', { class: 'msg ' + (play.messageKind || 'info'), text: play.message }));

        const alternatives = (advice.candidates || []).filter((cand) => !cand.best && cand.choice !== advice.choice);
        if (alternatives.length) {
          const rows = [];
          for (const cand of alternatives) {
            const bars = [];
            if (cand.prior !== null && cand.prior !== undefined) bars.push(meter('prior', 'prior', cand.prior, pct(cand.prior)));
            if (cand.value !== null && cand.value !== undefined) bars.push(meter('value', 'value', cand.value, signed(cand.value)));
            if (cand.weight !== null && cand.weight !== undefined) bars.push(meter('weight', 'weight', cand.weight, pct(cand.weight)));
            rows.push(el('div', { class: 'cand', title: cand.choice || '' }, el('div', { class: 'ctext', text: (cand.text || cand.choice || '').replace(/\s*->\s*/g, ' → ') }), el('div', { class: 'bars' }, bars)));
          }
          analysis.push(this.disclosure('alternatives', 'Other options (' + alternatives.length + ')', rows));
        }
        if (advice.opponent && advice.opponent.length) {
          const rows = [];
          for (const reply of advice.opponent) {
            const bars = [];
            if (reply.prior !== null && reply.prior !== undefined) bars.push(meter('prior', 'prior', reply.prior, pct(reply.prior)));
            if (reply.weight !== null && reply.weight !== undefined) bars.push(meter('weight', 'weight', reply.weight, pct(reply.weight)));
            rows.push(el('div', { class: 'cand' }, el('div', { class: 'ctext', text: (reply.text || '').replace(/\s*->\s*/g, ' → ') }), el('div', { class: 'bars two' }, bars)));
          }
          analysis.push(this.disclosure('opponent', 'Opponent moves', rows));
        }
        analysis.push(this.disclosure('search', 'Search details', [el('div', { class: 'win' }, el('span', { class: 'meta muted', text: advice.meta || '' })), el('div', { class: 'muted', text: status.text }), el('div', { class: 'choice' }, el('code', { text: advice.choice || '' })), advice.note ? el('div', { class: 'note', text: advice.note }) : null]));
      }
      if (view.sent) out.push(el('div', { class: 'msg info', text: view.sent }));

      const log = view.botLog;
      if (log && log.enabled) {
        const rows = [];
        if (log.error) rows.push(el('div', { class: 'muted', text: log.error }));
        else if (!log.entries || !log.entries.length) rows.push(el('div', { class: 'muted', text: log.emptyText || 'No bot decisions in this room.' }));
        for (const entry of (log.entries || []).slice(0, log.limit || 8)) {
          rows.push(el('div', { class: 'log', title: entry.choice || '' }, el('div', { class: 'lh', text: entry.head }), el('div', { class: 'lc', text: entry.text })));
        }
        analysis.push(this.disclosure('bot-log', 'Bot log' + (log.entries && log.entries.length ? ' (' + log.entries.length + ')' : ''), rows));
      }
      if (analysis.length) out.push(el('div', { class: 'analysis' }, analysis));
      return out;
    }

    renderFoot(view) {
      const s = view.settings || {};
      const toggle = (key, label) => {
        const box = el('input', { type: 'checkbox' });
        box.checked = !!s[key];
        box.addEventListener('change', () => this.cb.onToggle && this.cb.onToggle(key, box.checked));
        return el('label', null, box, label);
      };
      return [
        toggle('autoAdvise', 'Auto-advise'),
        toggle('showBotLog', 'Bot log'),
        el('span', { class: 'sp' }),
        el('button', { class: 'link', text: 'Advise now', title: 'Ask the advisor again for the current request', onclick: () => this.cb.onAdviseNow && this.cb.onAdviseNow() }),
      ];
    }
  }

  root.NumbrionPanel = Panel;
})(typeof globalThis !== 'undefined' ? globalThis : this);
