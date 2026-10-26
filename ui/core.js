// Axentra Notepad's editor core: the document, selections, undo, highlighting cache,
// wrapping/folding layout and the editing commands. There's no DOM here, so
// tests/core_test.js runs it under gjs. Positions are {line, ch} with ch in UTF-16 units.
globalThis.AxnCore = (() => {
  "use strict";

  // ------------------------------------------------------------------ positions
  const P = (line, ch) => ({ line, ch });
  const cmp = (a, b) => a.line - b.line || a.ch - b.ch;
  const peq = (a, b) => a.line === b.line && a.ch === b.ch;
  const pmin = (a, b) => (cmp(a, b) <= 0 ? a : b);
  const pmax = (a, b) => (cmp(a, b) >= 0 ? a : b);
  const clamp = (v, lo, hi) => (v < lo ? lo : v > hi ? hi : v);

  // A selection. `goal` is the visual column kept by vertical moves; `bias` < 0 keeps a
  // cursor that sits exactly on a wrap point at the end of the upper row.
  const S = (anchor, head = anchor, goal = null, bias = 0) => ({ anchor, head, goal, bias });
  const sFrom = (s) => pmin(s.anchor, s.head);
  const sTo = (s) => pmax(s.anchor, s.head);
  const sEmpty = (s) => peq(s.anchor, s.head);

  // Large splices go element by element: spreading 100k+ arguments overflows the stack.
  function splice(arr, start, del, items) {
    if (items.length < 20000 && del < 20000) {
      arr.splice(start, del, ...items);
      return;
    }
    const tail = arr.slice(start + del);
    arr.length = start;
    for (const x of items) arr.push(x);
    for (const x of tail) arr.push(x);
  }

  // ------------------------------------------------------------------ characters
  const isHigh = (c) => c >= 0xd800 && c <= 0xdbff;
  const isLow = (c) => c >= 0xdc00 && c <= 0xdfff;
  const SIMPLE = /^[\x20-\x7e]*$/;

  /** Columns a code point takes in a monospace grid: 0 for combining marks, 2 for wide CJK/emoji. */
  function charWidth(cp) {
    if (cp < 0x300) return 1;
    if (cp <= 0x36f) return 0;
    if (cp < 0x1100) return 1;
    if (cp <= 0x115f) return 2;
    if ((cp >= 0x200b && cp <= 0x200f) || (cp >= 0xfe00 && cp <= 0xfe0f) || (cp >= 0x20d0 && cp <= 0x20ff) ||
        (cp >= 0x1ab0 && cp <= 0x1aff) || (cp >= 0x1dc0 && cp <= 0x1dff) || (cp >= 0xfe20 && cp <= 0xfe2f)) return 0;
    if ((cp >= 0x2e80 && cp <= 0x303e) || (cp >= 0x3041 && cp <= 0x33ff) || (cp >= 0x3400 && cp <= 0x4dbf) ||
        (cp >= 0x4e00 && cp <= 0x9fff) || (cp >= 0xa000 && cp <= 0xa4cf) || (cp >= 0xa960 && cp <= 0xa97f) ||
        (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xf900 && cp <= 0xfaff) || (cp >= 0xfe10 && cp <= 0xfe19) ||
        (cp >= 0xfe30 && cp <= 0xfe6f) || (cp >= 0xff00 && cp <= 0xff60) || (cp >= 0xffe0 && cp <= 0xffe6) ||
        (cp >= 0x1f300 && cp <= 0x1f64f) || (cp >= 0x1f680 && cp <= 0x1f6ff) || (cp >= 0x1f900 && cp <= 0x1f9ff) ||
        (cp >= 0x1fa70 && cp <= 0x1faff) || (cp >= 0x20000 && cp <= 0x3fffd)) return 2;
    return 1;
  }

  /** Visual columns of text[from, to), with tab stops counted from `from`. */
  function vwidth(text, from, to, tab) {
    let col = 0;
    for (let i = from; i < to; i++) {
      const c = text.charCodeAt(i);
      if (c === 9) col += tab - (col % tab);
      else if (c < 0x300) col++;
      else if (isHigh(c) && i + 1 < to) {
        col += charWidth(text.codePointAt(i));
        i++;
      } else col += charWidth(c);
    }
    return col;
  }

  /** The boundary in text[from, to) nearest to visual column x (fractional). */
  function offsetAt(text, from, to, x, tab) {
    let col = 0;
    for (let i = from; i < to;) {
      const c = text.charCodeAt(i);
      let n = 1, w;
      if (c === 9) w = tab - (col % tab);
      else if (c < 0x300) w = 1;
      else if (isHigh(c) && i + 1 < to) {
        w = charWidth(text.codePointAt(i));
        n = 2;
      } else w = charWidth(c);
      if (x < col + w / 2) return i;
      col += w;
      i += n;
    }
    return to;
  }

  /** Where each visual row of a wrapped line starts. Breaks after spaces and around wide characters. */
  function wrapBreaks(text, cols, tab) {
    const breaks = [0];
    cols = Math.max(cols, 8);
    let col = 0, rowStart = 0, lastBreak = -1;
    for (let i = 0; i < text.length;) {
      const c = text.charCodeAt(i);
      let n = 1, w;
      if (c === 9) w = tab - (col % tab);
      else if (c < 0x300) w = 1;
      else if (isHigh(c) && i + 1 < text.length) {
        w = charWidth(text.codePointAt(i));
        n = 2;
      } else w = charWidth(c);
      if (w === 2 && i > rowStart) lastBreak = i;
      if (col + w > cols && col > 0 && c !== 32) {
        const at = lastBreak > rowStart ? lastBreak : i;
        breaks.push(at);
        rowStart = at;
        lastBreak = -1;
        col = vwidth(text, at, i, tab);
        continue;
      }
      col += w;
      i += n;
      if (c === 32 || c === 9 || w === 2) lastBreak = i;
    }
    return breaks;
  }

  const DEFAULT_WORD = /[\p{L}\p{N}_$]/u;
  const isSpace = (ch) => ch === " " || ch === "\t";
  function charClass(ch, wordRe) {
    if (isSpace(ch)) return 0;
    return wordRe.test(ch) ? 1 : 2;
  }
  /** The word touching ch (preferring the one to the right), as [start, end], or null. */
  function wordAt(text, ch, wordRe = DEFAULT_WORD) {
    let a = ch, b = ch;
    if (!(ch < text.length && wordRe.test(text[ch])) && !(ch > 0 && wordRe.test(text[ch - 1]))) return null;
    while (a > 0 && wordRe.test(text[a - 1])) a--;
    while (b < text.length && wordRe.test(text[b])) b++;
    return [a, b];
  }
  const leadingWs = (t) => /^[ \t]*/.exec(t)[0];
  function indentWidth(t, tab) {
    return vwidth(t, 0, leadingWs(t).length, tab);
  }

  // ------------------------------------------------------------------ document
  class Doc {
    constructor(text = "") {
      this.lines = Array.isArray(text) ? text : String(text).split("\n");
      if (!this.lines.length) this.lines.push("");
    }
    get size() { return this.lines.length; }
    line(i) { return this.lines[i] ?? ""; }
    text() { return this.lines.join("\n"); }
    end() {
      const n = this.lines.length - 1;
      return P(n, this.lines[n].length);
    }
    clip(p) {
      const line = clamp(p.line, 0, this.lines.length - 1);
      return P(line, clamp(p.ch, 0, this.lines[line].length));
    }
    get(from, to) {
      if (from.line === to.line) return this.lines[from.line].slice(from.ch, to.ch);
      const out = [this.lines[from.line].slice(from.ch)];
      for (let i = from.line + 1; i < to.line; i++) out.push(this.lines[i]);
      out.push(this.lines[to.line].slice(0, to.ch));
      return out.join("\n");
    }
    charCount() {
      let n = this.lines.length - 1;
      for (const l of this.lines) n += l.length;
      return n;
    }
    /** Replaces [from, to) with text and describes the change for undo and listeners. */
    replace(from, to, text) {
      from = this.clip(from);
      to = this.clip(to);
      if (cmp(from, to) > 0) [from, to] = [to, from];
      const removed = this.get(from, to);
      const head = this.lines[from.line].slice(0, from.ch), tail = this.lines[to.line].slice(to.ch);
      let newTo;
      if (from.line === to.line && text.indexOf("\n") < 0) {
        this.lines[from.line] = head + text + tail;
        newTo = P(from.line, from.ch + text.length);
      } else {
        const parts = text.split("\n");
        const repl = [head + parts[0]];
        for (let k = 1; k < parts.length - 1; k++) repl.push(parts[k]);
        if (parts.length > 1) repl.push(parts[parts.length - 1] + tail);
        else repl[0] += tail;
        splice(this.lines, from.line, to.line - from.line + 1, repl);
        newTo = P(from.line + parts.length - 1, parts.length === 1 ? from.ch + parts[0].length : parts[parts.length - 1].length);
      }
      return { from, to, text, removed, newTo };
    }
  }

  /** Maps a position taken before change c to after it. assoc > 0 sticks to the inserted text's end. */
  function mapPos(p, c, assoc = 1) {
    const a = cmp(p, c.from);
    if (a < 0) return p;
    const b = cmp(p, c.to);
    if (b > 0 || (b === 0 && (a > 0 || assoc > 0))) {
      if (p.line === c.to.line) return P(c.newTo.line, c.newTo.ch + p.ch - c.to.ch);
      return P(p.line + c.newTo.line - c.to.line, p.ch);
    }
    if (a === 0) return p;
    return assoc > 0 ? c.newTo : c.from;
  }
  function mapSel(s, c, assoc = 1) {
    return { anchor: mapPos(s.anchor, c, assoc), head: mapPos(s.head, c, assoc), goal: null, bias: 0 };
  }
  /** A line number after change c, or -1 when the line was removed. */
  function mapLine(line, c) {
    if (line < c.from.line) return line;
    if (line > c.to.line) return line + c.newTo.line - c.to.line;
    if (line === c.from.line) return line;
    return -1;
  }

  /** Sorts selections and merges the ones that overlap, keeping track of the primary one. */
  function normalize(sels, primary) {
    const idx = sels.map((s, i) => i).sort((i, j) => cmp(sFrom(sels[i]), sFrom(sels[j])) || i - j);
    const out = [];
    let prim = 0;
    for (const i of idx) {
      const s = sels[i];
      const last = out[out.length - 1];
      if (last) {
        const lt = sTo(last.s), f = sFrom(s);
        const touch = cmp(f, lt) < 0 || (peq(f, lt) && (sEmpty(s) || sEmpty(last.s)));
        if (touch) {
          const from = pmin(sFrom(last.s), f), to = pmax(lt, sTo(s));
          const back = cmp(last.s.head, last.s.anchor) < 0;
          last.s = back ? S(to, from) : S(from, to);
          if (i === primary) last.p = true;
          continue;
        }
      }
      out.push({ s, p: i === primary });
    }
    out.forEach((o, k) => { if (o.p) prim = k; });
    return { sels: out.map((o) => o.s), primary: prim };
  }

  // ------------------------------------------------------------------ highlighting cache
  const LONG_LINE = 10000;
  const NO_TOKENS = Object.freeze([]);

  /**
   * Keeps the tokenizer state at the start of every line up to `valid`. Edits invalidate from the
   * edited line; the old states below an edit stay as candidates, so re-tokenizing stops as soon
   * as it reaches a line whose start state didn't change.
   */
  class Highlighter {
    constructor(buf, tokenize) {
      this.buf = buf;
      this.tokenize = tokenize || (() => ({ tokens: NO_TOKENS, state: null }));
      this.setLang("plaintext");
    }
    setLang(id) {
      this.lang = id;
      this.states = [null];
      this.valid = 0;
      this.regions = [];
      this.cache = new Map();
    }
    onChange(c) {
      const from = c.from.line, oldTo = c.to.line, newTo = c.newTo.line, delta = newTo - oldTo;
      if (oldTo !== newTo || from !== oldTo) {
        if (this.states.length > from + 1) splice(this.states, from + 1, Math.min(oldTo - from, this.states.length - from - 1), new Array(newTo - from).fill(undefined));
      }
      const regions = [];
      for (const [r0, r1] of this.regions) {
        let r = null;
        if (r1 <= from) r = [r0, r1];
        else if (r0 > oldTo) r = [r0 + delta, r1 + delta];
        else if (r0 <= from) r = [r0, from];
        else if (r1 > oldTo) r = [newTo + 1, r1 + delta];
        if (r && r[1] > r[0]) regions.push(r);
      }
      if (this.valid > oldTo) regions.push([newTo + 1, this.valid + delta]);
      this.valid = Math.min(this.valid, from);
      regions.sort((a, b) => a[0] - b[0]);
      this.regions = regions.slice(0, 8);
    }
    /** Brings states up to line `line`; false when the deadline passed first. */
    ensure(line, deadline = Infinity) {
      const lines = this.buf.doc.lines;
      line = Math.min(line, lines.length);
      let n = 0;
      while (this.valid < line) {
        const i = this.valid;
        const text = lines[i];
        const st = this.states[i];
        const next = text.length > LONG_LINE ? st : this.tokenize(this.lang, text, st).state;
        const idx = i + 1;
        while (this.regions.length && this.regions[0][1] < idx) this.regions.shift();
        const r = this.regions[0];
        if (r && idx >= r[0] && this.states[idx] === next) {
          this.valid = r[1];
          this.regions.shift();
          continue;
        }
        this.states[idx] = next;
        this.valid = idx;
        if ((++n & 127) === 0 && Date.now() > deadline) return false;
      }
      return true;
    }
    get done() { return this.valid >= this.buf.doc.lines.length; }
    /** Tokens for a line; lines past a missed deadline are tokenized from a guessed state and not cached. */
    tokens(i, deadline = Infinity) {
      const text = this.buf.doc.lines[i];
      if (text === undefined || text.length > LONG_LINE || !text.length) return NO_TOKENS;
      let st;
      if (this.valid >= i || this.ensure(i, deadline)) st = this.states[i];
      const hit = this.cache.get(i);
      if (st !== undefined && hit && hit.text === text && hit.st === st) return hit.tokens;
      const tokens = this.tokenize(this.lang, text, st ?? null).tokens;
      if (st !== undefined) {
        if (this.cache.size > 4000) this.cache.clear();
        this.cache.set(i, { text, st, tokens });
      }
      return tokens;
    }
    typeAt(line, ch) {
      const toks = this.tokens(line);
      let lo = 0, hi = toks.length - 1;
      while (lo <= hi) {
        const m = (lo + hi) >> 1, t = toks[m];
        if (ch < t[0]) hi = m - 1;
        else if (ch >= t[1]) lo = m + 1;
        else return t[2];
      }
      return null;
    }
  }
  const NON_CODE = new Set(["string", "comment", "regexp", "escape"]);

  // ------------------------------------------------------------------ buffer and undo
  let txSeq = 0;
  class Buffer {
    constructor(text = "", { lang = "plaintext", tokenize = null } = {}) {
      this.doc = new Doc(text);
      this.sels = [S(P(0, 0))];
      this.primary = 0;
      this.undoStack = [];
      this.redoStack = [];
      this.cleanId = 0;
      this.version = 0;
      this.readonly = false;
      this.listeners = new Set();
      this.hl = new Highlighter(this, tokenize);
      this.hl.setLang(lang);
    }
    get lang() { return this.hl.lang; }
    get main() { return this.sels[this.primary]; }
    get head() { return this.main.head; }
    get dirty() { return this.topId() !== this.cleanId; }
    topId() { return this.undoStack.length ? this.undoStack[this.undoStack.length - 1].id : 0; }
    markClean() { this.cleanId = this.topId(); }
    markDirty() { this.cleanId = -1; }
    on(fn) {
      this.listeners.add(fn);
      return () => this.listeners.delete(fn);
    }
    emit(c) {
      this.version++;
      this.hl.onChange(c);
      for (const fn of this.listeners) fn(c);
    }
    setSels(sels, primary = sels.length - 1) {
      const clipped = sels.map((s) => ({ anchor: this.doc.clip(s.anchor), head: this.doc.clip(s.head), goal: s.goal ?? null, bias: s.bias || 0 }));
      const r = normalize(clipped, clamp(primary, 0, clipped.length - 1));
      this.sels = r.sels;
      this.primary = r.primary;
    }
    /**
     * Applies edits as one undo step. items: [{from, to, text, sel?(change)}] in current
     * coordinates; overlapping ranges are merged. The new selections are each item's `sel`
     * (default: a cursor after its text), or the old selections mapped (`mapSels`), or `sels`.
     */
    apply(items, kind = "edit", opts = {}) {
      if (this.readonly) return false;
      const before = this.sels.map((s) => ({ ...s }));
      // Merge overlapping items (two cursors deleting into each other).
      const asc = items.map((it, i) => {
        const a = this.doc.clip(it.from), b = this.doc.clip(it.to);
        return { ...it, from: pmin(a, b), to: pmax(a, b), i };
      }).sort((a, b) => cmp(a.from, b.from) || a.i - b.i);
      const merged = [];
      for (const it of asc) {
        const last = merged[merged.length - 1];
        if (last && cmp(it.from, last.to) < 0) {
          last.to = pmax(last.to, it.to);
          last.text += it.text;
          continue;
        }
        merged.push(it);
      }
      const applied = [];
      const out = [];
      for (let k = merged.length - 1; k >= 0; k--) {
        const it = merged[k];
        const noop = !it.text && peq(it.from, it.to);
        const c = noop ? { from: it.from, to: it.to, text: "", removed: "", newTo: it.from } : this.doc.replace(it.from, it.to, it.text);
        if (!noop) {
          if (c.removed === c.text) {
            // Typed over an identical character: nothing changed in the document.
          } else {
            applied.push(c);
            this.emit(c);
            for (const o of out) {
              o.anchor = mapPos(o.anchor, c);
              o.head = mapPos(o.head, c);
            }
          }
        }
        const sel = it.sel ? it.sel(c) : S(c.newTo);
        out.push({ anchor: sel.anchor, head: sel.head, goal: sel.goal ?? null, bias: sel.bias || 0 });
      }
      let after;
      if (opts.sels) after = opts.sels;
      else if (opts.mapSels) {
        after = before.map((s) => {
          let a = s.anchor, h = s.head;
          for (const c of applied) {
            a = mapPos(a, c, opts.assoc ? opts.assoc(a) : 1);
            h = mapPos(h, c, opts.assoc ? opts.assoc(h) : 1);
          }
          return S(a, h);
        });
      } else after = out.reverse();
      this.setSels(after, Math.min(this.primary, after.length - 1));
      if (!applied.length) return false;
      const now = Date.now();
      const last = this.undoStack[this.undoStack.length - 1];
      const text = applied.length === 1 ? applied[0].text : null;
      const wsAfterWord = kind === "type" && last && text !== null && /^\s+$/.test(text) && last.lastText && !/^\s+$/.test(last.lastText);
      if (last && !opts.noMerge && (kind === "type" || kind === "delback" || kind === "delfwd") && last.kind === kind &&
          last.id !== this.cleanId && now - last.time < 1500 && sameSels(last.after, before) && !wsAfterWord) {
        last.changes.push(...applied);
        last.after = this.sels.map((s) => ({ ...s }));
        last.time = now;
        last.lastText = text;
        last.id = ++txSeq;
      } else {
        this.undoStack.push({ id: ++txSeq, kind, changes: applied, before, after: this.sels.map((s) => ({ ...s })), time: now, lastText: text });
        if (this.undoStack.length > 5000) this.undoStack.splice(0, 1000);
      }
      this.redoStack.length = 0;
      return true;
    }
    undo() {
      const tx = this.undoStack.pop();
      if (!tx) return false;
      for (let k = tx.changes.length - 1; k >= 0; k--) {
        const c = tx.changes[k];
        this.emit(this.doc.replace(c.from, c.newTo, c.removed));
      }
      this.redoStack.push(tx);
      this.setSels(tx.before.map((s) => ({ ...s })));
      return true;
    }
    redo() {
      const tx = this.redoStack.pop();
      if (!tx) return false;
      for (const c of tx.changes) this.emit(this.doc.replace(c.from, c.to, c.text));
      this.undoStack.push(tx);
      this.setSels(tx.after.map((s) => ({ ...s })));
      return true;
    }
    /** Replaces the whole text as one undo step (reload from disk, format document). */
    setText(text, kind = "replace") {
      const ok = this.apply([{ from: P(0, 0), to: this.doc.end(), text }], kind, { mapSels: true, noMerge: true });
      return ok;
    }
  }
  function sameSels(a, b) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!peq(a[i].anchor, b[i].anchor) || !peq(a[i].head, b[i].head)) return false;
    return true;
  }

  // ------------------------------------------------------------------ layout: wrapping and folds
  const ZERO = Object.freeze([0]);
  const FIT = /^[^\t̀-￿]*$/;

  class Layout {
    constructor(buf) {
      this.buf = buf;
      this.wrap = false;
      this.cols = 80;
      this.tab = 4;
      this.folds = [];
      this.counts = null;
      this.prefix = null;
      this.stale = 0;
      this.breakCache = new Map();
      this.maxW = 0;
      this.maxStale = true;
      this.off = buf.on((c) => this.onChange(c));
    }
    dispose() { this.off(); }
    configure({ wrap = this.wrap, cols = this.cols, tab = this.tab } = {}) {
      cols = Math.max(8, Math.floor(cols));
      if (wrap === this.wrap && (cols === this.cols || !wrap) && tab === this.tab) return false;
      const tabChanged = tab !== this.tab;
      this.wrap = wrap;
      this.cols = cols;
      this.tab = tab;
      this.breakCache.clear();
      if (tabChanged) this.maxStale = true;
      this.rebuild();
      return true;
    }
    get plain() { return !this.wrap && !this.folds.length; }
    breaks(i) {
      if (!this.wrap) return ZERO;
      const text = this.buf.doc.lines[i] ?? "";
      if (text.length <= this.cols && FIT.test(text)) return ZERO;
      let b = this.breakCache.get(text);
      if (!b) {
        b = wrapBreaks(text, this.cols, this.tab);
        if (this.breakCache.size > 5000) this.breakCache.clear();
        this.breakCache.set(text, b);
      }
      return b;
    }
    hiddenBy(i) {
      for (const f of this.folds) if (f.start < i && i <= f.end) return f;
      return null;
    }
    rebuild() {
      if (this.plain) {
        this.counts = this.prefix = null;
        return;
      }
      const n = this.buf.doc.lines.length;
      const counts = new Array(n);
      const folds = this.folds;
      let fi = 0, hideTo = -1;
      for (let i = 0; i < n; i++) {
        while (fi < folds.length && folds[fi].start < i) {
          hideTo = Math.max(hideTo, folds[fi].end);
          fi++;
        }
        counts[i] = i <= hideTo ? 0 : this.breaks(i).length;
      }
      this.counts = counts;
      this.prefix = null;
      this.stale = 0;
    }
    rows(i) {
      if (this.plain) return 1;
      return this.counts[i] ?? 1;
    }
    flush() {
      const n = this.counts.length;
      if (!this.prefix || this.prefix.length !== n + 1) {
        this.prefix = new Float64Array(n + 1);
        this.stale = 0;
      }
      if (this.stale > n) return;
      const pre = this.prefix, counts = this.counts;
      for (let i = this.stale; i < n; i++) pre[i + 1] = pre[i] + counts[i];
      this.stale = n + 1;
    }
    onChange(c) {
      const folds = this.mapFolds(c);
      for (let k = c.from.line; k <= c.newTo.line; k++) {
        const w = this.lineWidth(k);
        if (w > this.maxW) this.maxW = w;
      }
      if (c.removed.length >= this.maxW && this.maxW > 0) this.maxStale = true;
      if (folds) {
        this.rebuild();
        return;
      }
      if (this.plain) return;
      const from = c.from.line, oldTo = c.to.line, newTo = c.newTo.line;
      const fresh = [];
      for (let i = from; i <= newTo; i++) fresh.push(this.hiddenBy(i) ? 0 : this.breaks(i).length);
      splice(this.counts, from, oldTo - from + 1, fresh);
      this.stale = Math.min(this.stale, from);
      if (this.prefix && this.prefix.length !== this.counts.length + 1) this.prefix = null;
    }
    /** Shifts folds through an edit; edits inside a folded block unfold it. True when any fold changed. */
    mapFolds(c) {
      if (!this.folds.length) return false;
      const delta = c.newTo.line - c.to.line;
      let changed = false;
      const next = [];
      for (const f of this.folds) {
        const a = c.from.line, b = c.to.line;
        if (a > f.end) next.push(f);
        else if (b < f.start) {
          if (delta) changed = true;
          next.push({ start: f.start + delta, end: f.end + delta });
        } else if (b === f.start && a <= f.start) {
          const start = c.newTo.line;
          if (start !== f.start || delta) changed = true;
          next.push({ start, end: f.end + delta });
        } else changed = true;
      }
      this.folds = next;
      return changed;
    }
    setFolds(folds) {
      this.folds = folds.filter((f) => f.end > f.start).sort((a, b) => a.start - b.start || b.end - a.end);
      this.rebuild();
    }
    fold(start, end) {
      if (end <= start || this.folds.some((f) => f.start === start)) return false;
      this.setFolds([...this.folds, { start, end }]);
      return true;
    }
    unfold(start) {
      const n = this.folds.length;
      this.setFolds(this.folds.filter((f) => f.start !== start));
      return this.folds.length !== n;
    }
    /** Unfolds whatever hides line i. */
    reveal(i) {
      const n = this.folds.length;
      if (!n) return false;
      this.setFolds(this.folds.filter((f) => !(f.start < i && i <= f.end)));
      return this.folds.length !== n;
    }
    foldAt(start) { return this.folds.find((f) => f.start === start) || null; }
    totalRows() {
      if (this.plain) return this.buf.doc.lines.length;
      this.flush();
      return this.prefix[this.counts.length];
    }
    lineRow(i) {
      if (this.plain) return i;
      this.flush();
      return this.prefix[clamp(i, 0, this.counts.length)];
    }
    /** {line, sub} for a visual row. */
    rowLine(r) {
      const n = this.buf.doc.lines.length;
      if (this.plain) return { line: clamp(r, 0, n - 1), sub: 0 };
      this.flush();
      const pre = this.prefix;
      r = clamp(r, 0, Math.max(0, pre[n] - 1));
      let lo = 0, hi = n - 1;
      while (lo < hi) {
        const m = (lo + hi + 1) >> 1;
        if (pre[m] <= r) lo = m;
        else hi = m - 1;
      }
      while (lo > 0 && this.counts[lo] === 0) lo--;
      return { line: lo, sub: r - pre[lo] };
    }
    /** [start, end) offsets of row `sub` of line `line`. */
    rowRange(line, sub) {
      const b = this.breaks(line);
      const len = (this.buf.doc.lines[line] ?? "").length;
      return [b[sub] ?? len, b[sub + 1] ?? len, sub + 1 >= b.length];
    }
    /** The visual row of a position and where that row starts. */
    posRow(p, bias = 0) {
      let line = p.line;
      const hid = this.folds.length ? this.hiddenBy(line) : null;
      if (hid) line = hid.start;
      const b = this.breaks(line);
      let sub = 0;
      if (b.length > 1 && !hid) {
        let lo = 0, hi = b.length - 1;
        while (lo < hi) {
          const m = (lo + hi + 1) >> 1;
          if (b[m] <= p.ch) lo = m;
          else hi = m - 1;
        }
        sub = lo;
        if (bias < 0 && sub > 0 && b[sub] === p.ch) sub--;
      }
      const len = (this.buf.doc.lines[line] ?? "").length;
      return { row: this.lineRow(line) + sub, line, sub, start: hid ? 0 : b[sub], end: b[sub + 1] ?? len };
    }
    lineWidth(i) {
      const t = this.buf.doc.lines[i] ?? "";
      return t.indexOf("\t") < 0 && FIT.test(t) ? t.length : vwidth(t, 0, t.length, this.tab);
    }
    /** The widest line in columns (for the horizontal scroll range without wrapping). */
    maxWidth() {
      if (this.maxStale) {
        let m = 0;
        const n = this.buf.doc.lines.length;
        for (let i = 0; i < n; i++) {
          const w = this.lineWidth(i);
          if (w > m) m = w;
        }
        this.maxW = m;
        this.maxStale = false;
      }
      return this.maxW;
    }
  }

  // ------------------------------------------------------------------ movement
  function moveChar(doc, p, dir) {
    const t = doc.line(p.line);
    if (dir < 0) {
      if (p.ch > 0) {
        let ch = p.ch - 1;
        if (ch > 0 && isLow(t.charCodeAt(ch)) && isHigh(t.charCodeAt(ch - 1))) ch--;
        return P(p.line, ch);
      }
      return p.line > 0 ? P(p.line - 1, doc.line(p.line - 1).length) : p;
    }
    if (p.ch < t.length) {
      let ch = p.ch + 1;
      if (ch < t.length && isLow(t.charCodeAt(ch)) && isHigh(t.charCodeAt(ch - 1))) ch++;
      return P(p.line, ch);
    }
    return p.line < doc.size - 1 ? P(p.line + 1, 0) : p;
  }

  /** Ctrl+Left/Right: skip spaces, then a run of word or punctuation characters. */
  function moveWord(doc, p, dir, wordRe = DEFAULT_WORD) {
    const t = doc.line(p.line);
    let i = p.ch;
    if (dir > 0) {
      if (i >= t.length) return p.line < doc.size - 1 ? P(p.line + 1, 0) : p;
      while (i < t.length && isSpace(t[i])) i++;
      if (i < t.length) {
        const cls = charClass(t[i], wordRe);
        while (i < t.length && charClass(t[i], wordRe) === cls) i++;
      }
      return P(p.line, i);
    }
    if (i === 0) return p.line > 0 ? P(p.line - 1, doc.line(p.line - 1).length) : p;
    while (i > 0 && isSpace(t[i - 1])) i--;
    if (i > 0) {
      const cls = charClass(t[i - 1], wordRe);
      while (i > 0 && charClass(t[i - 1], wordRe) === cls) i--;
    }
    return P(p.line, i);
  }

  /** Up/down by visual rows, keeping the goal column. Returns {pos, goal, bias}. */
  function moveVertical(layout, s, rows) {
    const doc = layout.buf.doc;
    const p = s.head;
    const r = layout.posRow(p, s.bias);
    const t = doc.line(r.line);
    const goal = s.goal ?? vwidth(t, r.start, Math.max(r.start, Math.min(p.ch, r.end)), layout.tab);
    const target = r.row + rows;
    const total = layout.totalRows();
    if (target < 0) return { pos: P(0, 0), goal, bias: 0 };
    if (target >= total) return { pos: doc.end(), goal, bias: 0 };
    const { line, sub } = layout.rowLine(target);
    const [a, b, last] = layout.rowRange(line, sub);
    const text = doc.line(line);
    const ch = offsetAt(text, a, b, goal, layout.tab);
    return { pos: P(line, ch), goal, bias: !last && ch === b ? -1 : 0 };
  }

  function homePos(layout, s) {
    const p = s.head;
    const r = layout.posRow(p, s.bias);
    if (r.start > 0 && p.ch !== r.start) return P(p.line, r.start);
    const ind = leadingWs(layout.buf.doc.line(p.line)).length;
    return P(p.line, p.ch === ind ? 0 : ind);
  }
  function endPos(layout, s) {
    const p = s.head;
    const r = layout.posRow(p, s.bias);
    const len = layout.buf.doc.line(p.line).length;
    if (r.end < len && !(p.ch === r.end && s.bias < 0)) return { pos: P(p.line, r.end), bias: -1 };
    return { pos: P(p.line, len), bias: 0 };
  }

  /** Line ranges [first, last] covered by the selections, merged. A selection ending at column 0 skips that line. */
  function mergedRanges(sels) {
    const rs = sels.map((s) => {
      const f = sFrom(s), t = sTo(s);
      return [f.line, t.ch === 0 && t.line > f.line ? t.line - 1 : t.line];
    }).sort((a, b) => a[0] - b[0]);
    const out = [];
    for (const r of rs) {
      const last = out[out.length - 1];
      if (last && r[0] <= last[1] + 1) last[1] = Math.max(last[1], r[1]);
      else out.push([...r]);
    }
    return out;
  }

  // ------------------------------------------------------------------ editing commands
  const indentUnit = (o) => (o.insertSpaces ? " ".repeat(o.tabSize) : "\t");
  const AUTO_CLOSE_BEFORE = /^($|[\s;:.,=}\])>`'"])/;

  function insertText(buf, text, kind = "type") {
    const per = Array.isArray(text) ? text : null;
    return buf.apply(buf.sels.map((s, i) => ({ from: sFrom(s), to: sTo(s), text: per ? per[i] ?? "" : text })), kind);
  }

  /** Types one character with auto-closing pairs, typing over closers, surrounding selections and outdenting closers. */
  function typeChar(buf, ch, o) {
    const doc = buf.doc;
    const lang = o.lang || {};
    const pairs = o.autoClose ? lang.autoClose || [] : [];
    const open = pairs.find((p) => p[0] === ch);
    const close = pairs.find((p) => p[1] === ch);
    const wordRe = lang.wordChars || DEFAULT_WORD;
    const brackets = lang.brackets || [];
    const closer = brackets.find((b) => b[1] === ch);
    const items = buf.sels.map((s) => {
      const from = sFrom(s), to = sTo(s);
      if (open && !sEmpty(s) && o.autoSurround !== false) {
        const back = cmp(s.head, s.anchor) < 0;
        return {
          from, to, text: open[0] + doc.get(from, to) + open[1],
          sel: (c) => {
            const a = P(c.from.line, c.from.ch + open[0].length);
            const b = P(c.newTo.line, c.newTo.ch - open[1].length);
            return back ? S(b, a) : S(a, b);
          },
        };
      }
      const t = doc.line(to.line);
      if (close && sEmpty(s) && t.startsWith(close[1], to.ch) && (close[0] !== close[1] || o.overtypeQuotes !== false)) {
        return { from: to, to, text: "", sel: () => S(P(to.line, to.ch + close[1].length)) };
      }
      if (open && sEmpty(s)) {
        const next = t.slice(to.ch);
        const prev = t[to.ch - 1] ?? "";
        const quote = open[0] === open[1];
        const okNext = AUTO_CLOSE_BEFORE.test(next) && !(quote && next.startsWith(open[0]));
        const okPrev = !quote || (!wordRe.test(prev) && prev !== open[0] && prev !== "\\");
        if (okNext && okPrev) return { from, to, text: open[0] + open[1], sel: (c) => S(P(c.newTo.line, c.newTo.ch - open[1].length)) };
      }
      if (closer && sEmpty(s) && o.autoIndent !== false && /^[ \t]*$/.test(t.slice(0, to.ch))) {
        const m = matchBackward(buf, to, closer[0], closer[1]);
        if (m) {
          const ind = leadingWs(doc.line(m.line));
          return { from: P(to.line, 0), to, text: ind + ch };
        }
      }
      return { from, to, text: ch };
    });
    return buf.apply(items, "type");
  }

  /** Finds the unmatched `open` before p (skipping strings and comments). */
  function matchBackward(buf, p, open, close, maxLines = 3000) {
    let depth = 0;
    for (let line = p.line; line >= 0 && p.line - line < maxLines; line--) {
      const t = buf.doc.line(line);
      for (let i = line === p.line ? p.ch - 1 : t.length - 1; i >= 0; i--) {
        const c = t[i];
        if (c !== open && c !== close) continue;
        if (NON_CODE.has(buf.hl.typeAt(line, i))) continue;
        if (c === close) depth++;
        else if (depth === 0) return P(line, i);
        else depth--;
      }
    }
    return null;
  }

  function newline(buf, o) {
    const doc = buf.doc;
    const lang = o.lang || {};
    const unit = indentUnit(o);
    const items = buf.sels.map((s) => {
      let from = sFrom(s), to = sTo(s);
      if (o.autoIndent === false) return { from, to, text: "\n" };
      const t = doc.line(from.line);
      const before = t.slice(0, from.ch);
      let after = doc.line(to.line).slice(to.ch);
      const indent = leadingWs(t).slice(0, from.ch);
      const extra = lang.indentAfter && lang.indentAfter.test(before) ? unit : "";
      // Trim whitespace left behind on a line that only had indentation.
      if (/^[ \t]+$/.test(before) && !after.trim()) from = P(from.line, 0);
      const spaces = /^[ \t]*/.exec(after)[0].length;
      if (spaces && after.trim()) {
        to = P(to.line, to.ch + spaces);
        after = after.slice(spaces);
      }
      const prev = before.trimEnd().slice(-1), next = after[0];
      if (extra && next && (lang.brackets || []).some((b) => b[0] === prev && b[1] === next)) {
        const text = "\n" + indent + extra + "\n" + indent;
        return { from, to, text, sel: (c) => S(P(c.from.line + 1, (indent + extra).length)) };
      }
      return { from, to, text: "\n" + indent + extra };
    });
    return buf.apply(items, "newline");
  }

  function backspace(buf, o = {}) {
    const doc = buf.doc;
    const pairs = o.autoClose ? (o.lang && o.lang.autoClose) || [] : [];
    const items = buf.sels.map((s) => {
      if (!sEmpty(s)) return { from: sFrom(s), to: sTo(s), text: "" };
      const p = s.head;
      if (p.ch === 0) return { from: moveChar(doc, p, -1), to: p, text: "" };
      const t = doc.line(p.line);
      const before = t.slice(0, p.ch);
      if (o.insertSpaces && o.tabSize > 1 && /^ +$/.test(before)) {
        const n = ((before.length - 1) % o.tabSize) + 1;
        return { from: P(p.line, p.ch - n), to: p, text: "" };
      }
      if (pairs.some((pr) => t.startsWith(pr[1], p.ch) && before.endsWith(pr[0]) && pr[0].length === 1)) {
        return { from: P(p.line, p.ch - 1), to: P(p.line, p.ch + 1), text: "" };
      }
      return { from: moveChar(doc, p, -1), to: p, text: "" };
    });
    return buf.apply(items, "delback");
  }
  function deleteForward(buf) {
    return buf.apply(buf.sels.map((s) => (sEmpty(s) ? { from: s.head, to: moveChar(buf.doc, s.head, 1), text: "" } : { from: sFrom(s), to: sTo(s), text: "" })), "delfwd");
  }
  function deleteWord(buf, dir, wordRe) {
    return buf.apply(buf.sels.map((s) => {
      if (!sEmpty(s)) return { from: sFrom(s), to: sTo(s), text: "" };
      const q = moveWord(buf.doc, s.head, dir, wordRe);
      return { from: pmin(s.head, q), to: pmax(s.head, q), text: "" };
    }), "delword", { noMerge: true });
  }
  /** Deletes from each cursor to the start (dir < 0) or end of its line. */
  function deleteToLineEdge(buf, dir) {
    return buf.apply(buf.sels.map((s) => {
      if (!sEmpty(s)) return { from: sFrom(s), to: sTo(s), text: "" };
      const p = s.head, len = buf.doc.line(p.line).length;
      if (dir < 0) return p.ch === 0 ? { from: moveChar(buf.doc, p, -1), to: p, text: "" } : { from: P(p.line, 0), to: p, text: "" };
      return p.ch === len ? { from: p, to: moveChar(buf.doc, p, 1), text: "" } : { from: p, to: P(p.line, len), text: "" };
    }), "delline");
  }

  function deleteLines(buf) {
    const doc = buf.doc;
    const col = buf.head.ch;
    const items = mergedRanges(buf.sels).map(([a, b]) => {
      if (b < doc.size - 1) return { from: P(a, 0), to: P(b + 1, 0), text: "", sel: (c) => S(P(c.newTo.line, Math.min(col, doc.line(c.newTo.line).length))) };
      if (a > 0) return { from: P(a - 1, doc.line(a - 1).length), to: P(b, doc.line(b).length), text: "", sel: (c) => S(P(c.from.line, Math.min(col, doc.line(c.from.line).length))) };
      return { from: P(0, 0), to: doc.end(), text: "" };
    });
    return buf.apply(items, "deletelines");
  }

  function moveLines(buf, dir) {
    const doc = buf.doc;
    const ranges = mergedRanges(buf.sels);
    if (dir < 0 ? ranges[0][0] === 0 : ranges[ranges.length - 1][1] >= doc.size - 1) return false;
    const items = ranges.map(([a, b]) => {
      const block = doc.lines.slice(a, b + 1).join("\n");
      if (dir < 0) return { from: P(a - 1, 0), to: P(b, doc.line(b).length), text: block + "\n" + doc.line(a - 1) };
      return { from: P(a, 0), to: P(b + 1, doc.line(b + 1).length), text: doc.line(b + 1) + "\n" + block };
    });
    const sels = buf.sels.map((s) => S(P(s.anchor.line + dir, s.anchor.ch), P(s.head.line + dir, s.head.ch)));
    return buf.apply(items, "movelines", { sels, noMerge: true });
  }

  function copyLines(buf, dir) {
    const doc = buf.doc;
    const ranges = mergedRanges(buf.sels);
    const items = ranges.map(([a, b]) => {
      const block = doc.lines.slice(a, b + 1).join("\n");
      return dir > 0 ? { from: P(b, doc.line(b).length), to: P(b, doc.line(b).length), text: "\n" + block } : { from: P(a, 0), to: P(a, 0), text: block + "\n" };
    });
    // Selections land on the copies: below them when copying down, unchanged when copying up.
    const sels = buf.sels.map((s) => {
      const line = sFrom(s).line;
      let shift = 0;
      for (const [a, b] of ranges) {
        if (b < line || (dir > 0 && a <= line)) shift += b - a + 1;
      }
      return S(P(s.anchor.line + shift, s.anchor.ch), P(s.head.line + shift, s.head.ch));
    });
    return buf.apply(items, "copylines", { sels, noMerge: true });
  }

  /** Tab with a multi-line selection, Ctrl+] / Ctrl+[: indent or outdent every selected line. */
  function indentLines(buf, o, dir) {
    const doc = buf.doc;
    const unit = indentUnit(o);
    const items = [];
    for (const [a, b] of mergedRanges(buf.sels)) {
      for (let i = a; i <= b; i++) {
        const t = doc.line(i);
        if (dir > 0) {
          if (t.length || a === b) items.push({ from: P(i, 0), to: P(i, 0), text: unit });
        } else {
          const ws = leadingWs(t);
          let n = 0;
          if (ws[0] === "\t") n = 1;
          else while (n < ws.length && n < o.tabSize && ws[n] === " ") n++;
          if (n) items.push({ from: P(i, 0), to: P(i, n), text: "" });
        }
      }
    }
    if (!items.length) return false;
    return buf.apply(items, dir > 0 ? "indent" : "outdent", { mapSels: true, assoc: (p) => (p.ch === 0 && dir > 0 ? -1 : 1) });
  }

  /** Tab with ordinary cursors: insert to the next tab stop (spaces) or a tab. */
  function insertTab(buf, o) {
    const doc = buf.doc;
    return buf.apply(buf.sels.map((s) => {
      const from = sFrom(s), to = sTo(s);
      if (!o.insertSpaces) return { from, to, text: "\t" };
      const col = vwidth(doc.line(from.line), 0, from.ch, o.tabSize);
      return { from, to, text: " ".repeat(o.tabSize - (col % o.tabSize)) };
    }), "type");
  }

  function toggleLineComment(buf, lang) {
    const doc = buf.doc;
    const lc = lang.lineComment;
    if (!lc) return lang.blockComment ? toggleBlockComment(buf, lang, true) : false;
    const lines = [];
    for (const [a, b] of mergedRanges(buf.sels)) for (let i = a; i <= b; i++) lines.push(i);
    const code = lines.filter((i) => doc.line(i).trim());
    const target = code.length ? code : lines;
    const esc = lc.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const re = new RegExp("^([ \\t]*)" + esc + " ?");
    const all = target.every((i) => re.test(doc.line(i)));
    const items = [];
    if (all) {
      for (const i of target) {
        const m = re.exec(doc.line(i));
        items.push({ from: P(i, m[1].length), to: P(i, m[0].length), text: "" });
      }
    } else {
      const ind = Math.min(...target.map((i) => leadingWs(doc.line(i)).length));
      for (const i of target) items.push({ from: P(i, ind), to: P(i, ind), text: lc + " " });
    }
    return buf.apply(items, "comment", { mapSels: true, noMerge: true });
  }

  function toggleBlockComment(buf, lang, wholeLines = false) {
    const doc = buf.doc;
    const bc = lang.blockComment;
    if (!bc) return false;
    const [o, c] = bc;
    const items = [];
    const ranges = wholeLines
      ? mergedRanges(buf.sels).map(([a, b]) => [P(a, leadingWs(doc.line(a)).length), P(b, doc.line(b).length)])
      : buf.sels.map((s) => [sFrom(s), sTo(s)]);
    const unwrapAll = ranges.every(([f, t]) => {
      const text = doc.get(f, t).trim();
      return text.startsWith(o) && text.endsWith(c) && text.length >= o.length + c.length;
    });
    for (const [f, t] of ranges) {
      const text = doc.get(f, t);
      if (unwrapAll) {
        const lead = text.length - text.trimStart().length, trail = text.length - text.trimEnd().length;
        let inner = text.slice(lead + o.length, text.length - trail - c.length);
        if (inner.startsWith(" ") && inner.endsWith(" ") && inner.length >= 2) inner = inner.slice(1, -1);
        items.push({ from: f, to: t, text: text.slice(0, lead) + inner + text.slice(text.length - trail), sel: (ch) => S(ch.from, ch.newTo) });
      } else if (peq(f, t)) {
        items.push({ from: f, to: t, text: o + "  " + c, sel: (ch) => S(P(ch.from.line, ch.from.ch + o.length + 1)) });
      } else items.push({ from: f, to: t, text: o + " " + text + " " + c, sel: (ch) => S(ch.from, ch.newTo) });
    }
    return buf.apply(items, "comment", { noMerge: true });
  }

  /** Line-level transforms (sort, remove duplicates, trim…). With no selection they apply to the whole document. */
  function transformLines(buf, fn, kind = "lines") {
    const doc = buf.doc;
    const whole = buf.sels.length === 1 && (sEmpty(buf.main) || sFrom(buf.main).line === sTo(buf.main).line);
    const ranges = whole ? [[0, doc.size - 1]] : mergedRanges(buf.sels);
    const items = [];
    for (const [a, b] of ranges) {
      const src = doc.lines.slice(a, b + 1);
      const out = fn(src);
      if (out.length === src.length && out.every((l, i) => l === src[i])) continue;
      const text = out.join("\n");
      items.push({ from: P(a, 0), to: P(b, doc.line(b).length), text, sel: (c) => (whole ? S(P(clamp(buf.head.line, 0, c.newTo.line), 0)) : S(c.from, c.newTo)) });
    }
    if (!items.length) return false;
    return buf.apply(items, kind, { noMerge: true });
  }

  /** Text transforms per selection (case, encodings). Empty selections take the word at the cursor. */
  function transformText(buf, fn, wordRe, kind = "transform") {
    const doc = buf.doc;
    const items = [];
    for (const s of buf.sels) {
      let f = sFrom(s), t = sTo(s);
      if (sEmpty(s)) {
        const w = wordAt(doc.line(f.line), f.ch, wordRe);
        if (!w) continue;
        f = P(f.line, w[0]);
        t = P(f.line, w[1]);
      }
      const src = doc.get(f, t);
      let out;
      try { out = fn(src); } catch (e) { return { error: e.message || String(e) }; }
      if (out === src) continue;
      items.push({ from: f, to: t, text: out, sel: (c) => S(c.from, c.newTo) });
    }
    if (!items.length) return false;
    return buf.apply(items, kind, { noMerge: true });
  }

  function joinLines(buf) {
    const doc = buf.doc;
    const items = [];
    for (let [a, b] of mergedRanges(buf.sels)) {
      if (a === b) b = a + 1;
      if (b >= doc.size) continue;
      let text = doc.line(a).replace(/[ \t]+$/, "");
      for (let i = a + 1; i <= b; i++) {
        const next = doc.line(i).trim();
        if (next) text += (text && !/\s$/.test(text) ? " " : "") + next;
      }
      const at = doc.line(a).replace(/[ \t]+$/, "").length;
      items.push({ from: P(a, 0), to: P(b, doc.line(b).length), text, sel: () => S(P(a, at)) });
    }
    if (!items.length) return false;
    return buf.apply(items, "join", { noMerge: true });
  }

  function insertLine(buf, o, dir) {
    const doc = buf.doc;
    const lang = o.lang || {};
    const items = buf.sels.map((s) => {
      const line = dir > 0 ? sTo(s).line : sFrom(s).line;
      const t = doc.line(line);
      let ind = leadingWs(t);
      if (dir > 0) {
        if (lang.indentAfter && lang.indentAfter.test(t)) ind += indentUnit(o);
        return { from: P(line, t.length), to: P(line, t.length), text: "\n" + ind };
      }
      return { from: P(line, 0), to: P(line, 0), text: ind + "\n", sel: (c) => S(P(c.from.line, ind.length)) };
    });
    return buf.apply(items, "newline", { noMerge: true });
  }

  /** Ctrl+L: select whole lines, extending by a line when already whole. */
  function selectLines(buf) {
    const doc = buf.doc;
    buf.setSels(buf.sels.map((s) => {
      const f = sFrom(s), t = sTo(s);
      const end = t.line + 1;
      const head = end >= doc.size ? P(doc.size - 1, doc.line(doc.size - 1).length) : P(end, 0);
      return S(P(f.line, 0), head);
    }), buf.primary);
  }

  function addCursorVertical(buf, layout, dir) {
    const sorted = [...buf.sels].sort((a, b) => cmp(a.head, b.head));
    const edge = dir < 0 ? sorted[0] : sorted[sorted.length - 1];
    const m = moveVertical(layout, edge, dir);
    if (peq(m.pos, edge.head)) return false;
    buf.setSels([...buf.sels, S(m.pos, m.pos, m.goal, m.bias)], buf.sels.length);
    return true;
  }

  /** Ctrl+D. Returns the new selection, or null. */
  function addNextOccurrence(buf, { wholeWord = false, caseSensitive = true, wordRe } = {}) {
    const doc = buf.doc;
    const main = buf.main;
    if (sEmpty(main)) {
      const w = wordAt(doc.line(main.head.line), main.head.ch, wordRe);
      if (!w) return null;
      const s = S(P(main.head.line, w[0]), P(main.head.line, w[1]));
      const sels = [...buf.sels];
      sels[buf.primary] = s;
      buf.setSels(sels, buf.primary);
      return { sel: s, word: true };
    }
    const text = doc.get(sFrom(main), sTo(main));
    const cq = compileQuery(text, { regex: false, caseSensitive, wholeWord });
    if (!cq || cq.error) return null;
    const taken = new Set(buf.sels.map((s) => `${sFrom(s).line}:${sFrom(s).ch}`));
    const start = sTo(main);
    const hit = findNext(doc, cq, start, 1, (m) => !taken.has(`${m.from.line}:${m.from.ch}`));
    if (!hit) return null;
    const s = S(hit.from, hit.to);
    buf.setSels([...buf.sels, s], buf.sels.length);
    return { sel: s, word: false };
  }

  function selectAllOccurrences(buf, { wholeWord = false, caseSensitive = true, wordRe } = {}) {
    const doc = buf.doc;
    let main = buf.main;
    let word = wholeWord;
    if (sEmpty(main)) {
      const w = wordAt(doc.line(main.head.line), main.head.ch, wordRe);
      if (!w) return 0;
      main = S(P(main.head.line, w[0]), P(main.head.line, w[1]));
      word = true;
    }
    const text = doc.get(sFrom(main), sTo(main));
    const cq = compileQuery(text, { regex: false, caseSensitive, wholeWord: word });
    if (!cq || cq.error) return 0;
    const all = findAll(doc, cq, { limit: 50000 });
    if (!all.length) return 0;
    const at = all.findIndex((m) => peq(m.from, sFrom(main)));
    buf.setSels(all.map((m) => S(m.from, m.to)), Math.max(0, at));
    return all.length;
  }

  /** Column (box) selection between two visual columns on lines l1..l2. */
  function boxSels(doc, tab, l1, c1, l2, c2) {
    const out = [];
    const step = l2 >= l1 ? 1 : -1;
    for (let i = l1; ; i += step) {
      const t = doc.line(i);
      out.push(S(P(i, offsetAt(t, 0, t.length, c1, tab)), P(i, offsetAt(t, 0, t.length, c2, tab)), c2));
      if (i === l2) break;
    }
    return out;
  }

  // ------------------------------------------------------------------ search
  const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

  /** Builds the RegExp for a query. Returns null for an empty query or {error}. */
  function compileQuery(q, { regex = false, caseSensitive = false, wholeWord = false } = {}) {
    if (!q) return null;
    let src = regex ? q : escapeRe(q);
    let unicode = regex ? /\\[pP]\{|\\u\{/.test(src) : true;
    if (wholeWord) src = unicode ? `(?<![\\p{L}\\p{N}_])(?:${src})(?![\\p{L}\\p{N}_])` : `\\b(?:${src})\\b`;
    const flags = "gm" + (caseSensitive ? "" : "i") + (unicode ? "u" : "");
    const multiline = q.includes("\n") || (regex && /\\n|\\r/.test(q));
    try {
      return { re: new RegExp(src, flags), multiline, regex, src: q };
    } catch (e) {
      return { error: String(e.message || e).replace(/^Invalid regular expression: /, "") };
    }
  }

  /** Every match as {from, to, m}. `range` limits the search to [from, to). */
  function findAll(doc, cq, { limit = 20000, range = null } = {}) {
    const out = [];
    const re = cq.re;
    if (!cq.multiline) {
      const first = range ? range.from.line : 0, last = range ? range.to.line : doc.size - 1;
      for (let i = first; i <= last && out.length < limit; i++) {
        const t = doc.lines[i];
        const lo = range && i === range.from.line ? range.from.ch : 0;
        const hi = range && i === range.to.line ? range.to.ch : t.length;
        re.lastIndex = lo;
        let m;
        while ((m = re.exec(t))) {
          if (m.index + m[0].length > hi) break;
          out.push({ from: P(i, m.index), to: P(i, m.index + m[0].length), m: cq.regex ? m : null });
          if (!m[0].length) {
            if (re.lastIndex >= t.length) break;
            re.lastIndex++;
          }
          if (out.length >= limit) break;
        }
      }
      return out;
    }
    const text = doc.text();
    const starts = [0];
    for (let i = 0; i < doc.size - 1; i++) starts.push(starts[i] + doc.lines[i].length + 1);
    const toPos = (off) => {
      let lo = 0, hi = starts.length - 1;
      while (lo < hi) {
        const m = (lo + hi + 1) >> 1;
        if (starts[m] <= off) lo = m;
        else hi = m - 1;
      }
      return P(lo, off - starts[lo]);
    };
    const lo = range ? starts[range.from.line] + range.from.ch : 0;
    const hi = range ? starts[range.to.line] + range.to.ch : text.length;
    re.lastIndex = lo;
    let m;
    while ((m = re.exec(text)) && out.length < limit) {
      if (m.index + m[0].length > hi) break;
      out.push({ from: toPos(m.index), to: toPos(m.index + m[0].length), m: cq.regex ? m : null });
      if (!m[0].length) re.lastIndex++;
    }
    return out;
  }

  /** The next (dir > 0) or previous match from pos, wrapping around. */
  function findNext(doc, cq, pos, dir, accept = () => true) {
    const all = findAll(doc, cq, { limit: 200000 });
    if (!all.length) return null;
    if (dir > 0) {
      for (const m of all) if (cmp(m.from, pos) >= 0 && accept(m)) return m;
      for (const m of all) if (accept(m)) return m;
    } else {
      for (let i = all.length - 1; i >= 0; i--) if (cmp(all[i].to, pos) <= 0 && cmp(all[i].from, pos) < 0 && accept(all[i])) return all[i];
      for (let i = all.length - 1; i >= 0; i--) if (accept(all[i])) return all[i];
    }
    return null;
  }

  /** Regex replacement text: $0-$99, $&, $<name>, $$, \n, \t, and \U \L \E \u \l case changes. */
  function expandReplace(tpl, m) {
    if (!m) return tpl;
    let out = "", mode = null, next = null;
    const push = (s) => {
      if (!s) return;
      if (mode) s = mode === "U" ? s.toUpperCase() : s.toLowerCase();
      if (next) {
        s = (next === "u" ? s[0].toUpperCase() : s[0].toLowerCase()) + s.slice(1);
        next = null;
      }
      out += s;
    };
    for (let i = 0; i < tpl.length; i++) {
      const c = tpl[i];
      if (c === "\\" && i + 1 < tpl.length) {
        const d = tpl[++i];
        if (d === "n") push("\n");
        else if (d === "t") push("\t");
        else if (d === "\\") push("\\");
        else if (d === "U" || d === "L") mode = d;
        else if (d === "E") mode = null;
        else if (d === "u" || d === "l") next = d;
        else push("\\" + d);
      } else if (c === "$" && i + 1 < tpl.length) {
        const d = tpl[i + 1];
        if (d === "$") {
          push("$");
          i++;
        } else if (d === "&") {
          push(m[0]);
          i++;
        } else if (d >= "0" && d <= "9") {
          let num = d;
          const d2 = tpl[i + 2];
          if (d2 >= "0" && d2 <= "9" && Number(d + d2) < m.length) num = d + d2;
          if (Number(num) < m.length) {
            push(m[Number(num)] ?? "");
            i += num.length;
          } else push("$");
        } else if (d === "<") {
          const e = tpl.indexOf(">", i + 2);
          if (e > 0 && m.groups) {
            push(m.groups[tpl.slice(i + 2, e)] ?? "");
            i = e;
          } else push("$");
        } else push("$");
      } else push(c);
    }
    return out;
  }

  function replaceAll(buf, matches, tpl) {
    if (!matches.length) return 0;
    const items = matches.map((m) => ({ from: m.from, to: m.to, text: expandReplace(tpl, m.m) }));
    buf.apply(items, "replace", { mapSels: true, noMerge: true });
    return items.length;
  }

  // ------------------------------------------------------------------ brackets and folding
  const OPEN = { "(": ")", "[": "]", "{": "}" };
  const CLOSE = { ")": "(", "]": "[", "}": "{" };

  function scanBracket(buf, line, ch, dir, maxLines = 3000) {
    const doc = buf.doc;
    const start = doc.line(line)[ch];
    const want = dir > 0 ? OPEN[start] : CLOSE[start];
    let depth = 0;
    for (let l = line; l >= 0 && l < doc.size && Math.abs(l - line) < maxLines; l += dir) {
      const t = doc.line(l);
      if (t.length > LONG_LINE) continue;
      let i = l === line ? ch + dir : dir > 0 ? 0 : t.length - 1;
      for (; i >= 0 && i < t.length; i += dir) {
        const c = t[i];
        if (c !== start && c !== want) continue;
        if (NON_CODE.has(buf.hl.typeAt(l, i))) continue;
        if (c === start) depth++;
        else if (depth === 0) return P(l, i);
        else depth--;
      }
    }
    return null;
  }

  /** The bracket next to p and its partner, as {a, b}, or null. */
  function matchBracket(buf, p) {
    const t = buf.doc.line(p.line);
    if (t.length > LONG_LINE) return null;
    for (const ch of [p.ch - 1, p.ch]) {
      const c = t[ch];
      if (!c || !(OPEN[c] || CLOSE[c])) continue;
      if (NON_CODE.has(buf.hl.typeAt(p.line, ch))) continue;
      const other = scanBracket(buf, p.line, ch, OPEN[c] ? 1 : -1);
      return { a: P(p.line, ch), b: other };
    }
    return null;
  }

  const REGION_START = /^\s*(?:(?:\/\/|--|<!--|\/\*|;|'|::)\s*#?|#\s*|\{\s*#?)region\b/i;
  const REGION_END = /^\s*(?:(?:\/\/|--|<!--|\/\*|;|'|::)\s*#?|#\s*|\{\s*#?)end\s?region\b/i;
  const blank = (t) => !/\S/.test(t);

  /** The last opening bracket on a line that isn't closed on it (outside strings and comments). */
  function openBracketOn(buf, line) {
    const t = buf.doc.line(line);
    if (t.length > LONG_LINE) return -1;
    const stack = [];
    for (let i = 0; i < t.length; i++) {
      const c = t[i];
      if (!OPEN[c] && !CLOSE[c]) continue;
      if (NON_CODE.has(buf.hl.typeAt(line, i))) continue;
      if (OPEN[c]) stack.push(i);
      else if (stack.length && OPEN[t[stack[stack.length - 1]]] === c) stack.pop();
    }
    return stack.length ? stack[stack.length - 1] : -1;
  }

  function indentFold(doc, line, tab) {
    const t = doc.line(line);
    if (blank(t)) return null;
    const ind = indentWidth(t, tab);
    let end = line;
    for (let i = line + 1; i < doc.size; i++) {
      const u = doc.line(i);
      if (blank(u)) continue;
      if (indentWidth(u, tab) <= ind) break;
      end = i;
    }
    return end > line ? { start: line, end } : null;
  }

  /** Whether a line starts a foldable block (cheap; used for gutter markers). */
  function canFold(buf, line, mode, tab) {
    const doc = buf.doc;
    const t = doc.line(line);
    if (REGION_START.test(t)) return true;
    if (mode === "markdown") return /^#{1,6}\s/.test(t);
    if (mode === "brackets" && openBracketOn(buf, line) >= 0) return true;
    if (blank(t)) return false;
    for (let i = line + 1; i < doc.size && i < line + 200; i++) {
      const u = doc.line(i);
      if (blank(u)) continue;
      return indentWidth(u, tab) > indentWidth(t, tab) && (mode !== "brackets" || !/^\s*[)\]}]/.test(u));
    }
    return false;
  }

  /** The block a line starts: {start, end} where end is the last line to hide. */
  function foldRange(buf, line, mode, tab) {
    const doc = buf.doc;
    const t = doc.line(line);
    if (REGION_START.test(t)) {
      let depth = 0;
      for (let i = line + 1; i < doc.size; i++) {
        const u = doc.line(i);
        if (REGION_START.test(u)) depth++;
        else if (REGION_END.test(u)) {
          if (depth === 0) return i > line ? { start: line, end: i } : null;
          depth--;
        }
      }
      return null;
    }
    if (mode === "markdown") {
      const h = /^(#{1,6})\s/.exec(t);
      if (!h) return indentFold(doc, line, tab);
      let end = line;
      let fence = false;
      for (let i = line + 1; i < doc.size; i++) {
        const u = doc.line(i);
        if (/^\s*(```|~~~)/.test(u)) fence = !fence;
        const g = !fence && /^(#{1,6})\s/.exec(u);
        if (g && g[1].length <= h[1].length) break;
        if (!blank(u)) end = i;
      }
      return end > line ? { start: line, end } : null;
    }
    if (mode === "brackets") {
      const at = openBracketOn(buf, line);
      if (at >= 0) {
        const m = scanBracket(buf, line, at, 1, 50000);
        if (m && m.line > line) {
          const closeLine = doc.line(m.line);
          const end = /^[ \t]*$/.test(closeLine.slice(0, m.ch)) ? m.line - 1 : m.line;
          return end > line ? { start: line, end } : null;
        }
        return null;
      }
    }
    return indentFold(doc, line, tab);
  }

  /** Every foldable block at the given depth (0 = all), for Fold All / Fold Level N. */
  function allFolds(buf, mode, tab, level = 0) {
    const doc = buf.doc;
    const out = [];
    const stack = [];
    for (let i = 0; i < doc.size; i++) {
      while (stack.length && stack[stack.length - 1] < i) stack.pop();
      if (!canFold(buf, i, mode, tab)) continue;
      const r = foldRange(buf, i, mode, tab);
      if (!r) continue;
      const depth = stack.length + 1;
      if (!level || depth === level) out.push(r);
      stack.push(r.end);
    }
    return out;
  }

  // ------------------------------------------------------------------ indentation detection
  /** Guesses tabs vs spaces and the indent size from the first lines. */
  function detectIndent(doc, max = 10000) {
    let tabs = 0, spaces = 0, prev = 0;
    const deltas = new Map();
    const n = Math.min(doc.size, max);
    for (let i = 0; i < n; i++) {
      const t = doc.lines[i];
      const ws = leadingWs(t);
      if (ws.length === t.length) continue;
      if (ws[0] === "\t") {
        tabs++;
        prev = 0;
        continue;
      }
      if (ws.includes("\t")) continue;
      const w = ws.length;
      if (w) spaces++;
      const d = Math.abs(w - prev);
      if (d >= 2 && d <= 8) deltas.set(d, (deltas.get(d) || 0) + 1);
      prev = w;
    }
    if (!tabs && !spaces) return null;
    if (tabs > spaces) return { insertSpaces: false };
    let best = 4, bestN = 0;
    for (const size of [2, 4, 8, 3, 6]) {
      const k = deltas.get(size) || 0;
      if (k > bestN * 1.2 || (k && k === bestN && size === 4)) {
        best = size;
        bestN = k;
      }
    }
    return { insertSpaces: true, tabSize: best };
  }

  return {
    P, cmp, peq, pmin, pmax, clamp, S, sFrom, sTo, sEmpty,
    charWidth, vwidth, offsetAt, wrapBreaks, wordAt, leadingWs, indentWidth, isHigh, isLow, SIMPLE, DEFAULT_WORD, LONG_LINE,
    Doc, Buffer, Highlighter, Layout, mapPos, mapSel, mapLine, normalize, splice,
    moveChar, moveWord, moveVertical, homePos, endPos, lineRanges: mergedRanges,
    insertText, typeChar, newline, backspace, deleteForward, deleteWord, deleteToLineEdge, deleteLines, moveLines, copyLines,
    indentLines, insertTab, toggleLineComment, toggleBlockComment, transformLines, transformText, joinLines, insertLine,
    selectLines, addCursorVertical, addNextOccurrence, selectAllOccurrences, boxSels,
    escapeRe, compileQuery, findAll, findNext, expandReplace, replaceAll,
    matchBracket, matchBackward, canFold, foldRange, allFolds, detectIndent,
  };
})();
