/*
 * The advisor panel: a fixed, draggable, collapsible box in a shadow root (Showdown's CSS cannot reach it).
 * render(view) redraws it from a plain view model built by content.js; user actions go to the callbacks.
 */
(function (root) {
  'use strict';

  const HOST_ID = 'numbrion-advisor-host';
  const WIDTH = 340;

  const CSS = `
:host { all: initial; }
.panel {
  position: fixed; z-index: 2147483000; width: ${WIDTH}px; max-height: 78vh; display: flex; flex-direction: column;
  font: 12px/1.35 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; color: #e7eaf1;
  background: rgba(24, 28, 39, 0.97); border: 1px solid #3a4258; border-radius: 9px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45); overflow: hidden; user-select: none;
}
.head {
  display: flex; align-items: center; gap: 6px; padding: 6px 8px; background: #252b3b; cursor: move;
  border-bottom: 1px solid #343c52;
}
.brand { font-weight: 700; color: #8fc1ff; letter-spacing: 0.2px; }
.room { flex: 1; min-width: 0; color: #aab2c5; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.iconbtn {
  all: unset; cursor: pointer; width: 20px; height: 20px; text-align: center; border-radius: 4px; color: #c8cfdf;
  font-size: 13px; line-height: 20px;
}
.iconbtn:hover { background: #343c52; }
.body { overflow-y: auto; padding: 8px 10px 6px; user-select: text; }
.collapsed .body, .collapsed .foot { display: none; }
.status { display: flex; align-items: center; gap: 6px; color: #c3c9d8; margin-bottom: 6px; }
.dot { width: 8px; height: 8px; border-radius: 50%; background: #6b7488; flex: none; }
.dot.ok { background: #57c27a; } .dot.busy { background: #e2b84a; animation: pulse 1s infinite; }
.dot.warn { background: #e29a4a; } .dot.error { background: #e06363; }
@keyframes pulse { 50% { opacity: 0.35; } }
.status .txt { flex: 1; }
.muted { color: #8b93a7; }
.win { display: flex; align-items: center; gap: 8px; margin: 4px 0 6px; }
.win .pct { font-size: 18px; font-weight: 700; min-width: 48px; }
.win .meta { color: #9aa3b8; font-size: 11px; }
.bigbar { flex: 1; height: 7px; border-radius: 4px; background: #3a2f37; overflow: hidden; }
.bigbar > i { display: block; height: 100%; background: linear-gradient(90deg, #4f9c6a, #6fd08e); }
.slot { display: flex; gap: 6px; padding: 3px 0; border-top: 1px solid #2c3346; }
.slot:first-child { border-top: 0; }
.slot .mon { font-weight: 700; color: #f3d48a; flex: none; max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.slot .act { flex: 1; font-weight: 600; }
.choice { margin: 6px 0; display: flex; align-items: center; gap: 8px; }
.choice code {
  flex: 1; font: 11px ui-monospace, SFMono-Regular, Menlo, monospace; color: #b9c3da; background: #141824;
  padding: 3px 5px; border-radius: 4px; word-break: break-word;
}
.play {
  all: unset; cursor: pointer; padding: 5px 12px; border-radius: 6px; background: #3d7be0; color: white;
  font-weight: 700; white-space: nowrap;
}
.play:hover { background: #4b89ef; }
.play[disabled] { background: #3a4258; color: #8b93a7; cursor: default; }
.msg { font-size: 11px; margin: 2px 0 4px; }
.msg.ok { color: #6fd08e; } .msg.error { color: #ff8a8a; } .msg.info { color: #9aa3b8; }
h4 { margin: 9px 0 3px; font-size: 11px; text-transform: uppercase; letter-spacing: 0.6px; color: #8fa0c4; }
.cand { padding: 3px 0; border-top: 1px solid #2c3346; }
.cand.best .ctext { color: #fff; font-weight: 700; }
.ctext { color: #cfd5e3; }
.bars { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; margin-top: 2px; }
.bars.two { grid-template-columns: repeat(2, 1fr); }
.m { font-size: 10px; color: #98a1b6; display: flex; flex-direction: column; gap: 1px; }
.m .lab { display: flex; justify-content: space-between; }
.bar { height: 4px; border-radius: 2px; background: #2f3647; position: relative; overflow: hidden; }
.bar > i { position: absolute; top: 0; bottom: 0; left: 0; background: #6aa7ff; }
.bar.prior > i { background: #9b86e8; } .bar.weight > i { background: #56c0c9; }
.bar.value > i.pos { background: #63c784; } .bar.value > i.neg { background: #e07272; }
.bar.value::after { content: ""; position: absolute; left: 50%; top: 0; bottom: 0; width: 1px; background: #59627a; }
.note { margin-top: 6px; color: #c9b98a; font-size: 11px; }
.note.history { margin: -2px 0 6px; }
.log { padding: 3px 0; border-top: 1px solid #2c3346; }
.log .lh { color: #9aa3b8; font-size: 11px; }
.log .lc { color: #dfe4ef; }
.foot {
  display: flex; align-items: center; gap: 10px; padding: 5px 10px; border-top: 1px solid #343c52; background: #20263a;
  color: #b6bdcf; font-size: 11px;
}
.foot label { display: flex; align-items: center; gap: 4px; cursor: pointer; }
.foot input { margin: 0; }
.foot .sp { flex: 1; }
.link { all: unset; cursor: pointer; color: #8fc1ff; }
.link:hover { text-decoration: underline; }
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
      this.position = null; // {left, top}
      this.collapsed = false;
    }

    ensure() {
      if (this.host && this.host.isConnected) return;
      const old = document.getElementById(HOST_ID);
      if (old) old.remove(); // left by a previous version of the extension
      this.host = el('div', { id: HOST_ID });
      const shadow = this.host.attachShadow({ mode: 'open' });
      shadow.append(el('style', { text: CSS }));
      this.root = el('div', { class: 'panel' });
      this.roomEl = el('span', { class: 'room' });
      this.collapseBtn = el('button', { class: 'iconbtn', title: 'Collapse', onclick: () => this.setCollapsed(!this.collapsed) });
      this.head = el('div', { class: 'head' }, el('span', { class: 'brand', text: 'Numbrion' }), this.roomEl, this.collapseBtn);
      this.body = el('div', { class: 'body' });
      this.foot = el('div', { class: 'foot' });
      this.root.append(this.head, this.body, this.foot);
      shadow.append(this.root);
      (document.body || document.documentElement).append(this.host);
      this.installDrag();
      window.addEventListener('resize', () => this.applyPosition());
      this.applyPosition();
      this.setCollapsed(this.collapsed, true);
    }

    destroy() {
      if (this.host) this.host.remove();
      this.host = null;
    }

    setPosition(position) {
      this.position = position && Number.isFinite(position.left) && Number.isFinite(position.top) ? position : null;
      this.applyPosition();
    }

    applyPosition() {
      if (!this.root) return;
      const width = this.root.offsetWidth || WIDTH;
      const height = this.root.offsetHeight || 40;
      let left = this.position ? this.position.left : window.innerWidth - width - 16;
      let top = this.position ? this.position.top : 72;
      left = Math.max(0, Math.min(left, window.innerWidth - width));
      top = Math.max(0, Math.min(top, window.innerHeight - Math.min(height, 40)));
      this.root.style.left = left + 'px';
      this.root.style.top = top + 'px';
    }

    installDrag() {
      let start = null;
      this.head.addEventListener('pointerdown', (event) => {
        if (event.button !== 0 || event.target.closest('button')) return;
        const rect = this.root.getBoundingClientRect();
        start = { x: event.clientX, y: event.clientY, left: rect.left, top: rect.top };
        this.head.setPointerCapture(event.pointerId);
        event.preventDefault();
      });
      this.head.addEventListener('pointermove', (event) => {
        if (!start) return;
        this.position = { left: start.left + event.clientX - start.x, top: start.top + event.clientY - start.y };
        this.applyPosition();
      });
      const end = (event) => {
        if (!start) return;
        start = null;
        try {
          this.head.releasePointerCapture(event.pointerId);
        } catch (e) { /* ignore */ }
        const rect = this.root.getBoundingClientRect();
        this.position = { left: Math.round(rect.left), top: Math.round(rect.top) };
        if (this.cb.onMoved) this.cb.onMoved(this.position);
      };
      this.head.addEventListener('pointerup', end);
      this.head.addEventListener('pointercancel', end);
    }

    setCollapsed(collapsed, silent) {
      this.collapsed = !!collapsed;
      if (!this.root) return;
      this.root.classList.toggle('collapsed', this.collapsed);
      this.collapseBtn.textContent = this.collapsed ? '▸' : '–';
      this.collapseBtn.title = this.collapsed ? 'Expand' : 'Collapse';
      if (!silent && this.cb.onCollapse) this.cb.onCollapse(this.collapsed);
      if (this.view) this.renderHead(this.view);
    }

    render(view) {
      this.view = view;
      if (!view || !view.visible) {
        if (this.host) this.host.style.display = 'none';
        return;
      }
      this.ensure();
      this.host.style.display = '';
      this.renderHead(view);
      this.body.replaceChildren(...this.renderBody(view));
      this.foot.replaceChildren(...this.renderFoot(view));
    }

    renderHead(view) {
      let text = view.header || '';
      if (this.collapsed && view.advice && view.advice.summary) text = view.advice.summary;
      this.roomEl.textContent = text;
      this.roomEl.title = view.roomid || '';
    }

    renderBody(view) {
      const out = [];
      const status = view.status || { kind: 'idle', text: '' };
      out.push(el('div', { class: 'status' }, el('span', { class: 'dot ' + (status.kind || '') }), el('span', { class: 'txt', text: status.text }), status.right ? el('span', { class: 'muted', text: status.right }) : null));
      if (view.historyNote) out.push(el('div', { class: 'note history', text: view.historyNote }));

      const advice = view.advice;
      if (advice) {
        if (advice.winProbability !== null && advice.winProbability !== undefined) {
          const fill = el('i');
          fill.style.width = clamp01(advice.winProbability) * 100 + '%';
          out.push(el('div', { class: 'win' }, el('span', { class: 'pct', text: pct(advice.winProbability) }), el('div', { class: 'bigbar', title: 'Win probability' }, fill), el('span', { class: 'meta', text: advice.meta || '' })));
        } else if (advice.meta) {
          out.push(el('div', { class: 'muted', text: advice.meta }));
        }
        const slots = el('div');
        for (const slot of advice.slots || []) {
          slots.append(el('div', { class: 'slot' }, el('span', { class: 'mon', text: slot.pokemon || '' }), el('span', { class: 'act', text: slot.text || '' })));
        }
        out.push(slots);
        const play = view.play || {};
        out.push(el('div', { class: 'choice' }, el('code', { text: advice.choice || '(no choice)' }), el('button', {
          class: 'play',
          disabled: !play.enabled,
          title: play.title || 'Submit this choice in the client',
          text: play.label || 'Play this',
          onclick: () => this.cb.onPlay && this.cb.onPlay(),
        })));
        if (play.message) out.push(el('div', { class: 'msg ' + (play.messageKind || 'info'), text: play.message }));

        if (advice.candidates && advice.candidates.length) {
          out.push(el('h4', { text: 'Alternatives' }));
          for (const cand of advice.candidates) {
            const bars = [];
            if (cand.prior !== null && cand.prior !== undefined) bars.push(meter('prior', 'prior', cand.prior, pct(cand.prior)));
            if (cand.value !== null && cand.value !== undefined) bars.push(meter('value', 'value', cand.value, signed(cand.value)));
            if (cand.weight !== null && cand.weight !== undefined) bars.push(meter('weight', 'weight', cand.weight, pct(cand.weight)));
            out.push(el('div', { class: 'cand' + (cand.best ? ' best' : ''), title: cand.choice || '' }, el('div', { class: 'ctext', text: cand.text || cand.choice || '' }), el('div', { class: 'bars' }, bars)));
          }
        }
        if (advice.opponent && advice.opponent.length) {
          out.push(el('h4', { text: 'Opponent likely' }));
          for (const reply of advice.opponent) {
            const bars = [];
            if (reply.prior !== null && reply.prior !== undefined) bars.push(meter('prior', 'prior', reply.prior, pct(reply.prior)));
            if (reply.weight !== null && reply.weight !== undefined) bars.push(meter('weight', 'weight', reply.weight, pct(reply.weight)));
            out.push(el('div', { class: 'cand' }, el('div', { class: 'ctext', text: reply.text || '' }), el('div', { class: 'bars two' }, bars)));
          }
        }
        if (advice.note) out.push(el('div', { class: 'note', text: advice.note }));
      }
      if (view.sent) out.push(el('div', { class: 'msg info', text: view.sent }));

      const log = view.botLog;
      if (log && log.enabled) {
        out.push(el('h4', { text: 'Bot log' + (log.entries && log.entries.length ? ' (' + log.entries.length + ')' : '') }));
        if (log.error) out.push(el('div', { class: 'muted', text: log.error }));
        else if (!log.entries || !log.entries.length) out.push(el('div', { class: 'muted', text: log.emptyText || 'No bot decisions in this room.' }));
        for (const entry of (log.entries || []).slice(0, log.limit || 8)) {
          out.push(el('div', { class: 'log', title: entry.choice || '' }, el('div', { class: 'lh', text: entry.head }), el('div', { class: 'lc', text: entry.text })));
        }
      }
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
