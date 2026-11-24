"use strict";
// The text view. It renders the active tab's buffer as virtualised rows (only what's on
// screen exists in the DOM), plus the gutter, selections, decorations and minimap, and turns
// mouse, keyboard and IME input into calls on the host (app.js) and the editor core.

const X = AxnCore;
const PAD = 6; // content padding left of column 0, in px
const escHtml = (s) => s.replace(/[&<>]/g, (c) => (c === "&" ? "&amp;" : c === "<" ? "&lt;" : "&gt;"));
const PLAIN_RUN = /^[\x21-\x7e]*$/;
const PLAIN_ASCII = /^[\x20-\x7e]*$/;

class EditorView {
  constructor(root, host) {
    this.root = root;
    this.host = host;
    this.tab = null;
    this.scroller = root.querySelector(".scroller");
    this.sizer = root.querySelector(".sizer");
    this.gutter = root.querySelector(".gutter");
    this.content = root.querySelector(".content");
    this.decoLayer = root.querySelector(".deco");
    this.textLayer = root.querySelector(".text");
    this.cursorLayer = root.querySelector(".cursors");
    this.input = root.querySelector(".input");
    this.minimap = root.querySelector(".minimap");
    this.opts = {
      fontFamily: "monospace", fontSize: 14, lineHeight: 1.5, wrap: false, lineNumbers: "on", folding: true, glyphMargin: true,
      minimap: false, whitespace: "none", eol: false, indentGuides: true, rulers: [], currentLine: true, cursorStyle: "line",
      cursorBlink: true, scrollBeyond: true, matchBrackets: true, occurrences: true,
    };
    this.cw = 8;
    this.lh = 21;
    this.gw = 0;
    this.rows = new Map();
    this.htmlCache = new Map();
    this.foldCache = new Map();
    this.foldCacheKey = "";
    this.gutterHtml = "";
    this.decoHtml = "";
    this.cursorHtml = "";
    this.raf = 0;
    this.focused = false;
    this.composing = false;
    this.drag = null;
    this.deco = { find: [], current: -1, occ: [], bracket: null, flash: null };
    this.mm = { top: 0, drag: null };
    this.sizeKey = "";
    this.bind();
    new ResizeObserver(() => this.schedule()).observe(this.scroller);
    // The font can finish loading after the first measurement; measure again once it has.
    if (document.fonts) {
      document.fonts.ready.then(() => this.remeasure());
      document.fonts.addEventListener("loadingdone", () => this.remeasure());
    }
  }

  // ---------------------------------------------------------------- setup
  configure(o) {
    Object.assign(this.opts, o);
    const st = this.root.style;
    st.setProperty("--font", this.opts.fontFamily);
    st.setProperty("--fs", this.opts.fontSize + "px");
    this.lh = Math.max(10, Math.round(this.opts.fontSize * this.opts.lineHeight));
    st.setProperty("--lh", this.lh + "px");
    this.measure();
    this.root.classList.toggle("noMinimap", !this.opts.minimap);
    this.root.classList.toggle("wrapping", !!this.opts.wrap);
    this.root.dataset.cursor = this.opts.cursorStyle;
    this.htmlCache.clear();
    this.rows.forEach((el) => (el._html = null));
    this.schedule();
  }
  measure() {
    const probe = document.createElement("span");
    probe.className = "probe";
    probe.textContent = "0".repeat(200);
    this.content.append(probe);
    const w = probe.getBoundingClientRect().width / 200;
    probe.remove();
    this.cw = w > 0 ? w : this.opts.fontSize * 0.6;
    this.root.style.setProperty("--cw", this.cw + "px");
    this.checkedWidth = false;
  }
  /** Measures the character width again and redraws if it changed. */
  remeasure() {
    const old = this.cw;
    this.measure();
    if (Math.abs(old - this.cw) > 0.001) this.widthChanged();
  }
  widthChanged() {
    this.root.style.setProperty("--cw", this.cw + "px");
    this.htmlCache.clear();
    this.rows.forEach((el) => (el._html = null));
    this.sizeKey = "";
    this.schedule();
  }
  /**
   * Checks one drawn plain-ASCII row against the width the layout assumes. If the font arrived
   * after measuring, or the renderer rounds glyph advances, positions would drift; adopt the
   * real width instead.
   */
  checkWidth() {
    if (this.checkedWidth || !this.tab) return;
    const lines = this.tab.buf.doc.lines;
    for (const [, el] of this.rows) {
      const v = el._v;
      if (!v || v.a !== 0 || el._left !== PAD + "px") continue;
      const text = lines[v.line];
      if (!text || text.length < 16 || v.b !== text.length || !PLAIN_RUN.test(text.replace(/ /g, "!")) || el.querySelector(".eolMark,.foldMark,.ws")) continue;
      const w = el.getBoundingClientRect().width;
      this.checkedWidth = true;
      if (w > 0 && Math.abs(w - text.length * this.cw) > 0.75) {
        this.cw = w / text.length;
        this.widthChanged();
      }
      return;
    }
  }
  setTab(tab) {
    if (this.tab === tab) return;
    if (this.tab) this.tab.scroll = { top: this.scroller.scrollTop, left: this.scroller.scrollLeft };
    this.tab = tab;
    this.htmlCache.clear();
    this.foldCache.clear();
    for (const el of this.rows.values()) el.remove();
    this.rows.clear();
    this.deco = { find: [], current: -1, occ: [], bracket: null, flash: null };
    this.root.hidden = !tab;
    if (!tab) return;
    this.updateSize();
    const s = tab.scroll || { top: 0, left: 0 };
    this.scroller.scrollTop = s.top;
    this.scroller.scrollLeft = s.left;
    this.render();
  }

  // ---------------------------------------------------------------- events
  bind() {
    const inp = this.input;
    this.scroller.addEventListener("scroll", () => this.schedule());
    this.sizer.addEventListener("mousedown", (e) => this.onMouseDown(e));
    this.sizer.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      if (e.target.closest(".gutter")) return;
      const hit = this.hit(e);
      const b = this.tab.buf;
      if (hit && !b.sels.some((s) => X.cmp(X.sFrom(s), hit.pos) <= 0 && X.cmp(hit.pos, X.sTo(s)) <= 0)) b.setSels([X.S(hit.pos, hit.pos, null, hit.bias)]);
      this.focus();
      this.afterMove(false);
      this.host.contextMenu(e);
    });
    inp.addEventListener("keydown", (e) => {
      if (this.composing || e.isComposing || e.keyCode === 229) return;
      if (this.host.keydown(e)) {
        e.preventDefault();
        e.stopPropagation();
      }
    });
    inp.addEventListener("input", () => {
      if (this.composing) return;
      const v = inp.value;
      inp.value = "";
      if (v) this.host.typed(v);
    });
    inp.addEventListener("compositionstart", () => {
      this.composing = true;
      inp.classList.add("ime");
      this.placeInput();
    });
    inp.addEventListener("compositionend", () => {
      this.composing = false;
      inp.classList.remove("ime");
      const v = inp.value;
      inp.value = "";
      if (v) this.host.typed(v);
    });
    inp.addEventListener("paste", (e) => {
      e.preventDefault();
      const t = e.clipboardData && e.clipboardData.getData("text/plain");
      if (t) this.host.paste(t);
    });
    for (const kind of ["copy", "cut"]) {
      inp.addEventListener(kind, (e) => {
        e.preventDefault();
        if (e.clipboardData) e.clipboardData.setData("text/plain", this.host.copyText(kind === "cut"));
      });
    }
    inp.addEventListener("focus", () => {
      this.focused = true;
      this.root.classList.add("focused");
      this.restartBlink();
      this.host.focusChanged(true);
    });
    inp.addEventListener("blur", () => {
      this.focused = false;
      this.root.classList.remove("focused");
      this.host.focusChanged(false);
    });
    this.minimap.addEventListener("mousedown", (e) => this.onMinimapDown(e));
  }
  focus() {
    if (document.activeElement !== this.input) this.input.focus({ preventScroll: true });
  }

  /** Document position under a mouse event: {pos, bias, line, vcol, row}. */
  hit(e) {
    const tab = this.tab;
    if (!tab) return null;
    const { buf, layout } = tab;
    const r = this.content.getBoundingClientRect();
    const x = e.clientX - r.left - PAD, y = e.clientY - r.top;
    const total = layout.totalRows();
    const row = Math.floor(y / this.lh);
    if (row >= total) {
      const end = buf.doc.end();
      const t = buf.doc.line(end.line);
      return { pos: end, bias: 0, line: end.line, vcol: Math.max(0, x / this.cw), row: total - 1, past: true, text: t };
    }
    const { line, sub } = layout.rowLine(Math.max(0, row));
    const [a, b, last] = layout.rowRange(line, sub);
    const text = buf.doc.line(line);
    const vcol = Math.max(0, x / this.cw);
    const ch = X.offsetAt(text, a, b, vcol, layout.tab);
    return { pos: X.P(line, ch), bias: !last && ch === b ? -1 : 0, line, vcol: sub === 0 ? vcol : vcol + X.vwidth(text, 0, a, layout.tab), row, text };
  }

  onMouseDown(e) {
    const tab = this.tab;
    if (!tab || e.button !== 0) return;
    if (e.target === this.scroller) return;
    e.preventDefault();
    this.focus();
    const buf = tab.buf;
    const inGutter = !!e.target.closest(".gutter");
    const hit = this.hit(e);
    if (!hit) return;
    if (inGutter) {
      const g = e.target.closest("[data-fold]");
      if (g) return this.host.toggleFold(Number(g.dataset.fold));
      if (e.target.closest(".glyphs") || e.target.closest(".bm")) return this.host.toggleBookmark(hit.line);
      const line = hit.past ? buf.doc.size - 1 : hit.line;
      const lineEnd = (l) => (l + 1 < buf.doc.size ? X.P(l + 1, 0) : X.P(l, buf.doc.line(l).length));
      const anchor = e.shiftKey ? buf.main.anchor : X.P(line, 0);
      buf.setSels([X.S(anchor, lineEnd(line))]);
      this.drag = { kind: "lines", anchorLine: e.shiftKey ? buf.main.anchor.line : line, lineEnd };
      this.afterMove(false);
      return this.startDrag();
    }
    const count = Math.min(e.detail || 1, 3);
    const doc = buf.doc;
    const columnMode = this.host.columnMode && this.host.columnMode() && count === 1 && !e.altKey && !e.ctrlKey && !e.shiftKey;
    if ((e.altKey && e.shiftKey) || (e.altKey && !e.ctrlKey && count === 1) || columnMode) {
      // Alt+drag (Notepad++) / Shift+Alt+drag (VS Code): column selection. A plain Alt+click adds a cursor.
      this.drag = { kind: "box", line: hit.line, vcol: hit.vcol, moved: false, sels: buf.sels.slice(), primary: buf.primary, hit, single: columnMode };
      if (columnMode) {
        buf.setSels([X.S(hit.pos, hit.pos, null, hit.bias)]);
        this.afterMove(false);
      }
      return this.startDrag();
    }
    if (count === 2 || count === 3) {
      let from, to;
      if (count === 2) {
        const r = this.wordRange(doc.line(hit.pos.line), hit.pos.ch);
        from = X.P(hit.pos.line, r[0]);
        to = X.P(hit.pos.line, r[1]);
      } else {
        from = X.P(hit.pos.line, 0);
        to = hit.pos.line + 1 < doc.size ? X.P(hit.pos.line + 1, 0) : X.P(hit.pos.line, doc.line(hit.pos.line).length);
      }
      const s = X.S(from, to);
      if (e.ctrlKey) buf.setSels([...buf.sels, s], buf.sels.length);
      else buf.setSels([s]);
      this.drag = { kind: count === 2 ? "words" : "lines", from, to, anchorLine: hit.pos.line };
      this.afterMove(false);
      this.host.selectionMade("mouse");
      return this.startDrag();
    }
    if (e.shiftKey) {
      const m = buf.main;
      const sels = [...buf.sels];
      sels[buf.primary] = X.S(m.anchor, hit.pos, null, hit.bias);
      buf.setSels(sels, buf.primary);
      this.drag = { kind: "chars", anchor: m.anchor };
    } else if (e.ctrlKey) {
      const existing = buf.sels.findIndex((s) => X.sEmpty(s) && X.peq(s.head, hit.pos));
      if (existing >= 0 && buf.sels.length > 1) {
        buf.setSels(buf.sels.filter((_, i) => i !== existing), 0);
        this.afterMove(false);
        return;
      }
      buf.setSels([...buf.sels, X.S(hit.pos, hit.pos, null, hit.bias)], buf.sels.length);
      this.drag = { kind: "chars", anchor: hit.pos, multi: true };
    } else {
      buf.setSels([X.S(hit.pos, hit.pos, null, hit.bias)]);
      this.drag = { kind: "chars", anchor: hit.pos };
    }
    this.afterMove(false);
    this.startDrag();
  }

  wordRange(text, ch) {
    const wordRe = (this.host.langOf(this.tab).wordChars) || X.DEFAULT_WORD;
    const w = X.wordAt(text, ch, wordRe);
    if (w) return w;
    // Double-click on spaces or punctuation selects that run instead.
    const c = text[ch] ?? text[ch - 1];
    if (c === undefined) return [ch, ch];
    const sp = c === " " || c === "\t";
    let a = ch, b = ch;
    const same = (x) => x !== undefined && (x === " " || x === "\t") === sp && !wordRe.test(x);
    while (a > 0 && same(text[a - 1])) a--;
    while (b < text.length && same(text[b])) b++;
    return [a, b];
  }

  startDrag() {
    const move = (e) => this.onDragMove(e);
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      clearInterval(this.autoScroll);
      this.autoScroll = 0;
      const d = this.drag;
      this.drag = null;
      if (d && d.kind === "box" && !d.moved && !d.single) {
        // Alt+click without dragging adds (or removes) a cursor.
        const buf = this.tab.buf;
        const p = d.hit.pos;
        const at = d.sels.findIndex((s) => X.sEmpty(s) && X.peq(s.head, p));
        if (at >= 0 && d.sels.length > 1) buf.setSels(d.sels.filter((_, i) => i !== at), 0);
        else buf.setSels([...d.sels, X.S(p, p, null, d.hit.bias)], d.sels.length);
        this.afterMove(false);
      }
      if (d) this.host.selectionMade("mouse");
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  }

  onDragMove(e) {
    const d = this.drag;
    if (!d || !this.tab) return;
    if (!(e.buttons & 1)) {
      document.dispatchEvent(new MouseEvent("mouseup"));
      return;
    }
    this.lastDragEvent = e;
    const r = this.scroller.getBoundingClientRect();
    const out = e.clientY < r.top ? e.clientY - r.top : e.clientY > r.bottom ? e.clientY - r.bottom : 0;
    if (out && !this.autoScroll) {
      this.autoScroll = setInterval(() => {
        const ev = this.lastDragEvent;
        const rr = this.scroller.getBoundingClientRect();
        const o = ev.clientY < rr.top ? ev.clientY - rr.top : ev.clientY > rr.bottom ? ev.clientY - rr.bottom : 0;
        if (!o) return;
        this.scroller.scrollTop += Math.sign(o) * Math.min(60, Math.abs(o));
        this.dragTo(ev);
      }, 40);
    } else if (!out && this.autoScroll) {
      clearInterval(this.autoScroll);
      this.autoScroll = 0;
    }
    this.dragTo(e);
  }

  dragTo(e) {
    const d = this.drag;
    const buf = this.tab.buf;
    const hit = this.hit(e);
    if (!hit) return;
    if (d.kind === "box") {
      if (!d.moved && Math.abs(hit.vcol - d.vcol) < 0.5 && hit.line === d.line) return;
      d.moved = true;
      const sels = X.boxSels(buf.doc, this.tab.layout.tab, d.line, d.vcol, hit.line, hit.vcol);
      buf.setSels(sels, sels.length - 1);
    } else if (d.kind === "lines") {
      const line = hit.past ? buf.doc.size - 1 : hit.line;
      const a = d.anchorLine;
      const s = line >= a ? X.S(X.P(a, 0), d.lineEnd ? d.lineEnd(line) : this.lineEndPos(line)) : X.S(d.lineEnd ? d.lineEnd(a) : this.lineEndPos(a), X.P(line, 0));
      const sels = [...buf.sels];
      sels[buf.primary] = s;
      buf.setSels(sels, buf.primary);
    } else if (d.kind === "words") {
      const t = buf.doc.line(hit.pos.line);
      const w = this.wordRange(t, hit.pos.ch);
      let s;
      if (X.cmp(hit.pos, d.from) < 0) s = X.S(d.to, X.P(hit.pos.line, w[0]));
      else s = X.S(d.from, X.cmp(hit.pos, d.to) > 0 ? X.P(hit.pos.line, w[1]) : d.to);
      const sels = [...buf.sels];
      sels[buf.primary] = s;
      buf.setSels(sels, buf.primary);
    } else {
      const sels = [...buf.sels];
      sels[buf.primary] = X.S(d.anchor, hit.pos, null, hit.bias);
      buf.setSels(sels, buf.primary);
    }
    this.afterMove(false, true);
  }
  lineEndPos(l) {
    const doc = this.tab.buf.doc;
    return l + 1 < doc.size ? X.P(l + 1, 0) : X.P(l, doc.line(l).length);
  }

  onMinimapDown(e) {
    if (!this.tab || e.button !== 0) return;
    e.preventDefault();
    const rect = this.minimap.getBoundingClientRect();
    const mh = 2;
    const y = e.clientY - rect.top;
    const sTop = this.scroller.scrollTop, vh = this.scroller.clientHeight;
    const sliderY = (sTop / this.lh - this.mm.top) * mh, sliderH = Math.max(8, (vh / this.lh) * mh);
    let grab;
    if (y >= sliderY && y <= sliderY + sliderH) grab = y - sliderY;
    else {
      const row = this.mm.top + y / mh;
      this.scroller.scrollTop = row * this.lh - vh / 2;
      grab = sliderH / 2;
    }
    const startTop = this.scroller.scrollTop, startY = e.clientY;
    const total = this.tab.layout.totalRows();
    const max = this.scroller.scrollHeight - vh;
    const visible = Math.floor(rect.height / mh);
    const move = (ev) => {
      const dy = ev.clientY - startY;
      if (total <= visible) this.scroller.scrollTop = startTop + (dy / mh) * this.lh;
      else {
        // The slider spans the minimap height, so dragging it maps linearly onto the scroll range.
        const track = rect.height - sliderH;
        this.scroller.scrollTop = startTop + (track > 0 ? (dy / track) * max : 0);
      }
    };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
    };
    void grab;
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  }

  // ---------------------------------------------------------------- geometry and scrolling
  xOf(text, a, ch) {
    return PAD + X.vwidth(text, a, Math.max(a, ch), this.tab.layout.tab) * this.cw;
  }
  coords(pos, bias = 0) {
    const { layout, buf } = this.tab;
    const r = layout.posRow(pos, bias);
    const text = buf.doc.line(r.line);
    const ch = r.line === pos.line ? pos.ch : 0;
    return { x: this.xOf(text, r.start, Math.min(ch, r.end)), y: r.row * this.lh, row: r.row };
  }
  updateSize() {
    const tab = this.tab;
    if (!tab) return;
    const { buf, layout } = tab;
    const o = this.opts;
    const n = buf.doc.size;
    const digits = Math.max(o.lineNumbers === "relative" ? 3 : 2, String(n).length);
    const numW = o.lineNumbers !== "off" ? Math.ceil(digits * this.cw) + 18 : 0;
    const glyphW = o.glyphMargin ? 16 : 0;
    const foldW = o.folding ? 18 : 0;
    this.gw = glyphW + numW + foldW || 4;
    this.numW = numW;
    this.glyphW = glyphW;
    const vw = this.scroller.clientWidth, vh = this.scroller.clientHeight;
    if (o.wrap) {
      const cols = Math.floor((vw - this.gw - PAD - 14) / this.cw);
      layout.configure({ wrap: true, cols: o.wrapColumn ? Math.min(o.wrapColumn, cols) : cols, tab: tab.tabSize });
    } else layout.configure({ wrap: false, tab: tab.tabSize });
    const total = layout.totalRows();
    const extra = o.scrollBeyond ? Math.max(this.lh, vh - this.lh * 2) : Math.round(this.lh / 2);
    const height = Math.min(total * this.lh + extra, 30e6);
    const width = o.wrap ? vw : Math.min(this.gw + PAD + (layout.maxWidth() + 2) * this.cw + 40, 10e6);
    const key = `${height}|${width}|${this.gw}`;
    if (key !== this.sizeKey) {
      this.sizeKey = key;
      this.sizer.style.height = height + "px";
      this.sizer.style.width = Math.max(width, 1) + "px";
      this.gutter.style.width = this.gw + "px";
      this.gutter.style.height = height + "px";
      this.content.style.left = this.gw + "px";
      this.root.style.setProperty("--gw", this.gw + "px");
    }
  }
  /** Scrolls so the position is visible: "nearest" (default), "center", or "top". */
  reveal(pos, bias = 0, mode = "nearest") {
    if (!this.tab) return;
    this.updateSize();
    const c = this.coords(pos, bias);
    const sc = this.scroller;
    const vh = sc.clientHeight, vw = sc.clientWidth;
    const lh = this.lh;
    if (mode === "center") sc.scrollTop = Math.max(0, c.y - vh / 2 + lh / 2);
    else if (mode === "top") sc.scrollTop = Math.max(0, c.y - lh * 3);
    else if (c.y < sc.scrollTop) sc.scrollTop = c.y;
    else if (c.y + lh > sc.scrollTop + vh) sc.scrollTop = c.y + lh - vh;
    if (!this.opts.wrap) {
      const left = sc.scrollLeft, avail = vw - this.gw - 14;
      if (c.x < left + PAD + this.cw * 2) sc.scrollLeft = Math.max(0, c.x - this.cw * 8);
      else if (c.x > left + avail - this.cw * 2) sc.scrollLeft = c.x - avail + this.cw * 8;
    }
  }
  pageRows() {
    return Math.max(1, Math.floor(this.scroller.clientHeight / this.lh) - 1);
  }
  /** After a selection change: reveal the primary cursor, restart blinking, re-render. */
  afterMove(reveal = true, nearestOnly = false) {
    if (!this.tab) return;
    const b = this.tab.buf;
    const m = b.main;
    if (this.tab.layout.folds.length) {
      for (const s of b.sels) if (this.tab.layout.hiddenBy(s.head.line)) this.tab.layout.reveal(s.head.line);
    }
    if (reveal || nearestOnly) this.reveal(m.head, m.bias);
    this.restartBlink();
    this.render();
    this.host.selectionChanged();
  }
  restartBlink() {
    const l = this.cursorLayer;
    l.classList.remove("blink");
    clearTimeout(this.blinkTimer);
    if (this.opts.cursorBlink) this.blinkTimer = setTimeout(() => l.classList.add("blink"), 500);
  }
  schedule() {
    if (!this.raf) this.raf = requestAnimationFrame(() => this.render());
  }

  // ---------------------------------------------------------------- rendering
  render() {
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
    const tab = this.tab;
    if (!tab || this.root.hidden) return;
    this.updateSize();
    const { buf, layout } = tab;
    const sc = this.scroller;
    const lh = this.lh;
    const total = layout.totalRows();
    const top = sc.scrollTop, vh = sc.clientHeight;
    const first = Math.max(0, Math.floor(top / lh) - 2);
    const last = Math.min(total - 1, Math.ceil((top + vh) / lh) + 2);
    const deadline = Date.now() + 40;
    const vis = [];
    for (let row = first; row <= last; row++) {
      const { line, sub } = layout.rowLine(row);
      const [a, b, isLast] = layout.rowRange(line, sub);
      vis.push({ row, line, sub, a, b, last: isLast });
    }
    this.vis = vis;
    this.renderText(vis, deadline);
    this.renderGutter(vis);
    this.renderDeco(vis);
    this.renderCursors();
    if (this.opts.minimap) this.renderMinimap();
    this.placeInput();
    if (!buf.hl.done) this.host.idleTokenize();
  }

  renderText(vis, deadline) {
    const { buf } = this.tab;
    const keep = new Set();
    const sl = this.scroller.scrollLeft;
    const vw = this.scroller.clientWidth;
    for (const v of vis) {
      keep.add(v.row);
      const text = buf.doc.lines[v.line];
      let html;
      let shift = 0;
      if (!this.opts.wrap && text.length > 4000) {
        // Very long unwrapped lines: render only the horizontally visible slice.
        const fast = PLAIN_ASCII.test(text.length > 200000 ? text.slice(0, 2000) : text);
        const c0 = Math.max(0, Math.floor((sl - this.gw) / this.cw) - 200);
        const c1 = c0 + Math.ceil(vw / this.cw) + 400;
        const a = fast ? Math.min(text.length, c0) : X.offsetAt(text, 0, text.length, c0, this.tab.layout.tab);
        const b = fast ? Math.min(text.length, c1) : X.offsetAt(text, a, text.length, c1 - c0, this.tab.layout.tab) ;
        shift = X.vwidth(text, 0, a, this.tab.layout.tab) * this.cw;
        html = this.segment(text, a, b, text.length > X.LONG_LINE ? [] : buf.hl.tokens(v.line, deadline), v.last) ;
      } else html = this.rowHtml(v, text, deadline);
      if (v.last && this.tab.layout.folds.length && this.tab.layout.foldAt(v.line)) html += '<span class="foldMark">⋯</span>';
      let el = this.rows.get(v.row);
      if (!el) {
        el = document.createElement("div");
        el.className = "row";
        this.textLayer.append(el);
        this.rows.set(v.row, el);
      }
      // The line height changes with the font size and zoom, so place the row every time.
      const top = v.row * this.lh + "px";
      if (el._top !== top) {
        el.style.top = top;
        el._top = top;
      }
      if (el._html !== html) {
        el.innerHTML = html;
        el._html = html;
      }
      el._v = v;
      const left = PAD + shift + "px";
      if (el._left !== left) {
        el.style.left = left;
        el._left = left;
      }
    }
    for (const [row, el] of this.rows) {
      if (!keep.has(row)) {
        el.remove();
        this.rows.delete(row);
      }
    }
    this.checkWidth();
  }

  rowHtml(v, text, deadline) {
    const buf = this.tab.buf;
    const tokens = text.length > X.LONG_LINE ? [] : buf.hl.tokens(v.line, deadline);
    const key = `${this.tab.layout.tab}|${this.opts.whitespace}|${this.opts.eol}|${this.tab.eol}`;
    let c = this.htmlCache.get(v.line);
    if (!c || c.text !== text || c.tokens !== tokens || c.key !== key) {
      c = { text, tokens, key, rows: [] };
      if (this.htmlCache.size > 3000) this.htmlCache.clear();
      this.htmlCache.set(v.line, c);
    }
    let html = c.rows[v.sub];
    if (html === undefined) html = c.rows[v.sub] = this.segment(text, v.a, v.b, tokens, v.last);
    return html;
  }

  /** HTML for text[a, b): token spans, fixed-width tabs, wide and control characters. */
  segment(text, a, b, tokens, isLast) {
    let out = "";
    let col = 0;
    let lo = 0, hi = tokens.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      if (tokens[m][1] <= a) lo = m + 1;
      else hi = m;
    }
    let ti = lo, i = a;
    const ws = this.opts.whitespace;
    const trailFrom = ws === "trailing" ? text.replace(/[ \t]+$/, "").length : Infinity;
    const leadTo = ws === "boundary" ? X.leadingWs(text).length : -1;
    while (i < b) {
      let type = null, end = b;
      const t = tokens[ti];
      if (t) {
        if (t[0] <= i) {
          type = t[2];
          end = Math.min(b, t[1]);
        } else end = Math.min(b, t[0]);
      }
      const r = this.chars(text, i, end, col, ws, trailFrom, leadTo);
      out += type ? `<span class="t-${type}">${r.html}</span>` : r.html;
      col = r.col;
      i = end;
      if (t && t[1] <= i) ti++;
    }
    if (this.opts.eol && isLast && this.tab) out += `<span class="eolMark">${this.tab.eol === "crlf" ? "CRLF" : this.tab.eol === "cr" ? "CR" : "LF"}</span>`;
    return out;
  }
  chars(text, i, end, col, ws, trailFrom, leadTo) {
    const seg = text.slice(i, end);
    if (ws === "none" || (ws === "trailing" && end <= trailFrom) || (ws === "boundary" && i >= leadTo && !/ {2}|\t/.test(seg))) {
      if (PLAIN_RUN.test(seg) || (PLAIN_ASCII.test(seg) && ws === "none")) return { html: escHtml(seg), col: col + seg.length };
    }
    const tab = this.tab.layout.tab;
    const cw = this.cw;
    let out = "", run = "";
    const flush = () => {
      if (run) out += escHtml(run);
      run = "";
    };
    for (let k = i; k < end; k++) {
      const c = text.charCodeAt(k);
      const show = ws === "all" || (ws === "trailing" && k >= trailFrom) || (ws === "boundary" && (k < leadTo || text[k - 1] === " " || text[k + 1] === " " || c === 9));
      if (c === 9) {
        flush();
        const w = tab - (col % tab);
        out += `<span class="tb${show ? " sh" : ""}" style="width:${w * cw}px"></span>`;
        col += w;
      } else if (c === 32) {
        if (show) {
          flush();
          out += `<span class="ws">·</span>`;
        } else run += " ";
        col++;
      } else if (c < 32 || c === 127) {
        flush();
        out += `<span class="cc">${String.fromCharCode(c === 127 ? 0x2421 : 0x2400 + c)}</span>`;
        col++;
      } else if (c < 0x2000 && !X.isHigh(c)) {
        run += text[k];
        col += c >= 0x300 && c <= 0x36f ? 0 : 1;
      } else {
        let ch = text[k];
        let cp = c;
        if (X.isHigh(c) && k + 1 < end) {
          ch = text.slice(k, k + 2);
          cp = text.codePointAt(k);
          k++;
        }
        const w = X.charWidth(cp);
        if (w === 0) run += ch;
        else {
          flush();
          out += `<span class="${w === 2 ? "w2" : "w1"}">${escHtml(ch)}</span>`;
          col += w;
        }
      }
    }
    flush();
    return { html: out, col };
  }

  renderGutter(vis) {
    const o = this.opts;
    const tab = this.tab;
    const { buf, layout } = tab;
    const lh = this.lh;
    const cur = buf.head.line;
    const curLines = new Set(buf.sels.map((s) => s.head.line));
    const foldMode = this.host.langOf(tab).fold || "indent";
    const fkey = `${buf.version}|${layout.folds.length}`;
    if (fkey !== this.foldCacheKey) {
      this.foldCache.clear();
      this.foldCacheKey = fkey;
    }
    let html = "";
    if (this.glyphW) html += `<div class="glyphs" style="width:${this.glyphW}px"></div>`;
    for (const v of vis) {
      if (v.sub !== 0) continue;
      const y = v.row * lh;
      if (this.glyphW && tab.bookmarks.has(v.line)) html += `<div class="bm" style="top:${y}px;height:${lh}px"></div>`;
      if (o.lineNumbers !== "off") {
        let label = v.line + 1;
        if (o.lineNumbers === "relative" && v.line !== cur) label = Math.abs(v.line - cur);
        html += `<div class="ln${curLines.has(v.line) ? " on" : ""}" style="top:${y}px;left:${this.glyphW}px;width:${this.numW - 8}px">${label}</div>`;
      }
      if (o.folding) {
        const folded = layout.foldAt(v.line);
        let can = this.foldCache.get(v.line);
        if (can === undefined) {
          can = folded ? true : X.canFold(buf, v.line, foldMode, layout.tab);
          this.foldCache.set(v.line, can);
        }
        if (folded || can) html += `<div class="fold${folded ? " closed" : ""}" data-fold="${v.line}" style="top:${y}px;left:${this.glyphW + this.numW}px;height:${lh}px"><svg viewBox="0 0 16 16"><path d="${folded ? "m6 4 4 4-4 4" : "m4 6 4 4 4-4"}"/></svg></div>`;
      }
    }
    if (html !== this.gutterHtml) {
      this.gutter.innerHTML = html;
      this.gutterHtml = html;
    }
  }

  /** Rectangles behind the text: current line, selections, matches, brackets, guides, rulers. */
  renderDeco(vis) {
    const o = this.opts;
    const tab = this.tab;
    const { buf, layout } = tab;
    const lh = this.lh, cw = this.cw;
    const doc = buf.doc;
    let html = "";
    if (!vis.length) return;
    const firstLine = vis[0].line, lastLine = vis[vis.length - 1].line;
    const rowsOf = new Map();
    for (const v of vis) {
      if (!rowsOf.has(v.line)) rowsOf.set(v.line, []);
      rowsOf.get(v.line).push(v);
    }
    const rect = (v, x1, x2, cls) => {
      if (x2 - x1 < 0.5 && !cls.includes("caret")) return "";
      return `<div class="${cls}" style="top:${v.row * lh}px;left:${x1}px;width:${Math.max(1, x2 - x1)}px;height:${lh}px"></div>`;
    };
    const range = (from, to, cls, eolExtra = false) => {
      let out = "";
      for (let l = Math.max(from.line, firstLine); l <= Math.min(to.line, lastLine); l++) {
        const rows = rowsOf.get(l);
        if (!rows) continue;
        const text = doc.lines[l];
        for (const v of rows) {
          const s = l === from.line ? Math.max(v.a, from.ch) : v.a;
          const e = l === to.line ? Math.min(v.b, to.ch) : v.b;
          if (l === from.line && from.ch > v.b) continue;
          if (l === to.line && to.ch < v.a) continue;
          if (l === to.line && to.ch === v.a && v.sub > 0 && l !== from.line) continue;
          const x1 = this.xOf(text, v.a, s);
          let x2 = this.xOf(text, v.a, e);
          if (eolExtra && l < to.line && v.last) x2 += cw * 0.6;
          out += rect(v, x1, x2, cls);
        }
      }
      return out;
    };
    // Current line.
    if (o.currentLine && buf.sels.length < 200) {
      for (const s of buf.sels) {
        if (!X.sEmpty(s)) continue;
        const rows = rowsOf.get(s.head.line);
        if (rows) for (const v of rows) html += `<div class="curLine" style="top:${v.row * lh}px;height:${lh}px"></div>`;
      }
    }
    // Indent guides.
    if (o.indentGuides) {
      const t = layout.tab;
      for (const v of vis) {
        if (v.sub) continue;
        let text = doc.lines[v.line];
        if (!text.trim()) {
          let k = v.line + 1;
          while (k < doc.size && k < v.line + 100 && !doc.lines[k].trim()) k++;
          const prev = v.line > 0 ? X.indentWidth(doc.lines[v.line - 1], t) : 0;
          const next = k < doc.size ? X.indentWidth(doc.lines[k], t) : 0;
          text = " ".repeat(Math.min(prev, next));
        }
        const w = X.indentWidth(text, t);
        for (let c = 0; c < w; c += t) html += `<div class="guide" style="top:${v.row * lh}px;left:${PAD + c * cw}px;height:${lh}px"></div>`;
      }
    }
    // Rulers.
    for (const r of o.rulers || []) html += `<div class="rulerLine" style="left:${PAD + r * cw}px"></div>`;
    // Find matches, then occurrences, then selections on top.
    const d = this.deco;
    const findIn = (list, cls, currentIdx = -1) => {
      if (!list.length) return "";
      let lo = 0, hi = list.length;
      while (lo < hi) {
        const m = (lo + hi) >> 1;
        if (list[m].to.line < firstLine) lo = m + 1;
        else hi = m;
      }
      let out = "";
      for (let i = lo; i < list.length && list[i].from.line <= lastLine; i++) out += range(list[i].from, list[i].to, i === currentIdx ? cls + " current" : cls);
      return out;
    };
    html += findIn(d.find, "match", d.current);
    html += findIn(d.occ, "occ");
    if (d.flash) html += range(d.flash.from, d.flash.to, "flash");
    for (const s of buf.sels) {
      if (X.sEmpty(s)) continue;
      const f = X.sFrom(s), t = X.sTo(s);
      if (t.line < firstLine || f.line > lastLine) continue;
      html += range(f, t, "sel", true);
    }
    if (d.bracket) {
      for (const p of [d.bracket.a, d.bracket.b]) if (p) html += range(p, X.P(p.line, p.ch + 1), "bracket");
    }
    if (html !== this.decoHtml) {
      this.decoLayer.innerHTML = html;
      this.decoHtml = html;
    }
  }

  renderCursors() {
    const { buf } = this.tab;
    const lh = this.lh;
    let html = "";
    const top = this.scroller.scrollTop, bottom = top + this.scroller.clientHeight;
    const over = this.host.overtype && this.host.overtype();
    for (const s of buf.sels) {
      const c = this.coords(s.head, s.bias);
      if (c.y + lh < top || c.y > bottom) continue;
      html += `<div class="cursor${s === buf.main ? " primary" : ""}${over ? " over" : ""}" style="top:${c.y}px;left:${c.x}px;height:${lh}px"></div>`;
    }
    if (html !== this.cursorHtml) {
      this.cursorLayer.innerHTML = html;
      this.cursorHtml = html;
    }
  }

  placeInput() {
    if (!this.tab) return;
    const m = this.tab.buf.main;
    const c = this.coords(m.head, m.bias);
    this.input.style.top = c.y + "px";
    this.input.style.left = c.x + "px";
  }

  tokenColors() {
    if (this.colors && this.colors.theme === document.documentElement.dataset.theme) return this.colors;
    const cs = getComputedStyle(this.root);
    const map = { theme: document.documentElement.dataset.theme, text: cs.getPropertyValue("--ed-text").trim() || "#ccc" };
    for (const t of (globalThis.AxnLang && AxnLang.types) || []) map[t] = cs.getPropertyValue("--t-" + t).trim() || map.text;
    map.slider = cs.getPropertyValue("--mm-slider").trim() || "rgba(128,128,160,.18)";
    map.sel = cs.getPropertyValue("--ed-sel").trim() || "rgba(80,120,255,.35)";
    map.match = cs.getPropertyValue("--ed-match").trim() || "rgba(255,190,60,.6)";
    this.colors = map;
    return map;
  }

  renderMinimap() {
    const cv = this.minimap;
    const tab = this.tab;
    const { buf, layout } = tab;
    const dpr = window.devicePixelRatio || 1;
    const W = cv.clientWidth, H = cv.clientHeight;
    if (!W || !H) return;
    if (cv.width !== Math.round(W * dpr) || cv.height !== Math.round(H * dpr)) {
      cv.width = Math.round(W * dpr);
      cv.height = Math.round(H * dpr);
    }
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, W, H);
    const mh = 2, mw = 1.1;
    const total = layout.totalRows();
    const visible = Math.floor(H / mh);
    const sc = this.scroller;
    const maxTop = Math.max(1, sc.scrollHeight - sc.clientHeight);
    const top = total <= visible ? 0 : Math.round((sc.scrollTop / maxTop) * (total - visible));
    this.mm.top = top;
    const colors = this.tokenColors();
    const maxCols = Math.floor((W - 6) / mw);
    const deadline = Date.now() + 25;
    g.globalAlpha = 0.75;
    for (let r = top; r < Math.min(total, top + visible); r++) {
      const { line, sub } = layout.rowLine(r);
      const [a, b] = layout.rowRange(line, sub);
      const text = buf.doc.lines[line];
      if (a === b) continue;
      const toks = text.length > X.LONG_LINE ? [] : buf.hl.tokens(line, deadline);
      const y = (r - top) * mh;
      let ti = 0;
      let col = 0;
      let runStart = -1, runColor = null;
      const flushRun = (endCol) => {
        if (runStart >= 0 && runStart < maxCols) {
          g.fillStyle = runColor;
          g.fillRect(3 + runStart * mw, y, Math.min(endCol, maxCols) * mw - runStart * mw, mh - 0.5);
        }
        runStart = -1;
      };
      for (let k = a; k < b && col < maxCols; k++) {
        while (ti < toks.length && toks[ti][1] <= k) ti++;
        const c = text.charCodeAt(k);
        if (c === 32 || c === 9) {
          flushRun(col);
          col += c === 9 ? layout.tab - (col % layout.tab) : 1;
          continue;
        }
        const t = toks[ti];
        const color = t && t[0] <= k ? colors[t[2]] || colors.text : colors.text;
        if (runStart >= 0 && color !== runColor) flushRun(col);
        if (runStart < 0) {
          runStart = col;
          runColor = color;
        }
        col++;
      }
      flushRun(col);
    }
    g.globalAlpha = 1;
    // Matches, selections and the viewport slider.
    g.fillStyle = colors.match;
    for (const m of this.deco.find) {
      const r = layout.posRow(m.from).row - top;
      if (r >= 0 && r < visible) g.fillRect(0, r * mh, W, mh);
    }
    const sliderY = (sc.scrollTop / this.lh - top) * mh;
    const sliderH = Math.max(8, (sc.clientHeight / this.lh) * mh);
    g.fillStyle = colors.slider;
    g.fillRect(0, sliderY, W, sliderH);
  }
}
