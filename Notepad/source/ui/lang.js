"use strict";
// Axentra Notepad's syntax highlighting: language metadata, detection, and per-line
// tokenizers. One line goes in with the state left by the line above; tokens and the
// state to carry into the next line come out. No DOM: the tests run this under gjs.
//
// States are null (the default) or canonical strings, so the editor can compare them with
// === and stop re-tokenizing as soon as a line ends the way it did before.
globalThis.AxnLang = (() => {
  "use strict";

  const TYPES = [
    "comment", "string", "escape", "number", "keyword", "control", "type", "function", "constant",
    "variable", "property", "operator", "tag", "attr", "regexp", "meta", "heading", "strong",
    "emphasis", "link", "code", "quote", "list", "inserted", "deleted", "label", "section",
    "invalid", "error", "warning", "info", "debug", "col0", "col1", "col2", "col3", "col4", "col5",
  ];

  const words = (s) => new Set(s.split(/\s+/).filter(Boolean));
  const lowerWords = (s) => words(s.toLowerCase());
  const NONE = new Set();

  function push(out, s, e, type) {
    if (e > s) out.push([s, e, type]);
  }
  /** Shifts tokens from a sub-scan of text.slice(at) into `out`. */
  function append(out, tokens, at) {
    for (const t of tokens) out.push([t[0] + at, t[1] + at, t[2]]);
  }
  /** Colours the gaps between tokens in [s, e) with `type` (used for quotes and headings). */
  function fill(tokens, s, e, type) {
    const out = [];
    let at = s;
    for (const t of tokens) {
      if (t[0] > at) out.push([at, t[0], type]);
      out.push(t);
      at = t[1];
    }
    if (e > at) out.push([at, e, type]);
    return out;
  }

  const IDENT = /[\p{L}\p{Nl}_$][\p{L}\p{N}\p{Mn}\p{Mc}_$]*/uy;
  const NUMBER = /(?:0[xX][\da-fA-F_]+(?:\.[\da-fA-F_]*)?(?:[pP][+-]?\d+)?|0[bB][01_]+|0[oO][0-7_]+|(?:\d[\d_]*(?:\.(?![.\p{L}_])[\d_]*)?|\.\d[\d_]*)(?:[eE][+-]?\d[\d_]*)?)[\p{L}\d_]*/uy;
  const ESCAPE = /\\(?:x[0-9a-fA-F]{1,2}|u\{[0-9a-fA-F]*\}?|u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|[0-7]{1,3}|N\{[^}]*\}|[^]|$)/y;
  const isDigit = (c) => c >= 48 && c <= 57;
  const isUpper = (c) => c >= 65 && c <= 90;
  const OPS = new Set([..."+-*/%=&|^~<>!?:"].map((ch) => ch.charCodeAt(0)));
  const CLOSERS = new Set([41, 93, 125]); // ) ] }

  /** Scans string content from `i` up to and including `close`. Returns [end, closed]. */
  function stringBody(text, start, i, close, esc, out, dbl) {
    const n = text.length;
    const c0 = close.charCodeAt(0);
    let s = start;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (esc && c === 92) {
        ESCAPE.lastIndex = i;
        ESCAPE.test(text);
        const e = Math.max(ESCAPE.lastIndex, i + 1);
        push(out, s, i, "string");
        push(out, i, e, "escape");
        i = s = e;
        continue;
      }
      if (c === c0 && (close.length === 1 || text.startsWith(close, i))) {
        if (dbl && text.startsWith(close, i + close.length)) {
          i += close.length * 2;
          continue;
        }
        i += close.length;
        push(out, s, i, "string");
        return [i, true];
      }
      i++;
    }
    push(out, s, n, "string");
    return [n, false];
  }

  /** A block comment body from `i`; returns [end, depth left open (0 = closed)]. */
  function commentBody(text, start, i, open, close, depth, nest, out) {
    const n = text.length;
    for (;;) {
      const k = text.indexOf(close, i);
      const o = nest ? text.indexOf(open, i) : -1;
      if (o >= 0 && (k < 0 || o < k)) {
        depth++;
        i = o + open.length;
        continue;
      }
      if (k < 0) {
        push(out, start, n, "comment");
        return [n, depth];
      }
      i = k + close.length;
      if (--depth === 0) {
        push(out, start, i, "comment");
        return [i, 0];
      }
    }
  }

  /** A JS-style regex literal at `i` (a "/"); returns its end, or -1 if this "/" divides. */
  function regexLiteral(text, i) {
    const n = text.length;
    let j = i + 1;
    let cls = false;
    while (j < n) {
      const c = text.charCodeAt(j);
      if (c === 92) j += 2;
      else if (c === 91) { cls = true; j++; }
      else if (c === 93) { cls = false; j++; }
      else if (c === 47 && !cls) {
        j++;
        while (j < n && /[a-z]/.test(text[j])) j++;
        return j > i + 2 || text.charCodeAt(i + 1) !== 47 ? j : -1;
      } else j++;
    }
    return -1;
  }

  // ------------------------------------------------------------------ C-like scanner
  // Most programming languages share one scanner configured with word sets, comment and
  // string forms, and an optional `hook` for the syntax only that language has.
  //
  // State: "<template stack>|<mode>", or null. The stack holds "T" (inside a JS template
  // literal) and brace depths of the ${ } expressions inside them; the mode is an open
  // multi-line construct: c<block>:<depth> comment, s/S/d<close> string (escaped, raw,
  // doubled quotes), R<n> Rust raw string, V C# verbatim, X<delim> C++ raw string,
  // L<n>/M<n> Lua long string/comment, or a language's own (via cfg.resume).
  function clike(cfg) {
    const kw = cfg.keywords || NONE;
    const ctl = cfg.control || NONE;
    const types = cfg.types || NONE;
    const consts = cfg.constants || NONE;
    const builtins = cfg.builtins || NONE;
    const ci = !!cfg.caseInsensitive;
    const lineC = cfg.lineComment || [];
    const blocks = cfg.blockComment || [];
    const nest = !!cfg.nest;
    const strs = cfg.strings || { '"': { esc: true }, "'": { esc: true } };
    const triples = cfg.triple || [];
    const fnAfter = cfg.fnAfter || NONE;
    const typeAfter = cfg.typeAfter || NONE;
    const regexAfter = cfg.regexAfter || NONE;
    const ident = cfg.ident || IDENT;
    const lineFirst = new Set(lineC.map((p) => p.charCodeAt(0)));
    const blockFirst = new Set(blocks.map((b) => b[0].charCodeAt(0)));
    const tripleFirst = new Set(triples.map((t) => t.charCodeAt(0)));

    function resume(text, i, mode, out) {
      const m = mode[0];
      const rest = mode.slice(1);
      let r;
      switch (m) {
        case "c": {
          const k = rest.indexOf(":");
          const b = blocks[+rest.slice(0, k)];
          const d = commentBody(text, i, i, b[0], b[1], +rest.slice(k + 1), nest, out);
          return [d[0], d[1] ? `c${rest.slice(0, k)}:${d[1]}` : ""];
        }
        case "s": case "S": case "d":
          r = stringBody(text, i, i, rest, m === "s", out, m === "d");
          return [r[0], r[1] ? "" : mode];
        case "R":
          r = stringBody(text, i, i, '"' + "#".repeat(+rest), false, out, false);
          return [r[0], r[1] ? "" : mode];
        case "V":
          r = stringBody(text, i, i, '"', false, out, true);
          return [r[0], r[1] ? "" : mode];
        case "X":
          r = stringBody(text, i, i, ")" + rest + '"', false, out, false);
          return [r[0], r[1] ? "" : mode];
        case "L": case "M": {
          const close = "]" + "=".repeat(+rest) + "]";
          const k = text.indexOf(close, i);
          const e = k < 0 ? text.length : k + close.length;
          push(out, i, e, m === "L" ? "string" : "comment");
          return [e, k < 0 ? mode : ""];
        }
        default:
          return cfg.resume ? cfg.resume(text, i, mode, out) : [text.length, ""];
      }
    }

    return function tokenize(text, state) {
      const out = [];
      const n = text.length;
      let i = 0;
      let mode = "";
      let stack = null;
      if (state) {
        const k = state.indexOf("|");
        if (k > 0) stack = state.slice(0, k).split(",");
        mode = state.slice(k + 1);
      }
      let first = 0;
      while (first < n && (text.charCodeAt(first) === 32 || text.charCodeAt(first) === 9)) first++;
      // The scanner object that hooks read and update.
      const S = { prev: 0, mode: "", first, pendingFn: false, pendingType: false };
      let tplFrom = -1; // where the string token of a just-opened template starts
      while (i < n) {
        if (mode) {
          [i, mode] = resume(text, i, mode, out);
          if (mode) break;
          S.prev = 1;
          continue;
        }
        const top = stack && stack.length ? stack[stack.length - 1] : null;
        if (top === "T") {
          // Template literal text up to the closing backtick or the next ${.
          let j = i;
          let s = tplFrom >= 0 ? tplFrom : i;
          tplFrom = -1;
          let ev = 0;
          while (j < n) {
            const c = text.charCodeAt(j);
            if (c === 92) {
              ESCAPE.lastIndex = j;
              ESCAPE.test(text);
              const e = Math.max(ESCAPE.lastIndex, j + 1);
              push(out, s, j, "string");
              push(out, j, e, "escape");
              j = s = e;
            } else if (c === 96) {
              j++;
              push(out, s, j, "string");
              ev = 1;
              break;
            } else if (c === 36 && text.charCodeAt(j + 1) === 123) {
              push(out, s, j, "string");
              push(out, j, j + 2, "keyword");
              j += 2;
              ev = 2;
              break;
            } else j++;
          }
          if (!ev) push(out, s, n, "string");
          i = j;
          if (ev === 1) { stack.pop(); S.prev = 1; }
          else if (ev === 2) { stack.push("0"); S.prev = 0; }
          continue;
        }
        const c = text.charCodeAt(i);
        if (c === 32 || c === 9) { i++; continue; }
        if (cfg.hook) {
          S.mode = "";
          const e = cfg.hook(text, i, c, out, S);
          if (e >= 0) {
            i = e;
            if (S.mode) { mode = S.mode; if (i >= n) break; }
            continue;
          }
        }
        if (lineFirst.has(c) && lineC.some((p) => text.startsWith(p, i))) {
          push(out, i, n, "comment");
          i = n;
          break;
        }
        if (blockFirst.has(c)) {
          const b = blocks.findIndex((bc) => text.startsWith(bc[0], i));
          if (b >= 0) {
            const d = commentBody(text, i, i + blocks[b][0].length, blocks[b][0], blocks[b][1], 1, nest, out);
            i = d[0];
            if (d[1]) { mode = `c${b}:${d[1]}`; break; }
            continue;
          }
        }
        if (tripleFirst.has(c)) {
          const t = triples.find((q) => text.startsWith(q, i));
          if (t) {
            const raw = cfg.rawTriple;
            const r = stringBody(text, i, i + t.length, t, !raw, out, false);
            i = r[0];
            S.prev = 1;
            if (!r[1]) { mode = (raw ? "S" : "s") + t; break; }
            continue;
          }
        }
        if (cfg.templates && c === 96) {
          (stack || (stack = [])).push("T");
          tplFrom = i;
          i++;
          continue;
        }
        const quote = text[i];
        const q = strs[quote];
        if (q) {
          const r = stringBody(text, i, i + 1, quote, q.esc, out, q.dbl);
          i = r[0];
          S.prev = 1;
          if (!r[1] && q.multi) { mode = (q.esc ? "s" : q.dbl ? "d" : "S") + quote; break; }
          continue;
        }
        if (isDigit(c) || (c === 46 && isDigit(text.charCodeAt(i + 1)) && S.prev !== 1)) {
          NUMBER.lastIndex = i;
          NUMBER.test(text);
          const e = Math.max(NUMBER.lastIndex, i + 1);
          push(out, i, e, "number");
          i = e;
          S.prev = 1;
          continue;
        }
        ident.lastIndex = i;
        if (c !== 36 || cfg.dollarIdent !== false) {
          if (ident.test(text)) {
            let e = ident.lastIndex;
            const w = text.slice(i, e);
            const key = ci ? w.toLowerCase() : w;
            let type = null;
            S.prev = 1;
            if (ctl.has(key)) { type = "control"; if (regexAfter.has(key)) S.prev = 0; }
            else if (kw.has(key)) { type = "keyword"; if (regexAfter.has(key)) S.prev = 0; }
            else if (consts.has(key)) type = "constant";
            else if (S.pendingFn) type = "function";
            else if (S.pendingType) type = "type";
            else if (cfg.macros && text.charCodeAt(e) === 33 && text.charCodeAt(e + 1) !== 61) { type = "meta"; e++; }
            else {
              let j = e;
              while (text.charCodeAt(j) === 32) j++;
              if (text.charCodeAt(j) === 40 && !types.has(key) && cfg.calls !== false) type = "function";
              else if (types.has(key)) type = "type";
              else if (builtins.has(key)) type = "function";
              else if (cfg.capsConstants !== false && w.length > 1 && isUpper(w.charCodeAt(0)) && w === w.toUpperCase() && /^[A-Z][A-Z0-9_]+$/.test(w)) type = "constant";
              else if (cfg.pascalTypes && isUpper(w.charCodeAt(0)) && (cfg.pascalTypes === "any" || /[a-z]/.test(w))) type = "type";
            }
            S.pendingFn = fnAfter.has(key);
            S.pendingType = typeAfter.has(key);
            if (type) push(out, i, e, type);
            i = e;
            continue;
          }
        }
        S.pendingFn = S.pendingType = false;
        if (c === 47 && cfg.regex && S.prev === 0) {
          const e = regexLiteral(text, i);
          if (e > 0) {
            push(out, i, e, "regexp");
            i = e;
            S.prev = 1;
            continue;
          }
        }
        if (stack && stack.length && (c === 123 || c === 125)) {
          const d = +stack[stack.length - 1];
          if (c === 123) stack[stack.length - 1] = String(d + 1);
          else if (d === 0) { stack.pop(); push(out, i, i + 1, "keyword"); i++; S.prev = 1; continue; }
          else stack[stack.length - 1] = String(d - 1);
          i++;
          S.prev = c === 125 ? 1 : 0;
          continue;
        }
        if (OPS.has(c)) {
          let e = i + 1;
          while (e < n && OPS.has(text.charCodeAt(e)) && !(text.charCodeAt(e) === 47 && (text.charCodeAt(e + 1) === 47 || text.charCodeAt(e + 1) === 42))) e++;
          push(out, i, e, "operator");
          i = e;
          S.prev = 0;
          continue;
        }
        S.prev = CLOSERS.has(c) ? 1 : c === 46 ? 2 : 0;
        i++;
      }
      const st = stack && stack.length ? stack.join(",") : "";
      return { tokens: out, state: st || mode ? `${st}|${mode}` : null };
    };
  }

  // ------------------------------------------------------------------ hooks
  const DECORATOR = /@[\p{L}_][\p{L}\p{N}_.]*/uy;
  function annotation(text, i, c, out, S) {
    if (c !== 64) return -1;
    DECORATOR.lastIndex = i;
    if (!DECORATOR.test(text)) return -1;
    push(out, i, DECORATOR.lastIndex, "meta");
    S.prev = 1;
    return DECORATOR.lastIndex;
  }
  const HASH_WORD = /#[A-Za-z_]\w*/y;
  function hashMeta(text, i, c, out) {
    if (c !== 35) return -1;
    HASH_WORD.lastIndex = i;
    if (!HASH_WORD.test(text)) return -1;
    push(out, i, HASH_WORD.lastIndex, "meta");
    return HASH_WORD.lastIndex;
  }

  const PP = /#\s*([A-Za-z_]\w*)?/y;
  const PP_INCLUDE = /\s*(<[^>]*>?|"[^"]*"?)/y;
  function preprocessor(text, i, out) {
    PP.lastIndex = i;
    const m = PP.exec(text);
    let e = PP.lastIndex;
    const d = m[1] || "";
    if (/^(error|warning|pragma|region|endregion|line|nullable|message)$/.test(d)) e = text.length;
    push(out, i, e, "meta");
    if (d === "include" || d === "import" || d === "include_next") {
      PP_INCLUDE.lastIndex = e;
      const mm = PP_INCLUDE.exec(text);
      if (mm) {
        push(out, PP_INCLUDE.lastIndex - mm[1].length, PP_INCLUDE.lastIndex, "string");
        e = PP_INCLUDE.lastIndex;
      }
    }
    return e;
  }

  const CPP_RAW = /(?:u8|[uUL])?R"([^()\\\s"]{0,16})\(/y;
  const C_PREFIX = /(?:u8|[uUL])(?=["'])/y;
  function cHook(text, i, c, out, S) {
    if (c === 35 && i === S.first) return preprocessor(text, i, out);
    if (c === 82 || c === 117 || c === 85 || c === 76) {
      CPP_RAW.lastIndex = i;
      const m = CPP_RAW.exec(text);
      if (m) {
        const r = stringBody(text, i, CPP_RAW.lastIndex, ")" + m[1] + '"', false, out, false);
        if (!r[1]) S.mode = "X" + m[1];
        S.prev = 1;
        return r[0];
      }
      C_PREFIX.lastIndex = i;
      if (C_PREFIX.test(text)) {
        const q = text[C_PREFIX.lastIndex];
        S.prev = 1;
        return stringBody(text, i, C_PREFIX.lastIndex + 1, q, true, out, false)[0];
      }
    }
    return -1;
  }
  function objcHook(text, i, c, out, S) {
    if (c === 64 && text.charCodeAt(i + 1) === 34) {
      S.prev = 1;
      return stringBody(text, i, i + 2, '"', true, out, false)[0];
    }
    const a = annotation(text, i, c, out, S);
    return a >= 0 ? a : cHook(text, i, c, out, S);
  }

  function csHook(text, i, c, out, S) {
    if (c === 35 && i === S.first) return preprocessor(text, i, out);
    if (c !== 64 && c !== 36) return -1;
    let j = i;
    let verbatim = false;
    while (text.charCodeAt(j) === 36 || text.charCodeAt(j) === 64) {
      if (text.charCodeAt(j) === 64) verbatim = true;
      j++;
    }
    if (text.charCodeAt(j) !== 34) return -1;
    S.prev = 1;
    if (text.startsWith('"""', j)) {
      const r = stringBody(text, i, j + 3, '"""', false, out, false);
      if (!r[1]) S.mode = 'S"""';
      return r[0];
    }
    if (verbatim) {
      const r = stringBody(text, i, j + 1, '"', false, out, true);
      if (!r[1]) S.mode = "V";
      return r[0];
    }
    return stringBody(text, i, j + 1, '"', true, out, false)[0];
  }

  const RUST_RAW = /b?r(#*)"/y;
  const RUST_BSTR = /[bc]"/y;
  const RUST_CHAR = /b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|[^])|[^\\'])'/y;
  const LIFETIME = /'[\p{L}_][\p{L}\p{N}_]*/uy;
  function rustHook(text, i, c, out, S) {
    if (c === 35 && (text.charCodeAt(i + 1) === 91 || (text.charCodeAt(i + 1) === 33 && text.charCodeAt(i + 2) === 91))) {
      let depth = 0;
      let j = i;
      for (; j < text.length; j++) {
        const ch = text.charCodeAt(j);
        if (ch === 91) depth++;
        else if (ch === 93 && --depth === 0) { j++; break; }
        else if (ch === 34) j = stringBody(text, j, j + 1, '"', true, [], false)[0] - 1;
      }
      push(out, i, j, "meta");
      return j;
    }
    if (c === 114 || c === 98 || c === 99) {
      RUST_RAW.lastIndex = i;
      const m = RUST_RAW.exec(text);
      if (m) {
        const r = stringBody(text, i, RUST_RAW.lastIndex, '"' + m[1], false, out, false);
        if (!r[1]) S.mode = "R" + m[1].length;
        S.prev = 1;
        return r[0];
      }
      RUST_BSTR.lastIndex = i;
      if (RUST_BSTR.test(text)) {
        const r = stringBody(text, i, i + 2, '"', true, out, false);
        if (!r[1]) S.mode = 's"';
        S.prev = 1;
        return r[0];
      }
    }
    if (c === 39 || (c === 98 && text.charCodeAt(i + 1) === 39)) {
      RUST_CHAR.lastIndex = i;
      if (RUST_CHAR.test(text)) {
        S.prev = 1;
        return stringBody(text, i, i + (c === 98 ? 2 : 1), "'", true, out, false)[0];
      }
      if (c === 39) {
        LIFETIME.lastIndex = i;
        if (LIFETIME.test(text)) {
          push(out, i, LIFETIME.lastIndex, "keyword");
          S.prev = 1;
          return LIFETIME.lastIndex;
        }
      }
    }
    return -1;
  }

  const PY_PREFIX = /([rRbBuUfF]{1,2})(?=["'])/y;
  function pyHook(text, i, c, out, S) {
    if (c === 64 && i === S.first) return annotation(text, i, c, out, S);
    if ((c >= 65 && c <= 90) || (c >= 97 && c <= 122)) {
      PY_PREFIX.lastIndex = i;
      const m = PY_PREFIX.exec(text);
      if (m && /^(r|u|b|f|br|rb|fr|rf)$/i.test(m[1])) {
        const raw = /r/i.test(m[1]);
        const j = PY_PREFIX.lastIndex;
        const q3 = text.substr(j, 3);
        S.prev = 1;
        if (q3 === '"""' || q3 === "'''") {
          const r = stringBody(text, i, j + 3, q3, !raw, out, false);
          if (!r[1]) S.mode = (raw ? "S" : "s") + q3;
          return r[0];
        }
        return stringBody(text, i, j + 1, text[j], !raw, out, false)[0];
      }
    }
    return -1;
  }

  const LUA_LONG = /\[(=*)\[/y;
  function luaLong(text, i, at, out, S, type) {
    LUA_LONG.lastIndex = at;
    const m = LUA_LONG.exec(text);
    if (!m) return -1;
    const close = "]" + m[1] + "]";
    const k = text.indexOf(close, LUA_LONG.lastIndex);
    const e = k < 0 ? text.length : k + close.length;
    push(out, i, e, type);
    if (k < 0) S.mode = (type === "string" ? "L" : "M") + m[1].length;
    return e;
  }
  function luaHook(text, i, c, out, S) {
    if (c === 45 && text.charCodeAt(i + 1) === 45) {
      const e = luaLong(text, i, i + 2, out, S, "comment");
      if (e >= 0) return e;
      push(out, i, text.length, "comment");
      return text.length;
    }
    if (c === 91) {
      const e = luaLong(text, i, i, out, S, "string");
      if (e >= 0) S.prev = 1;
      return e;
    }
    return -1;
  }

  // Ruby =begin/=end and Perl POD blocks: mode "B<end marker>".
  function blockDocResume(text, i, mode, out) {
    push(out, i, text.length, "comment");
    return [text.length, text.startsWith(mode.slice(1)) ? "" : mode];
  }
  const RUBY_VAR = /(?:@@?|\$)[\p{L}_][\p{L}\p{N}_]*|\$[0-9!@&`'+~=\/\\,;.<>_*$?:"]/uy;
  const RUBY_SYM = /:[\p{L}_][\p{L}\p{N}_]*[?!=]?/uy;
  function rubyHook(text, i, c, out, S) {
    if (c === 61 && i === 0 && text.startsWith("=begin")) {
      push(out, 0, text.length, "comment");
      S.mode = "B=end";
      return text.length;
    }
    if (c === 64 || c === 36) {
      RUBY_VAR.lastIndex = i;
      if (RUBY_VAR.test(text)) {
        push(out, i, RUBY_VAR.lastIndex, "variable");
        S.prev = 1;
        return RUBY_VAR.lastIndex;
      }
    }
    if (c === 58 && text.charCodeAt(i + 1) !== 58 && text.charCodeAt(i - 1) !== 58) {
      RUBY_SYM.lastIndex = i;
      if (RUBY_SYM.test(text)) {
        push(out, i, RUBY_SYM.lastIndex, "constant");
        S.prev = 1;
        return RUBY_SYM.lastIndex;
      }
    }
    return -1;
  }
  const PERL_VAR = /[$@%]#?(?:[\p{L}_][\p{L}\p{N}_]*(?:::[\p{L}_][\p{L}\p{N}_]*)*|\{[^}]*\}|[0-9]+|[_&`'+!@\/\\,;.0<>\[\]^$])/uy;
  function perlHook(text, i, c, out, S) {
    if (c === 61 && i === 0 && /^=[a-z]/.test(text)) {
      push(out, 0, text.length, "comment");
      if (!text.startsWith("=cut")) S.mode = "B=cut";
      return text.length;
    }
    if (c === 36 || c === 64 || (c === 37 && S.prev !== 1)) {
      PERL_VAR.lastIndex = i;
      if (PERL_VAR.test(text)) {
        push(out, i, PERL_VAR.lastIndex, "variable");
        S.prev = 1;
        return PERL_VAR.lastIndex;
      }
    }
    return -1;
  }
  const PHP_VAR = /\$+[\p{L}_][\p{L}\p{N}_]*/uy;
  function phpHook(text, i, c, out, S) {
    if (c === 36) {
      PHP_VAR.lastIndex = i;
      if (PHP_VAR.test(text)) {
        push(out, i, PHP_VAR.lastIndex, "variable");
        S.prev = 1;
        return PHP_VAR.lastIndex;
      }
    }
    if (c === 35 && text.charCodeAt(i + 1) === 91) {
      const k = text.indexOf("]", i);
      const e = k < 0 ? text.length : k + 1;
      push(out, i, e, "meta");
      return e;
    }
    return -1;
  }
  function vbHook(text, i, c, out, S) {
    if ((c === 82 || c === 114) && /^rem\b/i.test(text.slice(i, i + 4)) && (i === S.first || text.charCodeAt(i - 1) === 58)) {
      push(out, i, text.length, "comment");
      return text.length;
    }
    return i === S.first ? hashMeta(text, i, c, out) : -1;
  }
  const SQL_QUOTED = /"(?:[^"]|"")*"?|`[^`]*`?|\[[\p{L}_][^\]]*\]/uy;
  const SQL_PARAM = /(?:@@?|:|\$)[\p{L}\p{N}_]+/uy;
  function sqlHook(text, i, c, out, S) {
    if (c === 34 || c === 96 || c === 91) {
      SQL_QUOTED.lastIndex = i;
      if (SQL_QUOTED.test(text)) {
        push(out, i, SQL_QUOTED.lastIndex, "variable");
        S.prev = 1;
        return SQL_QUOTED.lastIndex;
      }
    }
    if (c === 64 || (c === 58 && text.charCodeAt(i + 1) !== 58) || c === 36) {
      SQL_PARAM.lastIndex = i;
      if (SQL_PARAM.test(text)) {
        push(out, i, SQL_PARAM.lastIndex, "variable");
        S.prev = 1;
        return SQL_PARAM.lastIndex;
      }
    }
    return -1;
  }
  const ASM_LABEL = /[\p{L}_.$@?][\p{L}\p{N}_.$@?]*:/uy;
  const ASM_DIRECTIVE = /[.%][\p{L}_][\p{L}\p{N}_]*/uy;
  function asmHook(text, i, c, out, S) {
    if (i === S.first) {
      ASM_LABEL.lastIndex = i;
      if (ASM_LABEL.test(text)) {
        push(out, i, ASM_LABEL.lastIndex, "function");
        return ASM_LABEL.lastIndex;
      }
    }
    if (c === 46 || c === 37) {
      ASM_DIRECTIVE.lastIndex = i;
      if (ASM_DIRECTIVE.test(text)) {
        push(out, i, ASM_DIRECTIVE.lastIndex, c === 37 && i !== S.first ? "variable" : "meta");
        return ASM_DIRECTIVE.lastIndex;
      }
    }
    if (c === 36 && isDigit(text.charCodeAt(i + 1))) {
      NUMBER.lastIndex = i + 1;
      NUMBER.test(text);
      push(out, i, NUMBER.lastIndex, "number");
      return NUMBER.lastIndex;
    }
    return -1;
  }
  const chain = (...hooks) => (text, i, c, out, S) => {
    for (const h of hooks) {
      const e = h(text, i, c, out, S);
      if (e >= 0) return e;
    }
    return -1;
  };

  // ------------------------------------------------------------------ C-like languages
  const C_KW = "auto const extern register static volatile inline restrict typedef struct union enum sizeof _Alignas _Alignof _Atomic _Bool _Complex _Generic _Imaginary _Noreturn _Static_assert _Thread_local alignas alignof static_assert thread_local typeof constexpr";
  const C_CTL = "if else for while do return break continue switch case default goto";
  const C_TYPES = "void char short int long float double signed unsigned bool size_t ssize_t ptrdiff_t intptr_t uintptr_t int8_t int16_t int32_t int64_t uint8_t uint16_t uint32_t uint64_t wchar_t char8_t char16_t char32_t FILE va_list";
  const C_CONST = "true false NULL nullptr EOF stdin stdout stderr";
  const JS_KW = "var let const function class extends new delete typeof instanceof void in of this super static get set async with debugger enum";
  const JS_CTL = "if else for while do return break continue switch case default try catch finally throw yield await import export from as";
  const JS_TYPES = "Array Object String Number Boolean Symbol BigInt Map Set WeakMap WeakSet WeakRef Promise Date RegExp Error TypeError RangeError SyntaxError ReferenceError EvalError URIError AggregateError JSON Math Reflect Proxy Intl ArrayBuffer SharedArrayBuffer DataView Uint8Array Uint8ClampedArray Int8Array Uint16Array Int16Array Uint32Array Int32Array Float32Array Float64Array BigInt64Array BigUint64Array Function";
  const JS_REGEX_AFTER = words("return typeof instanceof in of new delete void throw case do else yield await");
  const TS_KW = JS_KW + " type interface namespace module declare abstract implements private protected public readonly keyof infer is satisfies override accessor asserts unique";
  const TS_TYPES = JS_TYPES + " string number boolean any unknown never object symbol bigint";

  const T = {};
  T.c = clike({
    keywords: words(C_KW), control: words(C_CTL), types: words(C_TYPES), constants: words(C_CONST),
    lineComment: ["//"], blockComment: [["/*", "*/"]], typeAfter: words("struct union enum"), hook: cHook,
  });
  T.cpp = clike({
    keywords: words(C_KW + " class namespace template typename using virtual override final public private protected friend operator explicit mutable this new delete consteval constinit decltype noexcept concept requires co_await co_return co_yield export import module static_cast dynamic_cast reinterpret_cast const_cast typeid"),
    control: words(C_CTL + " try catch throw"),
    types: words(C_TYPES + " string wstring string_view vector array map unordered_map set unordered_set list deque pair tuple optional variant unique_ptr shared_ptr weak_ptr function"),
    constants: words(C_CONST), lineComment: ["//"], blockComment: [["/*", "*/"]],
    typeAfter: words("struct union enum class typename"), pascalTypes: true, hook: cHook,
  });
  T["objective-c"] = clike({
    keywords: words(C_KW + " self super id instancetype nonatomic atomic strong weak copy assign readonly readwrite retain unsafe_unretained nullable nonnull class"),
    control: words(C_CTL + " try catch throw finally"), types: words(C_TYPES + " BOOL SEL IMP Class NSInteger NSUInteger CGFloat"),
    constants: words(C_CONST + " nil Nil YES NO"), lineComment: ["//"], blockComment: [["/*", "*/"]], pascalTypes: true, hook: objcHook,
  });
  T.csharp = clike({
    keywords: words("abstract as base class const delegate enum event explicit extern fixed implicit in interface internal is lock namespace new operator out override params private protected public readonly ref sealed sizeof stackalloc static struct this typeof unchecked checked unsafe using virtual volatile async get set init value var dynamic record partial where with nameof global required file scoped add remove alias ascending descending by equals group into join let on orderby select"),
    control: words("if else for foreach while do return break continue switch case default goto try catch finally throw yield when await"),
    types: words("bool byte sbyte char decimal double float int uint long ulong short ushort object string void nint nuint"),
    constants: words("true false null"), lineComment: ["//"], blockComment: [["/*", "*/"]], triple: ['"""'], rawTriple: true,
    typeAfter: words("class struct interface enum record new"), pascalTypes: true, hook: csHook,
  });
  T.java = clike({
    keywords: words("abstract class extends implements interface enum final native private protected public static strictfp synchronized transient volatile new this super instanceof package import var record sealed permits module requires exports opens uses provides with to transitive throws assert"),
    control: words("if else for while do return break continue switch case default try catch finally throw yield"),
    types: words("boolean byte char short int long float double void"), constants: words("true false null"),
    lineComment: ["//"], blockComment: [["/*", "*/"]], triple: ['"""'], typeAfter: words("class interface enum record extends implements new"),
    pascalTypes: true, hook: annotation,
  });
  T.kotlin = clike({
    keywords: words("val var fun class interface object enum sealed data inline value open abstract override private protected public internal companion lateinit const suspend operator infix tailrec external annotation import package typealias constructor init this super by where in out is as reified crossinline noinline vararg get set field it"),
    control: words("if else when for while do return break continue try catch finally throw"),
    types: words("Int Long Short Byte Double Float Boolean Char String Unit Any Nothing Array List MutableList Map MutableMap Set MutableSet"),
    constants: words("true false null"), lineComment: ["//"], blockComment: [["/*", "*/"]], nest: true, triple: ['"""'], rawTriple: true,
    fnAfter: words("fun"), typeAfter: words("class interface object typealias"), pascalTypes: true, hook: annotation,
  });
  T.scala = clike({
    keywords: words("abstract case class def extends final implicit lazy object override package private protected sealed super this trait type val var with import new given using extension enum export opaque inline transparent derives then end"),
    control: words("if else for while do return match try catch finally throw yield"),
    types: words("Int Long Short Byte Double Float Boolean Char String Unit Any AnyRef AnyVal Nothing Option Some List Seq Map Set Vector Array Either"),
    constants: words("true false null None Nil"), lineComment: ["//"], blockComment: [["/*", "*/"]], nest: true, triple: ['"""'], rawTriple: true,
    fnAfter: words("def"), typeAfter: words("class trait object type extends with new"), pascalTypes: true, hook: annotation,
  });
  T.swift = clike({
    keywords: words("associatedtype class deinit enum extension fileprivate func import init inout internal let open operator private precedencegroup protocol public rethrows static struct subscript typealias var actor async nonisolated isolated some any mutating nonmutating override final required convenience dynamic lazy optional weak unowned indirect self Self super throws is as in where get set willSet didSet"),
    control: words("if else guard for while repeat return break continue fallthrough switch case default do try catch throw defer await"),
    types: words("Int Int8 Int16 Int32 Int64 UInt UInt8 UInt16 UInt32 UInt64 Double Float Bool String Character Void Any AnyObject Array Dictionary Set Optional Never"),
    constants: words("true false nil"), lineComment: ["//"], blockComment: [["/*", "*/"]], nest: true, triple: ['"""'],
    fnAfter: words("func"), typeAfter: words("class struct enum protocol extension typealias actor"), pascalTypes: true, hook: chain(annotation, hashMeta),
  });
  T.go = clike({
    keywords: words("package import func var const type struct interface map chan"),
    control: words("if else for switch case default return break continue goto fallthrough select go defer range"),
    types: words("bool byte rune string int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 complex64 complex128 error any comparable"),
    constants: words("true false nil iota"), builtins: words("append cap clear close complex copy delete imag len make max min new panic print println real recover"),
    lineComment: ["//"], blockComment: [["/*", "*/"]], strings: { '"': { esc: true }, "'": { esc: true }, "`": { esc: false, multi: true } },
    fnAfter: words("func"), typeAfter: words("type"),
  });
  T.rust = clike({
    keywords: words("as async const crate dyn enum extern fn impl let mod move mut pub ref self Self static struct super trait type union unsafe use where macro_rules"),
    control: words("if else for while loop match return break continue yield await in"),
    types: words("bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64 String Vec Option Result Box Rc Arc RefCell Cell HashMap HashSet BTreeMap BTreeSet VecDeque"),
    constants: words("true false None Some Ok Err"), lineComment: ["//"], blockComment: [["/*", "*/"]], nest: true,
    strings: { '"': { esc: true, multi: true } }, fnAfter: words("fn"), typeAfter: words("struct enum trait type union"),
    macros: true, pascalTypes: true, hook: rustHook,
  });
  T.javascript = clike({
    keywords: words(JS_KW), control: words(JS_CTL), types: words(JS_TYPES), constants: words("true false null undefined NaN Infinity"),
    lineComment: ["//"], blockComment: [["/*", "*/"]], templates: true, regex: true, regexAfter: JS_REGEX_AFTER,
    fnAfter: words("function"), typeAfter: words("class extends new"), pascalTypes: true, hook: annotation,
  });
  T.typescript = clike({
    keywords: words(TS_KW), control: words(JS_CTL), types: words(TS_TYPES), constants: words("true false null undefined NaN Infinity"),
    lineComment: ["//"], blockComment: [["/*", "*/"]], templates: true, regex: true, regexAfter: JS_REGEX_AFTER,
    fnAfter: words("function"), typeAfter: words("class extends new interface implements enum namespace"), pascalTypes: true, hook: annotation,
  });
  T.dart = clike({
    keywords: words("abstract as async base class const covariant deferred dynamic enum export extends extension external factory final get hide implements import interface late library mixin new on operator part required sealed set show static super sync this typedef var with when"),
    control: words("if else for while do return break continue switch case default try catch finally throw rethrow yield await assert in is"),
    types: words("int double num bool String List Map Set void Object Future Stream Iterable Function Never Null"),
    constants: words("true false null"), lineComment: ["//"], blockComment: [["/*", "*/"]], nest: true, triple: ['"""', "'''"],
    typeAfter: words("class extends implements with mixin enum new"), pascalTypes: true, hook: annotation,
  });
  T.groovy = clike({
    keywords: words("def class interface enum trait extends implements import package new this super static final private protected public abstract var in as instanceof"),
    control: words("if else for while do return break continue switch case default try catch finally throw assert"),
    types: words("int long double float boolean char byte short void String Object List Map"), constants: words("true false null"),
    lineComment: ["//"], blockComment: [["/*", "*/"]], triple: ['"""', "'''"], typeAfter: words("class interface enum trait extends implements new"),
    pascalTypes: true, hook: annotation,
  });
  T.php = clike({
    keywords: lowerWords("abstract and array as callable class clone const declare echo enum extends final fn function global goto implements include include_once instanceof insteadof interface isset list match namespace new or print private protected public readonly require require_once static trait unset use var xor self parent empty"),
    control: lowerWords("if else elseif endif for foreach endforeach while endwhile do switch endswitch case default break continue return try catch finally throw yield"),
    types: lowerWords("int float bool string void mixed object iterable never"), constants: lowerWords("true false null __CLASS__ __DIR__ __FILE__ __FUNCTION__ __LINE__ __METHOD__ __NAMESPACE__"),
    caseInsensitive: true, lineComment: ["//", "#"], blockComment: [["/*", "*/"]],
    strings: { '"': { esc: true, multi: true }, "'": { esc: true, multi: true } }, dollarIdent: false,
    fnAfter: words("function fn"), typeAfter: words("class interface trait extends implements new enum"), pascalTypes: true, hook: phpHook,
  });
  T.python = clike({
    keywords: words("def class lambda global nonlocal del pass and or not in is async self cls"),
    control: words("if elif else for while break continue return try except finally raise with as import from yield await assert match case"),
    types: words("int float complex str bytes bytearray bool list tuple dict set frozenset object type range memoryview"),
    constants: words("True False None NotImplemented Ellipsis __name__ __file__"),
    builtins: words("print len open abs all any min max sum sorted reversed enumerate zip map filter isinstance issubclass getattr setattr hasattr super iter next repr format input id hash vars dir round"),
    lineComment: ["#"], triple: ['"""', "'''"], fnAfter: words("def"), typeAfter: words("class"), pascalTypes: true, hook: pyHook,
  });
  const RUBY_IDENT = /[\p{L}_][\p{L}\p{N}_]*[?!]?/uy;
  T.ruby = clike({
    keywords: words("alias and begin class def defined? do end ensure module not or redo retry self super then undef yield __method__ require require_relative include extend attr_accessor attr_reader attr_writer private protected public lambda proc"),
    control: words("if elsif else unless case when while until for in break next return rescue raise loop"),
    constants: words("true false nil __FILE__ __LINE__ __dir__"), lineComment: ["#"], ident: RUBY_IDENT,
    strings: { '"': { esc: true, multi: true }, "'": { esc: true, multi: true } }, regex: true, regexAfter: words("if elsif unless when while until and or not return"),
    fnAfter: words("def"), typeAfter: words("class module"), pascalTypes: true, dollarIdent: false, hook: rubyHook, resume: blockDocResume,
  });
  T.perl = clike({
    keywords: words("my our local sub package use no require bless ref defined undef wantarray __PACKAGE__ BEGIN END and or not eq ne lt gt le ge cmp"),
    control: words("if elsif else unless while until for foreach do last next redo return goto die eval"),
    builtins: words("print printf say open close chomp chop split join push pop shift unshift keys values each exists delete map grep sort reverse length substr index lc uc sprintf warn"),
    lineComment: ["#"], strings: { '"': { esc: true, multi: true }, "'": { esc: true, multi: true } }, regex: true, regexAfter: words("if unless and or not split grep return"),
    fnAfter: words("sub"), dollarIdent: false, hook: perlHook, resume: blockDocResume,
  });
  T.lua = clike({
    keywords: words("local function and or not"), control: words("break do else elseif end for goto if in repeat return then until while"),
    constants: words("true false nil"), builtins: words("print pairs ipairs require type tostring tonumber setmetatable getmetatable error assert pcall xpcall select next rawget rawset unpack"),
    strings: { '"': { esc: true }, "'": { esc: true } }, fnAfter: words("function"), hook: luaHook,
  });
  T.r = clike({
    keywords: words("function"), control: words("if else repeat while for in next break return switch"),
    constants: words("TRUE FALSE NULL NA NA_integer_ NA_real_ NA_character_ Inf NaN T F"),
    builtins: words("c list library require print cat paste paste0 length matrix vector"), lineComment: ["#"],
    strings: { '"': { esc: true, multi: true }, "'": { esc: true, multi: true }, "`": { esc: false } },
    ident: /[\p{L}._][\p{L}\p{N}._]*/uy, capsConstants: false,
  });
  T.haskell = clike({
    keywords: words("class data default deriving foreign import infix infixl infixr instance let module newtype type where qualified as hiding forall family in"),
    control: words("case of if then else do"), constants: words("True False Nothing otherwise"),
    lineComment: ["--"], blockComment: [["{-", "-}"]], nest: true, ident: /[\p{L}_][\p{L}\p{N}_']*/uy, pascalTypes: "any", capsConstants: false, calls: false,
  });
  T.vb = clike({
    keywords: lowerWords("addhandler addressof alias and andalso as byref byval call class const declare default delegate dim directcast enum erase error event explicit friend function get gettype global handles implements imports in inherits interface is isnot let lib like me mod module mustinherit mustoverride mybase myclass namespace narrowing new not notinheritable notoverridable of on operator option optional or orelse overloads overridable overrides paramarray partial private property protected public raiseevent readonly redim removehandler resume set shadows shared static structure sub synclock trycast typeof widening withevents writeonly xor"),
    control: lowerWords("if then else elseif end endif select case for each next to step while wend do loop until exit return goto gosub try catch finally throw continue with using stop"),
    types: lowerWords("boolean byte char date decimal double integer long object sbyte short single string uinteger ulong ushort variant"),
    constants: lowerWords("true false nothing"), caseInsensitive: true, capsConstants: false, lineComment: ["'"],
    strings: { '"': { esc: false, dbl: true } }, fnAfter: lowerWords("sub function"), typeAfter: lowerWords("class structure module interface enum as new inherits implements"),
    hook: vbHook,
  });
  T.sql = clike({
    keywords: lowerWords("select from where insert into values update set delete create table view index drop alter add column primary key foreign references unique not default constraint check join inner left right full outer cross on using group by order having limit offset union all distinct as and or in is like ilike between exists case when then else end asc desc with recursive returning begin commit rollback transaction grant revoke database schema if replace trigger procedure function returns declare cursor fetch open close top over partition window cast convert collate natural lateral materialized temporary temp truncate vacuum analyze explain pragma do language return"),
    types: lowerWords("int integer smallint bigint tinyint decimal numeric real float double precision char varchar nvarchar nchar text blob clob date time timestamp timestamptz datetime interval boolean bool serial bigserial uuid json jsonb xml money bit binary varbinary"),
    constants: lowerWords("true false null unknown"), caseInsensitive: true, capsConstants: false,
    lineComment: ["--"], blockComment: [["/*", "*/"]], strings: { "'": { esc: false, dbl: true, multi: true } }, dollarIdent: false, hook: sqlHook,
  });
  T.asm = clike({
    keywords: lowerWords("mov movl movq movb movw movzx movsx lea push pop call ret jmp je jne jz jnz jg jge jl jle ja jae jb jbe cmp test add sub mul imul div idiv inc dec and or xor not neg shl shr sal sar rol ror nop int syscall leave enter loop db dw dd dq resb resw resd resq equ times section global extern bits org"),
    caseInsensitive: true, capsConstants: false, lineComment: [";", "#", "//"], blockComment: [["/*", "*/"]],
    types: lowerWords("eax ebx ecx edx esi edi esp ebp rax rbx rcx rdx rsi rdi rsp rbp rip r8 r9 r10 r11 r12 r13 r14 r15 ax bx cx dx al ah bl bh cl ch dl dh sp bp si di cs ds es fs gs ss xmm0 xmm1 xmm2 xmm3 xmm4 xmm5 xmm6 xmm7 byte word dword qword ptr"),
    hook: asmHook,
  });

  // ------------------------------------------------------------------ HTML and XML
  // State: "<h|x>:" + c (comment) | t:<tag>:<quote> (inside a tag, maybe inside a quoted
  // value) | js:<inner> / css:<inner> (HTML <script>/<style> bodies) | cd (CDATA) |
  // pi (processing instruction) | dt (doctype or other <! declaration).
  const TAG_OPEN = /<\/?([A-Za-z_][\w:.-]*)/y;
  const ATTR_NAME = /[^\s"'<>\/=]+/y;
  const UNQUOTED = /[^\s>"']+/y;
  const ENTITY = /&(?:#\d+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);/y;
  const SCRIPT_END = /<\/script/i;
  const STYLE_END = /<\/style/i;
  function markup(html) {
    const P = html ? "h:" : "x:";
    return function tokenize(text, state) {
      const out = [];
      const n = text.length;
      let i = 0;
      let st = state ? state.slice(2) : "";
      while (i < n) {
        if (st === "c" || st === "cd" || st === "pi" || st === "dt") {
          const close = st === "c" ? "-->" : st === "cd" ? "]]>" : st === "pi" ? "?>" : ">";
          const k = text.indexOf(close, i);
          const e = k < 0 ? n : k + close.length;
          if (st === "cd") {
            push(out, i, k < 0 ? n : k, "string");
            if (k >= 0) push(out, k, e, "meta");
          } else push(out, i, e, st === "c" ? "comment" : "meta");
          i = e;
          if (k < 0) break;
          st = "";
          continue;
        }
        if (st.startsWith("js:") || st.startsWith("css:")) {
          const js = st.startsWith("js:");
          const inner = st.slice(js ? 3 : 4) || null;
          const k = text.slice(i).search(js ? SCRIPT_END : STYLE_END);
          const end = k < 0 ? n : i + k;
          const r = (js ? T.javascript : T.css)(text.slice(i, end), inner);
          append(out, r.tokens, i);
          i = end;
          if (k < 0) { st = (js ? "js:" : "css:") + (r.state || ""); break; }
          st = "";
          continue;
        }
        if (st.startsWith("t:")) {
          const k = st.lastIndexOf(":");
          const tag = st.slice(2, k);
          const q = st.slice(k + 1);
          if (q) {
            const e0 = text.indexOf(q, i);
            const e = e0 < 0 ? n : e0 + 1;
            push(out, i, e, "string");
            i = e;
            if (e0 < 0) break;
            st = `t:${tag}:`;
            continue;
          }
          const c = text.charCodeAt(i);
          if (c === 32 || c === 9) { i++; continue; }
          if (c === 47 && text.charCodeAt(i + 1) === 62) { push(out, i, i + 2, "tag"); i += 2; st = ""; continue; }
          if (c === 62 || (c === 63 && text.charCodeAt(i + 1) === 62)) {
            const e = c === 62 ? i + 1 : i + 2;
            push(out, i, e, "tag");
            i = e;
            st = html && tag === "script" ? "js:" : html && tag === "style" ? "css:" : "";
            continue;
          }
          if (c === 61) { i++; continue; }
          if (c === 34 || c === 39) {
            const qc = text[i];
            const e0 = text.indexOf(qc, i + 1);
            const e = e0 < 0 ? n : e0 + 1;
            push(out, i, e, "string");
            i = e;
            if (e0 < 0) { st = `t:${tag}:${qc}`; break; }
            continue;
          }
          if (text.charCodeAt(i - 1) === 61 || (text.charCodeAt(i - 1) === 32 && /=\s*$/.test(text.slice(Math.max(0, i - 8), i)))) {
            UNQUOTED.lastIndex = i;
            if (UNQUOTED.test(text)) { push(out, i, UNQUOTED.lastIndex, "string"); i = UNQUOTED.lastIndex; continue; }
          }
          ATTR_NAME.lastIndex = i;
          if (ATTR_NAME.test(text)) { push(out, i, ATTR_NAME.lastIndex, "attr"); i = ATTR_NAME.lastIndex; continue; }
          i++;
          continue;
        }
        // Text content.
        const c = text.charCodeAt(i);
        if (c === 38) {
          ENTITY.lastIndex = i;
          if (ENTITY.test(text)) { push(out, i, ENTITY.lastIndex, "escape"); i = ENTITY.lastIndex; continue; }
          i++;
          continue;
        }
        if (c !== 60) {
          let j = i + 1;
          while (j < n && text.charCodeAt(j) !== 60 && text.charCodeAt(j) !== 38) j++;
          i = j;
          continue;
        }
        if (text.startsWith("<!--", i)) { st = "c"; push(out, i, i + 4, "comment"); i += 4; continue; }
        if (text.startsWith("<![CDATA[", i)) { push(out, i, i + 9, "meta"); i += 9; st = "cd"; continue; }
        if (text.startsWith("<?", i)) { push(out, i, i + 2, "meta"); i += 2; st = "pi"; continue; }
        if (text.startsWith("<!", i)) { push(out, i, i + 2, "meta"); i += 2; st = "dt"; continue; }
        TAG_OPEN.lastIndex = i;
        const m = TAG_OPEN.exec(text);
        if (m) {
          push(out, i, TAG_OPEN.lastIndex, "tag");
          const name = html ? m[1].toLowerCase() : m[1];
          st = `t:${text.charCodeAt(i + 1) === 47 ? "/" : ""}${name}:`;
          i = TAG_OPEN.lastIndex;
          continue;
        }
        i++;
      }
      return { tokens: mergeComment(out), state: st ? P + st : null };
    };
  }
  /** Joins the comment and meta pieces scanners emit around "<!--", "<?" and "/*" into one token. */
  function mergeComment(out) {
    for (let k = out.length - 1; k > 0; k--) {
      const a = out[k - 1];
      const b = out[k];
      if (a[2] === b[2] && (a[2] === "comment" || a[2] === "meta") && a[1] === b[0]) {
        a[1] = b[1];
        out.splice(k, 1);
      }
    }
    return out;
  }
  T.html = markup(true);
  T.xml = markup(false);

  // PHP: HTML outside <?php … ?>. State "o:<html state>" outside, "p:<php state>" inside.
  const phpCode = T.php;
  const PHP_OPEN = /<\?(?:php\b|=)?/i;
  T.php = function (text, state) {
    const out = [];
    const n = text.length;
    let i = 0;
    let inPhp = !!state && state.startsWith("p:");
    let inner = state ? state.slice(2) || null : null;
    while (i < n) {
      if (inPhp) {
        // A "?>" inside a string doesn't end PHP; one in a // comment does, as in PHP itself.
        let k = text.indexOf("?>", i);
        let r = phpCode(text.slice(i, k < 0 ? n : k), inner);
        while (k >= 0 && r.state && /\|[sSd]/.test(r.state)) {
          k = text.indexOf("?>", k + 2);
          r = phpCode(text.slice(i, k < 0 ? n : k), inner);
        }
        append(out, r.tokens, i);
        if (k < 0) { inner = r.state; i = n; break; }
        push(out, k, k + 2, "meta");
        i = k + 2;
        inPhp = false;
        inner = null;
      } else {
        const m = PHP_OPEN.exec(text.slice(i));
        const k = m ? i + m.index : n;
        const r = T.html(text.slice(i, k), inner);
        append(out, r.tokens, i);
        inner = r.state;
        if (!m) { i = n; break; }
        push(out, k, k + m[0].length, "meta");
        i = k + m[0].length;
        inPhp = true;
        inner = null;
      }
    }
    if (inPhp) return { tokens: out, state: "p:" + (inner || "") };
    return { tokens: out, state: inner ? "o:" + inner : null };
  };

  // ------------------------------------------------------------------ CSS and SCSS
  // State: "<depth><c if in a comment><v if in a declaration value>", null at depth 0.
  const CSS_IDENT = /-?-?[\p{L}_][\p{L}\p{N}_-]*/uy;
  const CSS_NUM = /[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?(?:%|[a-zA-Z]+)?/y;
  const CSS_HEX = /#[0-9a-fA-F]{3,8}(?![\w-])/y;
  const CSS_AT = /@[\w-]+/y;
  const SCSS_VAR = /\$[\w-]+/y;
  function cssLike(scss) {
    return function tokenize(text, state) {
      const out = [];
      const n = text.length;
      let i = 0;
      let depth = 0;
      let comment = false;
      let value = false;
      if (state) {
        depth = parseInt(state, 10) || 0;
        comment = state.includes("c");
        value = state.includes("v");
      }
      let stmt = true; // at the start of a selector or declaration
      while (i < n) {
        if (comment) {
          const k = text.indexOf("*/", i);
          const e = k < 0 ? n : k + 2;
          push(out, i, e, "comment");
          i = e;
          if (k < 0) break;
          comment = false;
          continue;
        }
        const c = text.charCodeAt(i);
        if (c === 32 || c === 9) { i++; continue; }
        if (c === 47 && text.charCodeAt(i + 1) === 42) { push(out, i, i + 2, "comment"); i += 2; comment = true; continue; }
        if (scss && c === 47 && text.charCodeAt(i + 1) === 47) { push(out, i, n, "comment"); break; }
        if (c === 123) { depth++; value = false; stmt = true; i++; continue; }
        if (c === 125) { depth = Math.max(0, depth - 1); value = false; stmt = true; i++; continue; }
        if (c === 59) { value = false; stmt = true; i++; continue; }
        if (c === 34 || c === 39) { i = stringBody(text, i, i + 1, text[i], true, out, false)[0]; stmt = false; continue; }
        if (c === 64) {
          CSS_AT.lastIndex = i;
          if (CSS_AT.test(text)) {
            const at = text.slice(i, CSS_AT.lastIndex);
            push(out, i, CSS_AT.lastIndex, "meta");
            i = CSS_AT.lastIndex;
            if (/^@(include|mixin|extend|function|use|forward)$/.test(at)) {
              while (text.charCodeAt(i) === 32) i++;
              CSS_IDENT.lastIndex = i;
              if (CSS_IDENT.test(text)) { push(out, i, CSS_IDENT.lastIndex, "function"); i = CSS_IDENT.lastIndex; }
            }
            value = true; // at-rule preludes read like values (media features, numbers)
            stmt = false;
            continue;
          }
        }
        if (scss && c === 36) {
          SCSS_VAR.lastIndex = i;
          if (SCSS_VAR.test(text)) { push(out, i, SCSS_VAR.lastIndex, "variable"); i = SCSS_VAR.lastIndex; stmt = false; continue; }
        }
        if (c === 35 && text.charCodeAt(i + 1) === 123) { push(out, i, i + 2, "operator"); i += 2; continue; }
        if (value) {
          if (c === 35) {
            CSS_HEX.lastIndex = i;
            if (CSS_HEX.test(text)) { push(out, i, CSS_HEX.lastIndex, "number"); i = CSS_HEX.lastIndex; continue; }
          }
          if (isDigit(c) || ((c === 46 || c === 45 || c === 43) && isDigit(text.charCodeAt(i + 1)))) {
            CSS_NUM.lastIndex = i;
            if (CSS_NUM.test(text)) { push(out, i, CSS_NUM.lastIndex, "number"); i = CSS_NUM.lastIndex; continue; }
          }
          if (c === 33) {
            const m = /!\s*[a-zA-Z]+/y;
            m.lastIndex = i;
            if (m.test(text)) { push(out, i, m.lastIndex, "keyword"); i = m.lastIndex; continue; }
          }
          CSS_IDENT.lastIndex = i;
          if (CSS_IDENT.test(text)) {
            const e = CSS_IDENT.lastIndex;
            const w = text.slice(i, e);
            if (text.charCodeAt(e) === 40) {
              push(out, i, e, "function");
              if (/^url$/i.test(w)) {
                const k = text.indexOf(")", e);
                const end = k < 0 ? n : k;
                if (!/^\(\s*["']/.test(text.slice(e, e + 3))) push(out, e + 1, end, "string");
                i = end;
                continue;
              }
            } else push(out, i, e, w.startsWith("--") ? "variable" : "constant");
            i = e;
            continue;
          }
          if (c === 44 || c === 47 || c === 42 || c === 43) push(out, i, i + 1, "operator");
          i++;
          continue;
        }
        // Selectors, and property names inside blocks.
        CSS_IDENT.lastIndex = i;
        if (stmt && depth > 0 && CSS_IDENT.test(text)) {
          const e = CSS_IDENT.lastIndex;
          let j = e;
          while (text.charCodeAt(j) === 32) j++;
          if (text.charCodeAt(j) === 58) {
            const rest = text.slice(j);
            const brace = rest.search(/[{;}]/);
            if (brace < 0 || rest[brace] !== "{" || /^:\s*$/.test(rest.slice(0, brace))) {
              push(out, i, e, text.startsWith("--", i) ? "variable" : "property");
              i = j + 1;
              value = true;
              stmt = false;
              continue;
            }
          }
        }
        stmt = false;
        if (c === 46 || c === 35) {
          CSS_IDENT.lastIndex = i + 1;
          if (CSS_IDENT.test(text)) { push(out, i, CSS_IDENT.lastIndex, c === 46 ? "type" : "constant"); i = CSS_IDENT.lastIndex; continue; }
        }
        if (c === 58) {
          let j = i + 1;
          if (text.charCodeAt(j) === 58) j++;
          CSS_IDENT.lastIndex = j;
          if (CSS_IDENT.test(text)) { push(out, i, CSS_IDENT.lastIndex, "keyword"); i = CSS_IDENT.lastIndex; continue; }
        }
        if (c === 91) {
          const k = text.indexOf("]", i);
          const end = k < 0 ? n : k + 1;
          const inside = /^\[\s*([^\s=~|^$*\]]+)/.exec(text.slice(i, end));
          if (inside) push(out, i + inside[0].length - inside[1].length, i + inside[0].length, "attr");
          const v = /=\s*("[^"]*"?|'[^']*'?|[^\]\s]+)/.exec(text.slice(i, end));
          if (v) push(out, i + v.index + v[0].length - v[1].length, i + v.index + v[0].length, "string");
          i = end;
          continue;
        }
        if (c === 38 || c === 62 || c === 43 || c === 126 || c === 42 || c === 44) { push(out, i, i + 1, "operator"); i++; continue; }
        if (isDigit(c)) {
          CSS_NUM.lastIndex = i;
          CSS_NUM.test(text);
          push(out, i, CSS_NUM.lastIndex, "number");
          i = Math.max(CSS_NUM.lastIndex, i + 1);
          continue;
        }
        CSS_IDENT.lastIndex = i;
        if (CSS_IDENT.test(text)) { push(out, i, CSS_IDENT.lastIndex, "tag"); i = CSS_IDENT.lastIndex; continue; }
        i++;
      }
      const s = `${depth}${comment ? "c" : ""}${value ? "v" : ""}`;
      return { tokens: mergeComment(out), state: s === "0" ? null : s };
    };
  }
  T.css = cssLike(false);
  T.scss = cssLike(true);

  // ------------------------------------------------------------------ JSON
  const JSON_NUM = /-?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/y;
  const JSON_WORD = /[A-Za-z_$][\w$]*/y;
  function json(text, state) {
    const out = [];
    const n = text.length;
    let i = 0;
    let comment = state === "c";
    while (i < n) {
      if (comment) {
        const k = text.indexOf("*/", i);
        const e = k < 0 ? n : k + 2;
        push(out, i, e, "comment");
        i = e;
        if (k < 0) break;
        comment = false;
        continue;
      }
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 47 && text.charCodeAt(i + 1) === 47) { push(out, i, n, "comment"); break; }
      if (c === 47 && text.charCodeAt(i + 1) === 42) { push(out, i, i + 2, "comment"); i += 2; comment = true; continue; }
      if (c === 34) {
        const at = out.length;
        i = stringBody(text, i, i + 1, '"', true, out, false)[0];
        let j = i;
        while (text.charCodeAt(j) === 32 || text.charCodeAt(j) === 9) j++;
        if (text.charCodeAt(j) === 58) for (let k = at; k < out.length; k++) if (out[k][2] === "string") out[k][2] = "property";
        continue;
      }
      if (c === 45 || isDigit(c)) {
        JSON_NUM.lastIndex = i;
        if (JSON_NUM.test(text) && JSON_NUM.lastIndex > i) { push(out, i, JSON_NUM.lastIndex, "number"); i = JSON_NUM.lastIndex; continue; }
      }
      JSON_WORD.lastIndex = i;
      if (JSON_WORD.test(text)) {
        const w = text.slice(i, JSON_WORD.lastIndex);
        push(out, i, JSON_WORD.lastIndex, w === "true" || w === "false" || w === "null" ? "constant" : "invalid");
        i = JSON_WORD.lastIndex;
        continue;
      }
      i++;
    }
    return { tokens: mergeComment(out), state: comment ? "c" : null };
  }
  T.json = json;
  T.jsonc = json;

  // ------------------------------------------------------------------ Markdown
  // State: null, or "f:<fence>:<language id>:<inner state>" inside a fenced code block.
  const MD_SPECIAL = /[\\`*_~!\[<h|]/g;
  const MD_LINK = /!?\[(?:[^\]\\]|\\.)*\](?:\([^)\s]*(?:\s+"[^"]*")?\)|\[[^\]]*\])/y;
  const MD_AUTOLINK = /<(?:https?:\/\/|mailto:|ftp:\/\/)[^>\s]*>/y;
  const MD_HTML = /<\/?[A-Za-z][\w-]*(?:\s[^<>]*)?\/?>/y;
  const MD_URL = /https?:\/\/[^\s<>()\[\]]*[^\s<>()\[\].,;:!?'"*_]/y;
  function mdDelimited(text, i, mark, type, out) {
    const s = i + mark.length;
    if (s >= text.length || text[s] === " " || text[s] === "\t") return -1;
    let k = text.indexOf(mark, s + 1);
    while (k > 0) {
      const before = text[k - 1];
      const after = text[k + mark.length];
      const doubled = mark.length === 1 && (after === mark || before === mark);
      if (before !== " " && before !== "\t" && !doubled && !(mark === "_" && after && /[\p{L}\p{N}]/u.test(after))) {
        push(out, i, k + mark.length, type);
        return k + mark.length;
      }
      k = text.indexOf(mark, k + 1);
    }
    return -1;
  }
  function mdInline(text, i, out) {
    const n = text.length;
    while (i < n) {
      MD_SPECIAL.lastIndex = i;
      const m = MD_SPECIAL.exec(text);
      if (!m) break;
      i = m.index;
      const c = text.charCodeAt(i);
      if (c === 92) {
        if (i + 1 < n && /[!-\/:-@\[-`{-~]/.test(text[i + 1])) { push(out, i, i + 2, "escape"); i += 2; }
        else i++;
        continue;
      }
      if (c === 96) {
        let r = 1;
        while (text.charCodeAt(i + r) === 96) r++;
        const run = text.substr(i, r);
        let k = text.indexOf(run, i + r);
        while (k >= 0 && (text.charCodeAt(k + r) === 96 || text.charCodeAt(k - 1) === 96)) {
          let e = k;
          while (text.charCodeAt(e) === 96) e++;
          k = text.indexOf(run, e);
        }
        if (k < 0) { i += r; continue; }
        push(out, i, k + r, "code");
        i = k + r;
        continue;
      }
      if (c === 42 || c === 95) {
        if (c === 95 && i > 0 && /[\p{L}\p{N}]/u.test(text[i - 1])) { i++; continue; }
        const dbl = text.charCodeAt(i + 1) === c;
        const mark = dbl ? text.substr(i, 2) : text[i];
        const e = mdDelimited(text, i, mark, dbl ? "strong" : "emphasis", out);
        i = e > 0 ? e : i + mark.length;
        continue;
      }
      if (c === 126) {
        if (text.charCodeAt(i + 1) === 126) {
          const e = mdDelimited(text, i, "~~", "deleted", out);
          i = e > 0 ? e : i + 2;
        } else i++;
        continue;
      }
      if (c === 91 || (c === 33 && text.charCodeAt(i + 1) === 91)) {
        MD_LINK.lastIndex = i;
        if (MD_LINK.test(text)) {
          const e = MD_LINK.lastIndex;
          let k = text.indexOf("]", i + 1);
          while (k > 0 && text[k - 1] === "\\") k = text.indexOf("]", k + 1);
          push(out, i, k + 1, "link");
          push(out, k + 1, e, "string");
          i = e;
          continue;
        }
        i++;
        continue;
      }
      if (c === 60) {
        MD_AUTOLINK.lastIndex = i;
        if (MD_AUTOLINK.test(text)) { push(out, i, MD_AUTOLINK.lastIndex, "link"); i = MD_AUTOLINK.lastIndex; continue; }
        MD_HTML.lastIndex = i;
        if (MD_HTML.test(text)) { push(out, i, MD_HTML.lastIndex, "tag"); i = MD_HTML.lastIndex; continue; }
        i++;
        continue;
      }
      if (c === 104) {
        if (i === 0 || !/[\p{L}\p{N}]/u.test(text[i - 1])) {
          MD_URL.lastIndex = i;
          if (MD_URL.test(text)) { push(out, i, MD_URL.lastIndex, "link"); i = MD_URL.lastIndex; continue; }
        }
        i++;
        continue;
      }
      if (c === 124) push(out, i, i + 1, "operator");
      i++;
    }
    return out;
  }
  const MD_FENCE_OPEN = /^(\s{0,3})(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/;
  const MD_FENCE_CLOSE = /^(\s{0,3})(`{3,}|~{3,})\s*$/;
  function markdown(text, state) {
    const n = text.length;
    if (state && state.startsWith("f:")) {
      const a = state.indexOf(":", 2);
      const b = state.indexOf(":", a + 1);
      const fence = state.slice(2, a);
      const lang = state.slice(a + 1, b);
      const inner = state.slice(b + 1) || null;
      const m = MD_FENCE_CLOSE.exec(text);
      if (m && m[2][0] === fence[0] && m[2].length >= fence.length) return { tokens: [[m[1].length, n, "code"]], state: null };
      if (lang && T[lang]) {
        const r = T[lang](text, inner);
        return { tokens: r.tokens, state: `f:${fence}:${lang}:${r.state || ""}` };
      }
      return { tokens: /\S/.test(text) ? [[0, n, "code"]] : [], state };
    }
    const out = [];
    let m = MD_FENCE_OPEN.exec(text);
    if (m) {
      const info = m[3].toLowerCase().replace(/^\{\.?|\}$/g, "");
      const lang = info ? aliasOf(info) : "";
      return { tokens: [[m[1].length, n, "code"]], state: `f:${m[2]}:${lang && lang !== "plaintext" && lang !== "markdown" ? lang : ""}:` };
    }
    let first = 0;
    while (first < n && (text.charCodeAt(first) === 32 || text.charCodeAt(first) === 9)) first++;
    if (first === n) return { tokens: [], state: null };
    if (/^\s{0,3}#{1,6}(\s|$)/.test(text)) return { tokens: fill(mdInline(text, first, []), first, n, "heading"), state: null };
    if (/^\s{0,3}=+\s*$/.test(text)) return { tokens: [[first, n, "heading"]], state: null };
    if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(text)) return { tokens: [[first, n, "meta"]], state: null };
    let i = first;
    m = /^(?:\s{0,3}>\s?)+/.exec(text);
    if (m) {
      push(out, first, m[0].length, "quote");
      for (const t of fill(mdInline(text, m[0].length, []), m[0].length, n, "quote")) out.push(t);
      return { tokens: out, state: null };
    }
    m = /^(\s*)([-*+]|\d{1,9}[.)])(\s+|$)/.exec(text);
    if (m) {
      push(out, m[1].length, m[1].length + m[2].length, "list");
      i = m[0].length;
      if (/^\[[ xX]\](?=\s)/.test(text.slice(i))) { push(out, i, i + 3, "keyword"); i += 3; }
    } else {
      m = /^\s{0,3}(\[[^\]]+\]):\s*(\S+)/.exec(text);
      if (m) {
        push(out, first, first + m[1].length + 1, "label");
        push(out, m[0].length - m[2].length, m[0].length, "link");
        return { tokens: out, state: null };
      }
      if (first < 4 && text.charCodeAt(first) === 60 && /^<\/?[A-Za-z!]/.test(text.slice(first))) {
        return { tokens: T.html(text, null).tokens, state: null };
      }
    }
    mdInline(text, i, out);
    return { tokens: out, state: null };
  }
  T.markdown = markdown;

  // ------------------------------------------------------------------ YAML
  // State: null | "b:<indent>" inside a block scalar (lines indented past <indent>) |
  // "q:<quote>" inside a multi-line quoted scalar.
  const YAML_KEY = /^((?:"(?:[^"\\]|\\.)*"|'(?:[^']|'')*'|[^\s#"'\[\]{},&*!|>%@`?][^#]*?|\?\s[^#]*?))\s*:(?=\s|$)/;
  const YAML_NUM = /^[-+]?(?:\d[\d_]*(?:\.\d*)?(?:[eE][-+]?\d+)?|\.\d+(?:[eE][-+]?\d+)?|0x[0-9a-fA-F_]+|0o[0-7_]+|0b[01_]+|\.(?:inf|Inf|INF|nan|NaN|NAN))$/;
  const YAML_CONST = /^(?:true|false|yes|no|on|off|y|n|True|False|Yes|No|On|Off|TRUE|FALSE|YES|NO|ON|OFF|null|Null|NULL|~)$/;
  const YAML_DATE = /^\d{4}-\d\d?-\d\d?(?:[Tt ]\d\d?:\d\d:\d\d(?:\.\d*)?(?:\s*(?:Z|[-+]\d\d?(?::\d\d)?))?)?$/;
  const YAML_ANCHOR = /[&*][^\s,\[\]{}]+/y;
  const YAML_TAG = /![^\s,\[\]{}]*/y;
  function yamlScalarType(s) {
    if (YAML_NUM.test(s) || YAML_DATE.test(s)) return "number";
    if (YAML_CONST.test(s)) return "constant";
    return "string";
  }
  function yamlValue(text, i, out, blockIndent, flow) {
    const n = text.length;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 35 && (i === 0 || text.charCodeAt(i - 1) === 32 || text.charCodeAt(i - 1) === 9)) { push(out, i, n, "comment"); return null; }
      if (c === 38 || c === 42) {
        YAML_ANCHOR.lastIndex = i;
        if (YAML_ANCHOR.test(text)) { push(out, i, YAML_ANCHOR.lastIndex, "variable"); i = YAML_ANCHOR.lastIndex; continue; }
      }
      if (c === 33) {
        YAML_TAG.lastIndex = i;
        YAML_TAG.test(text);
        push(out, i, YAML_TAG.lastIndex, "type");
        i = Math.max(YAML_TAG.lastIndex, i + 1);
        continue;
      }
      if (!flow && (c === 124 || c === 62) && /^[|>][-+0-9]*\s*(#.*)?$/.test(text.slice(i))) {
        const m = /^[|>][-+0-9]*/.exec(text.slice(i));
        push(out, i, i + m[0].length, "operator");
        const hash = text.indexOf("#", i);
        if (hash > 0) push(out, hash, n, "comment");
        return "b:" + blockIndent;
      }
      if (c === 34 || c === 39) {
        const r = stringBody(text, i, i + 1, text[i], c === 34, out, c === 39);
        if (!r[1]) return "q:" + text[i];
        i = r[0];
        continue;
      }
      if (c === 91 || c === 123 || c === 93 || c === 125 || c === 44) {
        if (c === 91 || c === 123) flow++;
        else if (c !== 44) flow = Math.max(0, flow - 1);
        i++;
        continue;
      }
      if (c === 45 && (text.charCodeAt(i + 1) === 32 || i + 1 === n) && !flow) { push(out, i, i + 1, "operator"); i++; continue; }
      // A plain scalar runs to " #", or to a flow indicator inside [ ] / { }.
      let e = i;
      while (e < n) {
        const ch = text.charCodeAt(e);
        if (ch === 35 && (text.charCodeAt(e - 1) === 32 || text.charCodeAt(e - 1) === 9)) break;
        if (flow && (ch === 44 || ch === 93 || ch === 125)) break;
        if (flow && ch === 58 && (text.charCodeAt(e + 1) === 32 || e + 1 === n)) break;
        e++;
      }
      let end = e;
      while (end > i && (text.charCodeAt(end - 1) === 32 || text.charCodeAt(end - 1) === 9)) end--;
      if (flow && text.charCodeAt(e) === 58) {
        push(out, i, end, "property");
        push(out, e, e + 1, "operator");
        i = e + 1;
        continue;
      }
      push(out, i, end, yamlScalarType(text.slice(i, end)));
      i = Math.max(e, i + 1);
    }
    return null;
  }
  function yaml(text, state) {
    const out = [];
    const n = text.length;
    let first = 0;
    while (first < n && text.charCodeAt(first) === 32) first++;
    if (state && state[0] === "b") {
      const ind = +state.slice(2);
      if (first === n) return { tokens: [], state };
      if (first > ind) return { tokens: [[first, n, "string"]], state };
    }
    let i = first;
    if (state && state[0] === "q") {
      const q = state[2];
      const r = stringBody(text, 0, 0, q, q === '"', out, q === "'");
      if (!r[1]) return { tokens: out, state };
      return { tokens: out, state: yamlValue(text, r[0], out, first, 0) };
    }
    if (first === n) return { tokens: [], state: null };
    if (/^(?:---|\.\.\.)(?:\s|$)/.test(text)) {
      push(out, 0, 3, "meta");
      return { tokens: out, state: yamlValue(text, 3, out, 0, 0) };
    }
    if (text.charCodeAt(first) === 35) return { tokens: [[first, n, "comment"]], state: null };
    if (first === 0 && text.charCodeAt(0) === 37) return { tokens: [[0, n, "meta"]], state: null };
    let keyCol = first;
    while (text.charCodeAt(i) === 45 && (text.charCodeAt(i + 1) === 32 || i + 1 === n)) {
      push(out, i, i + 1, "operator");
      keyCol = i;
      i++;
      while (text.charCodeAt(i) === 32) i++;
    }
    const km = YAML_KEY.exec(text.slice(i));
    if (km) {
      push(out, i, i + km[1].length, "property");
      push(out, i + km[0].length - 1, i + km[0].length, "operator");
      keyCol = i;
      i += km[0].length;
    }
    return { tokens: out, state: yamlValue(text, i, out, keyCol, 0) };
  }
  T.yaml = yaml;

  // ------------------------------------------------------------------ TOML and INI
  // TOML state: null, or "s<quotes>" inside a multi-line string.
  const TOML_KEY = /(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*"|'[^']*')(?:\s*\.\s*(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*"|'[^']*'))*(?=\s*=)/y;
  const TOML_SECTION = /\[\[?[^\]]*\]?\]?/y;
  const TOML_DATE = /\d{4}-\d{2}-\d{2}(?:[Tt ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:[Zz]|[+-]\d{2}:\d{2})?)?|\d{2}:\d{2}:\d{2}(?:\.\d+)?/y;
  const TOML_NUM = /[+-]?(?:0x[0-9a-fA-F_]+|0o[0-7_]+|0b[01_]+|inf|nan|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?)/y;
  function toml(text, state) {
    const out = [];
    const n = text.length;
    let i = 0;
    let expectKey = true;
    if (state) {
      const q = state.slice(1);
      const r = stringBody(text, 0, 0, q, q === '"""', out, false);
      if (!r[1]) return { tokens: out, state };
      i = r[0];
      expectKey = false;
    } else {
      while (i < n && (text.charCodeAt(i) === 32 || text.charCodeAt(i) === 9)) i++;
      if (text.charCodeAt(i) === 91 && TOML_KEY_LIKE.test(text.slice(i))) {
        TOML_SECTION.lastIndex = i;
        TOML_SECTION.test(text);
        push(out, i, TOML_SECTION.lastIndex, "section");
        i = TOML_SECTION.lastIndex;
        expectKey = false;
      }
    }
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 35) { push(out, i, n, "comment"); break; }
      if (c === 123 || c === 44) { i++; expectKey = true; continue; }
      if (expectKey) {
        expectKey = false;
        TOML_KEY.lastIndex = i;
        if (TOML_KEY.test(text)) { push(out, i, TOML_KEY.lastIndex, "label"); i = TOML_KEY.lastIndex; continue; }
      }
      if (c === 61) { push(out, i, i + 1, "operator"); i++; continue; }
      if (c === 34 || c === 39) {
        const triple = text.startsWith(c === 34 ? '"""' : "'''", i);
        const close = triple ? text.substr(i, 3) : text[i];
        const r = stringBody(text, i, i + close.length, close, c === 34, out, false);
        i = r[0];
        if (!r[1] && triple) return { tokens: out, state: "s" + close };
        continue;
      }
      if (isDigit(c)) {
        TOML_DATE.lastIndex = i;
        if (TOML_DATE.test(text)) { push(out, i, TOML_DATE.lastIndex, "number"); i = TOML_DATE.lastIndex; continue; }
      }
      if (isDigit(c) || c === 43 || c === 45 || c === 105 || c === 110) {
        TOML_NUM.lastIndex = i;
        if (TOML_NUM.test(text) && TOML_NUM.lastIndex > i + (c === 43 || c === 45 ? 1 : 0)) {
          push(out, i, TOML_NUM.lastIndex, "number");
          i = TOML_NUM.lastIndex;
          continue;
        }
      }
      if (text.startsWith("true", i) || text.startsWith("false", i)) {
        const e = i + (text.startsWith("true", i) ? 4 : 5);
        push(out, i, e, "constant");
        i = e;
        continue;
      }
      i++;
    }
    return { tokens: out, state: null };
  }
  // "[table]" at the start of a line, as opposed to a multi-line array's "[1, 2]".
  const TOML_KEY_LIKE = /^\[\[?\s*(?:[A-Za-z0-9_-]+|"[^"]*"|'[^']*')(?:\s*\.\s*(?:[A-Za-z0-9_-]+|"[^"]*"|'[^']*'))*\s*\]\]?/;
  T.toml = toml;

  const INI_KEY = /^(\s*)([^=:\s;#\[][^=:]*?)\s*([=:])/;
  function ini(text) {
    const n = text.length;
    let first = 0;
    while (first < n && (text.charCodeAt(first) === 32 || text.charCodeAt(first) === 9)) first++;
    if (first === n) return { tokens: [], state: null };
    const c = text.charCodeAt(first);
    if (c === 59 || c === 35 || c === 33) return { tokens: [[first, n, "comment"]], state: null };
    const out = [];
    if (c === 91) {
      const k = text.indexOf("]", first);
      const e = k < 0 ? n : k + 1;
      push(out, first, e, "section");
      const hash = text.slice(e).search(/[;#]/);
      if (hash >= 0) push(out, e + hash, n, "comment");
      return { tokens: out, state: null };
    }
    const m = INI_KEY.exec(text);
    if (!m) return { tokens: [], state: null };
    const ks = m[1].length;
    push(out, ks, ks + m[2].length, "label");
    push(out, m[0].length - 1, m[0].length, "operator");
    let i = m[0].length;
    while (i < n && (text.charCodeAt(i) === 32 || text.charCodeAt(i) === 9)) i++;
    let end = n;
    while (end > i && (text.charCodeAt(end - 1) === 32 || text.charCodeAt(end - 1) === 9)) end--;
    const v = text.slice(i, end);
    if (!v) return { tokens: out, state: null };
    if (v[0] === '"') stringBody(text, i, i + 1, '"', true, out, false);
    else if (/^'.*'$/.test(v)) push(out, i, end, "string");
    else if (/^[-+]?(\d+(\.\d+)?|0x[0-9a-fA-F]+)$/.test(v) || /^(dword|qword|hex(\([0-9a-f]\))?):[0-9a-fA-F,]*$/i.test(v)) push(out, i, end, "number");
    else if (/^(true|false|yes|no|on|off|enabled|disabled)$/i.test(v)) push(out, i, end, "constant");
    return { tokens: out, state: null };
  }
  T.ini = ini;

  // ------------------------------------------------------------------ CSV and TSV
  // Rainbow columns. State "q:<column>" while a quoted field runs onto the next line.
  function delimited(delim) {
    return function (text, state) {
      const out = [];
      const n = text.length;
      let col = 0;
      let i = 0;
      let inQuote = false;
      if (state) {
        col = +state.slice(2);
        inQuote = true;
      }
      for (;;) {
        const s = i;
        let j = i;
        if (inQuote || text.charCodeAt(i) === 34) {
          if (!inQuote) j++;
          inQuote = false;
          let closed = false;
          while (j < n) {
            if (text.charCodeAt(j) === 34) {
              if (text.charCodeAt(j + 1) === 34) { j += 2; continue; }
              j++;
              closed = true;
              break;
            }
            j++;
          }
          if (!closed) {
            push(out, s, n, "col" + (col % 6));
            return { tokens: out, state: "q:" + col };
          }
        }
        const k = text.indexOf(delim, j);
        const e = k < 0 ? n : k;
        if (e > s && /\S/.test(text.slice(s, e))) push(out, s, e, "col" + (col % 6));
        if (k < 0) break;
        i = k + 1;
        col++;
      }
      return { tokens: out, state: null };
    };
  }
  T.csv = delimited(",");
  T.tsv = delimited("\t");

  // ------------------------------------------------------------------ logs, diffs, ignore files
  const LOG_RE = /(\b\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:[.,]\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?|\b\d{1,2}[\/.]\d{1,2}[\/.]\d{2,4}\b|\b\d{2}:\d{2}:\d{2}(?:[.,]\d+)?\b|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) +\d{1,2}\b)|\b(ERROR|ERR|FATAL|FAIL(?:ED|URE)?|CRITICAL|CRIT|SEVERE|PANIC|EMERG(?:ENCY)?|ALERT|\w+(?:Exception|Error))\b|\b(WARN(?:ING)?)\b|\b(INFO|NOTICE|NOTE)\b|\b(DEBUG|TRACE|VERBOSE|FINE(?:R|ST)?)\b|("[^"]*")|((?:https?|ftp|file):\/\/[^\s"'<>]+)|\b([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|0x[0-9a-fA-F]+)\b/gi;
  const LOG_TYPES = [null, "number", "error", "warning", "info", "debug", "string", "link", "constant"];
  function log(text) {
    const out = [];
    LOG_RE.lastIndex = 0;
    let m;
    while ((m = LOG_RE.exec(text))) {
      let g = 1;
      while (m[g] === undefined) g++;
      push(out, m.index, m.index + m[0].length, LOG_TYPES[g]);
      if (!m[0].length) LOG_RE.lastIndex++;
    }
    return { tokens: out, state: null };
  }
  T.log = log;

  function diff(text) {
    const n = text.length;
    if (!n) return { tokens: [], state: null };
    const line = (type) => ({ tokens: [[0, n, type]], state: null });
    if (/^(?:diff |index |similarity |dissimilarity |rename |copy |new file|deleted file|old mode|new mode|Binary files|Only in )/.test(text)) return line("keyword");
    if (/^(?:\+\+\+|---)(?: |$)/.test(text) || text.startsWith("*** ")) return line("meta");
    if (text.startsWith("@@")) {
      const k = text.indexOf("@@", 2);
      return { tokens: [[0, k < 0 ? n : k + 2, "meta"]], state: null };
    }
    const c = text[0];
    if (c === "+" || c === ">") return line("inserted");
    if (c === "-" || c === "<") return line("deleted");
    if (c === "!") return line("meta");
    if (c === "\\") return line("comment");
    return { tokens: [], state: null };
  }
  T.diff = diff;

  function ignore(text) {
    const out = [];
    const n = text.length;
    let first = 0;
    while (first < n && text.charCodeAt(first) === 32) first++;
    if (text.charCodeAt(first) === 35) return { tokens: [[first, n, "comment"]], state: null };
    for (let i = first; i < n; i++) {
      const c = text[i];
      if (c === "\\") { push(out, i, Math.min(n, i + 2), "escape"); i++; }
      else if (c === "!" && i === first) push(out, i, i + 1, "operator");
      else if (c === "*" || c === "?" || c === "[" || c === "]") {
        let e = i + 1;
        while (e < n && (text[e] === "*" || text[e] === "?")) e++;
        push(out, i, e, "operator");
        i = e - 1;
      }
    }
    return { tokens: out, state: null };
  }
  T.ignore = ignore;

  // ------------------------------------------------------------------ LaTeX
  // State: null, or "m<closer>" inside display math that started on an earlier line.
  const TEX_CMD = /\\(?:[A-Za-z@]+\*?|[^A-Za-z@])/y;
  function latex(text, state) {
    const out = [];
    const n = text.length;
    let i = 0;
    if (state) {
      const close = state.slice(1);
      const k = text.indexOf(close);
      const e = k < 0 ? n : k + close.length;
      push(out, 0, e, "string");
      if (k < 0) return { tokens: out, state };
      i = e;
    }
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 37) { push(out, i, n, "comment"); break; }
      if (c === 92) {
        if (text.startsWith("\\[", i) || text.startsWith("\\(", i)) {
          const close = text[i + 1] === "[" ? "\\]" : "\\)";
          const k = text.indexOf(close, i + 2);
          const e = k < 0 ? n : k + 2;
          push(out, i, e, "string");
          if (k < 0) return { tokens: out, state: close === "\\]" ? "m\\]" : null };
          i = e;
          continue;
        }
        TEX_CMD.lastIndex = i;
        TEX_CMD.test(text);
        const e = Math.max(TEX_CMD.lastIndex, i + 1);
        const w = text.slice(i, e);
        push(out, i, e, /^\\[A-Za-z@]/.test(w) ? "keyword" : "escape");
        i = e;
        if ((w === "\\begin" || w === "\\end") && text.charCodeAt(i) === 123) {
          const k = text.indexOf("}", i);
          if (k > 0) { push(out, i + 1, k, "type"); i = k + 1; }
        }
        continue;
      }
      if (c === 36) {
        const dbl = text.charCodeAt(i + 1) === 36;
        const close = dbl ? "$$" : "$";
        let k = text.indexOf(close, i + close.length);
        while (k > 0 && text.charCodeAt(k - 1) === 92) k = text.indexOf(close, k + 1);
        const e = k < 0 ? n : k + close.length;
        push(out, i, e, "string");
        if (k < 0 && dbl) return { tokens: out, state: "m$$" };
        i = e;
        continue;
      }
      if (c === 38 || c === 126 || c === 94 || c === 95) push(out, i, i + 1, "operator");
      i++;
    }
    return { tokens: out, state: null };
  }
  T.latex = latex;

  // ------------------------------------------------------------------ shell
  // State: null | "'" / '"' inside a quoted string | "h<->:<delimiter>" inside a here-doc.
  const SH_CONTROL = words("if then else elif fi for while until do done case esac");
  const SH_KEYWORD = words("function select time coproc in");
  const SH_BUILTIN = words("echo cd export local return source alias set unset read printf test exit eval exec shift trap declare typeset readonly let break continue wait kill pwd pushd popd dirs getopts command type hash ulimit umask bg fg jobs disown shopt complete mapfile readarray builtin enable logout true false");
  const SH_VAR = /\$(?:\{[^}]*\}?|[A-Za-z_]\w*|[0-9@#?$!*\-])/y;
  const MAKE_VAR = /\$[@<^+?*%|]/y;
  const SH_WORD = /[^\s'"`$|&;<>()#\\]+/y;
  const SH_ASSIGN = /[A-Za-z_]\w*(?=\+?=)/y;
  const HEREDOC = /<<(-?)\s*(['"]?)([A-Za-z_][\w.-]*)\2/y;
  /** "$(" … ")" with balanced parentheses, for make variables and functions. */
  function balanced(text, i) {
    const open = text[i + 1];
    const close = open === "(" ? ")" : "}";
    let depth = 0;
    for (let j = i + 1; j < text.length; j++) {
      if (text[j] === open) depth++;
      else if (text[j] === close && --depth === 0) return j + 1;
    }
    return text.length;
  }
  function shDouble(text, start, i, out, make) {
    const n = text.length;
    let s = start;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 92) { push(out, s, i, "string"); push(out, i, Math.min(n, i + 2), "escape"); i = s = Math.min(n, i + 2); continue; }
      if (c === 36) {
        const e = shVariable(text, i, make);
        if (e > i) { push(out, s, i, "string"); push(out, i, e, "variable"); i = s = e; continue; }
      }
      if (c === 34) { push(out, s, i + 1, "string"); return [i + 1, true]; }
      i++;
    }
    push(out, s, n, "string");
    return [n, false];
  }
  function shVariable(text, i, make) {
    if (make) {
      const nx = text.charCodeAt(i + 1);
      if (nx === 40 || nx === 123) return balanced(text, i);
      if (nx === 36) return -1;
      MAKE_VAR.lastIndex = i;
      if (MAKE_VAR.test(text)) return MAKE_VAR.lastIndex;
    }
    if (text.charCodeAt(i + 1) === 40) return -1;
    SH_VAR.lastIndex = i;
    return SH_VAR.test(text) ? SH_VAR.lastIndex : -1;
  }
  function shell(text, state, make) {
    const out = [];
    const n = text.length;
    let i = 0;
    if (state) {
      if (state[0] === "h") {
        const k = state.indexOf(":");
        const delim = state.slice(k + 1);
        const body = state[1] === "-" ? text.replace(/^\t+/, "") : text;
        if (body === delim) return { tokens: [[0, n, "keyword"]], state: null };
        return { tokens: n ? [[0, n, "string"]] : [], state };
      }
      if (state === "'") {
        const k = text.indexOf("'");
        push(out, 0, k < 0 ? n : k + 1, "string");
        if (k < 0) return { tokens: out, state };
        i = k + 1;
      } else if (state === '"') {
        const r = shDouble(text, 0, 0, out, make);
        if (!r[1]) return { tokens: out, state };
        i = r[0];
      }
    }
    let command = true; // the next word is in command position
    let heredoc = null;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 35 && (i === 0 || /[\s;|&(]/.test(text[i - 1]))) { push(out, i, n, "comment"); break; }
      if (c === 39) {
        const k = text.indexOf("'", i + 1);
        push(out, i, k < 0 ? n : k + 1, "string");
        if (k < 0) return { tokens: out, state: "'" };
        i = k + 1;
        command = false;
        continue;
      }
      if (c === 36 && text.charCodeAt(i + 1) === 39) {
        i = stringBody(text, i, i + 2, "'", true, out, false)[0];
        command = false;
        continue;
      }
      if (c === 34) {
        const r = shDouble(text, i, i + 1, out, make);
        if (!r[1]) return { tokens: out, state: '"' };
        i = r[0];
        command = false;
        continue;
      }
      if (c === 36) {
        if (make && text.charCodeAt(i + 1) === 36) { push(out, i, i + 2, "escape"); i += 2; continue; }
        const e = shVariable(text, i, make);
        if (e > i) { push(out, i, e, "variable"); i = e; command = false; continue; }
        if (text.charCodeAt(i + 1) === 40) {
          const e2 = text.charCodeAt(i + 2) === 40 ? i + 3 : i + 2;
          push(out, i, e2, "operator");
          i = e2;
          command = true;
          continue;
        }
      }
      if (c === 92) { push(out, i, Math.min(n, i + 2), "escape"); i += 2; continue; }
      if (c === 60 && text.charCodeAt(i + 1) === 60 && text.charCodeAt(i + 2) !== 60) {
        HEREDOC.lastIndex = i;
        const m = HEREDOC.exec(text);
        if (m) {
          push(out, i, HEREDOC.lastIndex, "operator");
          heredoc = `h${m[1]}:${m[3]}`;
          i = HEREDOC.lastIndex;
          continue;
        }
      }
      if (c === 124 || c === 38 || c === 59 || c === 60 || c === 62 || c === 40 || c === 41 || c === 96) {
        let e = i + 1;
        while (e < n && /[|&;<>]/.test(text[e])) e++;
        push(out, i, e, "operator");
        command = c !== 62 && c !== 60 && c !== 41;
        i = e;
        continue;
      }
      if (command) {
        SH_ASSIGN.lastIndex = i;
        if (SH_ASSIGN.test(text)) {
          push(out, i, SH_ASSIGN.lastIndex, "variable");
          i = SH_ASSIGN.lastIndex;
          const e = text.charCodeAt(i) === 43 ? i + 2 : i + 1;
          push(out, i, e, "operator");
          i = e;
          continue;
        }
      }
      SH_WORD.lastIndex = i;
      if (SH_WORD.test(text)) {
        const e = SH_WORD.lastIndex;
        const w = text.slice(i, e);
        let type = null;
        if (SH_CONTROL.has(w) && command) type = "control";
        else if (SH_KEYWORD.has(w) && (command || w === "in")) type = "keyword";
        else if (/^\(\)/.test(text.slice(e).trimStart()) && /^[\w.:-]+$/.test(w)) type = "function";
        else if (command && SH_BUILTIN.has(w)) type = "function";
        else if (/^\d+$/.test(w)) type = "number";
        if (type) push(out, i, e, type);
        command = type === "control" || type === "keyword";
        if (w === "function") command = false;
        i = e;
        continue;
      }
      i++;
    }
    return { tokens: out, state: heredoc };
  }
  T.shell = (text, state) => shell(text, state, false);

  // ------------------------------------------------------------------ PowerShell
  // State: null | "c" in <# #> | 'h"' / "h'" in a here-string | '"' / "'" in a string.
  const PS_CONTROL = lowerWords("if else elseif switch for foreach while do until break continue return throw try catch finally trap exit");
  const PS_KEYWORD = lowerWords("begin data define dynamicparam end enum filter from function hidden in param process static using var workflow parallel sequence inlinescript configuration class");
  const PS_VAR = /\$(?:\{[^}]*\}?|[\p{L}\p{N}_]+(?::[\p{L}\p{N}_]+)?|[?$^_])|@[\p{L}_][\p{L}\p{N}_]*/uy;
  const PS_OP = /-(?:eq|ne|gt|ge|lt|le|like|notlike|match|notmatch|contains|notcontains|in|notin|replace|split|join|and|or|not|xor|band|bor|bxor|bnot|shl|shr|is|isnot|as|f|c(?:eq|ne|gt|ge|lt|le|like|match|replace|contains)|i(?:eq|ne|gt|ge|lt|le|like|match|replace|contains))(?![\w-])/iy;
  const PS_CMDLET = /[A-Za-z]+-[A-Za-z][A-Za-z0-9]*/y;
  const PS_TYPE = /\[[A-Za-z_][\w.`]*(?:\[[^\]]*\])?\]/y;
  const PS_WORD = /[\p{L}_][\p{L}\p{N}_]*/uy;
  const PS_NUM = /(?:0x[0-9a-fA-F]+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)(?:kb|mb|gb|tb|pb|[ldLD])?(?![\w])/y;
  function psString(text, start, i, q, out) {
    const n = text.length;
    let s = start;
    while (i < n) {
      const c = text[i];
      if (q === '"' && c === "`") { push(out, s, i, "string"); push(out, i, Math.min(n, i + 2), "escape"); i = s = Math.min(n, i + 2); continue; }
      if (q === '"' && c === "$") {
        PS_VAR.lastIndex = i;
        if (PS_VAR.test(text)) { push(out, s, i, "string"); push(out, i, PS_VAR.lastIndex, "variable"); i = s = PS_VAR.lastIndex; continue; }
      }
      if (c === q) {
        if (text[i + 1] === q) { i += 2; continue; }
        push(out, s, i + 1, "string");
        return [i + 1, true];
      }
      i++;
    }
    push(out, s, n, "string");
    return [n, false];
  }
  function powershell(text, state) {
    const out = [];
    const n = text.length;
    let i = 0;
    if (state === "c") {
      const k = text.indexOf("#>");
      push(out, 0, k < 0 ? n : k + 2, "comment");
      if (k < 0) return { tokens: out, state };
      i = k + 2;
    } else if (state && state[0] === "h") {
      const q = state[1];
      if (text.startsWith(q + "@")) { push(out, 0, 2, "string"); i = 2; }
      else return { tokens: n ? [[0, n, "string"]] : [], state };
    } else if (state) {
      const r = psString(text, 0, 0, state, out);
      if (!r[1]) return { tokens: out, state };
      i = r[0];
    }
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 60 && text.charCodeAt(i + 1) === 35) {
        const k = text.indexOf("#>", i + 2);
        push(out, i, k < 0 ? n : k + 2, "comment");
        if (k < 0) return { tokens: out, state: "c" };
        i = k + 2;
        continue;
      }
      if (c === 35) { push(out, i, n, "comment"); break; }
      if (c === 64 && (text[i + 1] === '"' || text[i + 1] === "'") && /^\s*$/.test(text.slice(i + 2))) {
        push(out, i, n, "string");
        return { tokens: out, state: "h" + text[i + 1] };
      }
      if (c === 34 || c === 39) {
        const r = psString(text, i, i + 1, text[i], out);
        if (!r[1]) return { tokens: out, state: text[i] };
        i = r[0];
        continue;
      }
      if (c === 36 || c === 64) {
        PS_VAR.lastIndex = i;
        if (PS_VAR.test(text)) { push(out, i, PS_VAR.lastIndex, "variable"); i = PS_VAR.lastIndex; continue; }
        if (c === 64) { push(out, i, i + 1, "operator"); i++; continue; }
      }
      if (c === 45) {
        PS_OP.lastIndex = i;
        if (PS_OP.test(text)) { push(out, i, PS_OP.lastIndex, "operator"); i = PS_OP.lastIndex; continue; }
      }
      if (c === 91) {
        PS_TYPE.lastIndex = i;
        if (PS_TYPE.test(text)) { push(out, i, PS_TYPE.lastIndex, "type"); i = PS_TYPE.lastIndex; continue; }
      }
      if (isDigit(c)) {
        PS_NUM.lastIndex = i;
        if (PS_NUM.test(text)) { push(out, i, PS_NUM.lastIndex, "number"); i = PS_NUM.lastIndex; continue; }
      }
      PS_CMDLET.lastIndex = i;
      if (/[A-Za-z]/.test(text[i]) && PS_CMDLET.test(text)) { push(out, i, PS_CMDLET.lastIndex, "function"); i = PS_CMDLET.lastIndex; continue; }
      PS_WORD.lastIndex = i;
      if (PS_WORD.test(text)) {
        const w = text.slice(i, PS_WORD.lastIndex).toLowerCase();
        const type = PS_CONTROL.has(w) ? "control" : PS_KEYWORD.has(w) ? "keyword" : w === "true" || w === "false" || w === "null" ? "constant" : null;
        if (type) push(out, i, PS_WORD.lastIndex, type);
        else if (text.charCodeAt(PS_WORD.lastIndex) === 40) push(out, i, PS_WORD.lastIndex, "function");
        i = PS_WORD.lastIndex;
        continue;
      }
      if (/[=+*\/%!<>|&,.]/.test(text[i]) && c !== 46 && c !== 44) push(out, i, i + 1, "operator");
      i++;
    }
    return { tokens: mergeComment(out), state: null };
  }
  T.powershell = powershell;

  // ------------------------------------------------------------------ Windows batch
  const BAT_CONTROL = lowerWords("if else for in do goto call exit not exist defined errorlevel equ neq lss leq gtr geq");
  const BAT_KEYWORD = lowerWords("echo set setlocal endlocal shift pause cls start cd chdir pushd popd title color mkdir md rmdir rd del erase copy move ren rename type cmd choice timeout enabledelayedexpansion enableextensions disabledelayedexpansion disableextensions off on nul con prn aux ver vol path prompt assoc ftype");
  const BAT_VAR = /%%~?[a-zA-Z]|%~[a-zA-Z$:]*\d|%\d|%[^%\s"]+%|![^!\s"]+!/y;
  const BAT_WORD = /[A-Za-z_][\w.-]*/y;
  function batch(text) {
    const out = [];
    const n = text.length;
    let i = 0;
    while (i < n && (text.charCodeAt(i) === 32 || text.charCodeAt(i) === 9)) i++;
    if (text.charCodeAt(i) === 64) { push(out, i, i + 1, "operator"); i++; }
    if (/^rem(\s|$)/i.test(text.slice(i, i + 4))) return { tokens: [...out, [i, n, "comment"]], state: null };
    if (text.startsWith("::", i)) return { tokens: [[i, n, "comment"]], state: null };
    if (text.charCodeAt(i) === 58) {
      const m = /:[^\s:]+/y;
      m.lastIndex = i;
      m.test(text);
      push(out, i, m.lastIndex, "label");
      return { tokens: out, state: null };
    }
    let echo = false;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 37 || c === 33) {
        BAT_VAR.lastIndex = i;
        if (BAT_VAR.test(text)) { push(out, i, BAT_VAR.lastIndex, "variable"); i = BAT_VAR.lastIndex; continue; }
      }
      if (c === 34) {
        const k = text.indexOf('"', i + 1);
        const e = k < 0 ? n : k + 1;
        // Variables stay visible inside quotes.
        const inner = [];
        for (let j = i; j < e; ) {
          BAT_VAR.lastIndex = j;
          if ((text[j] === "%" || text[j] === "!") && BAT_VAR.test(text)) { inner.push([j, BAT_VAR.lastIndex, "variable"]); j = BAT_VAR.lastIndex; }
          else j++;
        }
        for (const t of fill(inner, i, e, "string")) out.push(t);
        i = e;
        continue;
      }
      if (/[|&<>()]/.test(text[i])) {
        push(out, i, i + 1, "operator");
        if (c === 38 || c === 124 || c === 40) echo = false;
        i++;
        continue;
      }
      if (echo) { i++; continue; }
      BAT_WORD.lastIndex = i;
      if (BAT_WORD.test(text)) {
        const w = text.slice(i, BAT_WORD.lastIndex).toLowerCase();
        if (BAT_CONTROL.has(w)) push(out, i, BAT_WORD.lastIndex, "control");
        else if (BAT_KEYWORD.has(w)) push(out, i, BAT_WORD.lastIndex, "keyword");
        else if (/^\d+$/.test(w)) push(out, i, BAT_WORD.lastIndex, "number");
        if (w === "echo") echo = true;
        i = BAT_WORD.lastIndex;
        continue;
      }
      if (isDigit(c)) {
        let e = i;
        while (isDigit(text.charCodeAt(e))) e++;
        push(out, i, e, "number");
        i = e;
        continue;
      }
      i++;
    }
    return { tokens: out, state: null };
  }
  T.batch = batch;

  // ------------------------------------------------------------------ Dockerfile, Makefile, CMake
  const DF_INSTR = /^(\s*)(FROM|RUN|CMD|LABEL|MAINTAINER|EXPOSE|ENV|ADD|COPY|ENTRYPOINT|VOLUME|USER|WORKDIR|ARG|ONBUILD|STOPSIGNAL|HEALTHCHECK|SHELL)(?=\s|$)/i;
  // State: null, or "c:<shell state>" on a line continued with a trailing backslash.
  function dockerfile(text, state) {
    const out = [];
    const cont = /\\\s*$/.test(text);
    const heredoc = !!state && state[2] === "h";
    if (!heredoc && /^\s*#/.test(text)) {
      const first = text.search(/\S/);
      const directive = !state && /^\s*#\s*(syntax|escape|check)\s*=/.test(text);
      return { tokens: [[first, text.length, directive ? "meta" : "comment"]], state: state || null };
    }
    let i = 0;
    let inner = state ? state.slice(2) || null : null;
    if (!state) {
      const m = DF_INSTR.exec(text);
      if (m) {
        push(out, m[1].length, m[0].length, "keyword");
        i = m[0].length;
        if (/^from$/i.test(m[2])) {
          const as = /\s(as)\s/i.exec(text.slice(i));
          if (as) {
            const r = shell(text.slice(i, i + as.index), null, false);
            append(out, r.tokens, i);
            push(out, i + as.index + 1, i + as.index + 3, "keyword");
            i += as.index + 3;
          }
        }
      }
    }
    const r = shell(text.slice(i), inner, false);
    append(out, r.tokens, i);
    inner = r.state;
    return { tokens: out, state: cont || inner ? "c:" + (inner || "") : null };
  }
  T.dockerfile = dockerfile;

  const MAKE_DIRECTIVE = /^(\s*)(-?include|sinclude|ifeq|ifneq|ifdef|ifndef|else|endif|define|endef|export|unexport|override|vpath|undefine|private)(?=\s|$)/;
  const MAKE_ASSIGN = /^(\s*)((?:[\w.\-]|\$\([^)]*\)|\$\{[^}]*\})+)\s*(\?=|:::=|::=|:=|\+=|!=|=)/;
  const MAKE_RULE = /^([^\s:=#][^:=#]*?)\s*(::?)(?![=])/;
  /** Variables and comments in make text (values, prerequisites, define bodies). */
  function makeText(text, i, out) {
    const n = text.length;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 35 && text.charCodeAt(i - 1) !== 92) { push(out, i, n, "comment"); break; }
      if (c === 36) {
        if (text.charCodeAt(i + 1) === 36) { push(out, i, i + 2, "escape"); i += 2; continue; }
        const e = shVariable(text, i, true);
        if (e > i) { push(out, i, e, "variable"); i = e; continue; }
      }
      if (c === 34 || c === 39) {
        const k = text.indexOf(text[i], i + 1);
        if (k > 0) { push(out, i, k + 1, "string"); i = k + 1; continue; }
      }
      i++;
    }
    return out;
  }
  // State: null, or "d" inside a define … endef block.
  function makefile(text, state) {
    const out = [];
    const n = text.length;
    if (state === "d") {
      if (/^\s*endef\b/.test(text)) return { tokens: [[text.search(/\S/), text.search(/\S/) + 5, "keyword"]], state: null };
      return { tokens: makeText(text, 0, out), state };
    }
    if (text.charCodeAt(0) === 9) {
      let i = 1;
      while (i < n && /[@\-+]/.test(text[i])) { push(out, i, i + 1, "operator"); i++; }
      const r = shell(text.slice(i), null, true);
      append(out, r.tokens, i);
      return { tokens: out, state: null };
    }
    if (/^\s*#/.test(text)) return { tokens: [[text.search(/\S/), n, "comment"]], state: null };
    let m = MAKE_DIRECTIVE.exec(text);
    if (m) {
      push(out, m[1].length, m[0].length, "keyword");
      makeText(text, m[0].length, out);
      return { tokens: out, state: m[2] === "define" ? "d" : null };
    }
    m = MAKE_ASSIGN.exec(text);
    if (m) {
      push(out, m[1].length, m[1].length + m[2].length, "variable");
      push(out, m[0].length - m[3].length, m[0].length, "operator");
      return { tokens: makeText(text, m[0].length, out), state: null };
    }
    m = MAKE_RULE.exec(text);
    if (m) {
      const targets = [];
      makeText(text.slice(0, m[1].length), 0, targets);
      for (const t of fill(targets, 0, m[1].length, "function")) out.push(t);
      push(out, m[0].length - m[2].length, m[0].length, "operator");
      const semi = text.indexOf(";", m[0].length);
      if (semi > 0) {
        makeText(text.slice(0, semi), m[0].length, out);
        push(out, semi, semi + 1, "operator");
        const r = shell(text.slice(semi + 1), null, true);
        append(out, r.tokens, semi + 1);
      } else makeText(text, m[0].length, out);
      return { tokens: out, state: null };
    }
    return { tokens: makeText(text, 0, out), state: null };
  }
  T.makefile = makefile;

  // CMake state: null | "c<n>" bracket comment | "b<n>" bracket argument | "s" quoted argument.
  const CM_CONTROL = lowerWords("if elseif else endif foreach endforeach while endwhile function endfunction macro endmacro return break continue block endblock");
  const CM_VAR = /\$(?:ENV|CACHE)?\{[^}]*\}?|\$<[^>]*>?/y;
  const CM_WORD = /[A-Za-z_][\w.+-]*/y;
  const CM_BRACKET = /\[(=*)\[/y;
  function cmake(text, state) {
    const out = [];
    const n = text.length;
    let i = 0;
    const bracket = (start, eqs, type, at) => {
      const close = "]" + "=".repeat(eqs) + "]";
      const k = text.indexOf(close, at);
      const e = k < 0 ? n : k + close.length;
      push(out, start, e, type);
      return [e, k >= 0];
    };
    const quoted = (start, at) => {
      let s = start;
      let j = at;
      while (j < n) {
        const c = text.charCodeAt(j);
        if (c === 92) { push(out, s, j, "string"); push(out, j, Math.min(n, j + 2), "escape"); j = s = Math.min(n, j + 2); continue; }
        if (c === 36) {
          CM_VAR.lastIndex = j;
          if (CM_VAR.test(text)) { push(out, s, j, "string"); push(out, j, CM_VAR.lastIndex, "variable"); j = s = CM_VAR.lastIndex; continue; }
        }
        if (c === 34) { push(out, s, j + 1, "string"); return [j + 1, true]; }
        j++;
      }
      push(out, s, n, "string");
      return [n, false];
    };
    if (state) {
      const r = state === "s" ? quoted(0, 0) : bracket(0, +state.slice(1), state[0] === "c" ? "comment" : "string", 0);
      if (!r[1]) return { tokens: out, state };
      i = r[0];
    }
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 32 || c === 9) { i++; continue; }
      if (c === 35) {
        CM_BRACKET.lastIndex = i + 1;
        const m = CM_BRACKET.exec(text);
        if (m) {
          const r = bracket(i, m[1].length, "comment", CM_BRACKET.lastIndex);
          if (!r[1]) return { tokens: out, state: "c" + m[1].length };
          i = r[0];
          continue;
        }
        push(out, i, n, "comment");
        break;
      }
      if (c === 91) {
        CM_BRACKET.lastIndex = i;
        const m = CM_BRACKET.exec(text);
        if (m) {
          const r = bracket(i, m[1].length, "string", CM_BRACKET.lastIndex);
          if (!r[1]) return { tokens: out, state: "b" + m[1].length };
          i = r[0];
          continue;
        }
      }
      if (c === 34) {
        const r = quoted(i, i + 1);
        if (!r[1]) return { tokens: out, state: "s" };
        i = r[0];
        continue;
      }
      if (c === 36) {
        CM_VAR.lastIndex = i;
        if (CM_VAR.test(text)) { push(out, i, CM_VAR.lastIndex, "variable"); i = CM_VAR.lastIndex; continue; }
      }
      if (isDigit(c)) {
        NUMBER.lastIndex = i;
        NUMBER.test(text);
        push(out, i, NUMBER.lastIndex, "number");
        i = Math.max(NUMBER.lastIndex, i + 1);
        continue;
      }
      CM_WORD.lastIndex = i;
      if (CM_WORD.test(text)) {
        const e = CM_WORD.lastIndex;
        const w = text.slice(i, e);
        let j = e;
        while (text.charCodeAt(j) === 32 || text.charCodeAt(j) === 9) j++;
        const lw = w.toLowerCase();
        if (text.charCodeAt(j) === 40) push(out, i, e, CM_CONTROL.has(lw) ? "control" : "function");
        else if (/^(on|off|true|false|yes|no|y|n|notfound|ignore)$/i.test(w) || /-notfound$/i.test(w)) push(out, i, e, "constant");
        else if (/^[A-Z][A-Z0-9_]+$/.test(w)) push(out, i, e, "keyword");
        i = e;
        continue;
      }
      i++;
    }
    return { tokens: out, state: null };
  }
  T.cmake = cmake;

  // ------------------------------------------------------------------ language table
  const BR = [["(", ")"], ["[", "]"], ["{", "}"]];
  const AC = [["(", ")"], ["[", "]"], ["{", "}"], ['"', '"'], ["'", "'"]];
  const AC_TICK = [...AC, ["`", "`"]];
  const AC_DQ = [["(", ")"], ["[", "]"], ["{", "}"], ['"', '"']];
  const C_INDENT = /[{[(]\s*(\/\/.*|\/\*.*\*\/\s*)?$/;
  const HASH_INDENT = /[{[(]\s*(#.*)?$/;
  const C_OUTDENT = /^\s*[}\])]$/;
  const TAG_INDENT = /<(?!(?:area|base|br|col|embed|hr|img|input|link|meta|param|source|track|wbr)\b)[A-Za-z][\w:.-]*(?:\s[^<>]*[^<>\/])?>[^<]*$/i;
  const TAG_OUTDENT = /^\s*<\/[\w:.-]+>$/;
  const DASH_WORD = /[\p{L}\p{N}_-]/u;
  const C_STYLE = { lineComment: "//", blockComment: ["/*", "*/"], indentAfter: C_INDENT, outdent: C_OUTDENT };
  const HASH_STYLE = { lineComment: "#", indentAfter: HASH_INDENT, outdent: C_OUTDENT };

  const L = (id, name, text, color, o = {}) => ({
    id, name, exts: [], names: [], aliases: [], brackets: BR, autoClose: AC, fold: "brackets", icon: { text, color }, ...o,
  });
  const LANGS = [
    L("plaintext", "Plain Text", "TXT", "#9aa4b2", { exts: [".txt", ".text"], aliases: ["text", "txt", "plain"], autoClose: [], fold: "indent" }),
    L("asm", "Assembly", "ASM", "#b88a3e", { exts: [".asm", ".s", ".nasm"], aliases: ["assembly", "nasm", "x86"], lineComment: ";", autoClose: AC_DQ, fold: "indent" }),
    L("batch", "Batch", "BAT", "#a9c938", { exts: [".bat", ".cmd"], aliases: ["bat", "cmd", "dos"], lineComment: "REM", autoClose: AC_DQ, indentAfter: /\(\s*$/, outdent: /^\s*\)$/ }),
    L("c", "C", "C", "#6f8fca", { exts: [".c", ".h"], ...C_STYLE }),
    L("cmake", "CMake", "CM", "#da3434", { exts: [".cmake"], names: ["CMakeLists.txt"], lineComment: "#", autoClose: AC_DQ, fold: "indent",
      indentAfter: /^\s*(if|elseif|else|foreach|while|function|macro|block)\s*\(.*\)\s*(#.*)?$|\(\s*$/i, outdent: /^\s*(endif|endforeach|endwhile|endfunction|endmacro|endblock|else|elseif)\s*\(/i }),
    L("cpp", "C++", "C++", "#f34b7d", { exts: [".cpp", ".cc", ".cxx", ".c++", ".hpp", ".hh", ".hxx", ".h++", ".ino", ".ipp", ".tpp", ".cu"], aliases: ["c++", "cxx", "cplusplus"], ...C_STYLE }),
    L("csharp", "C#", "C#", "#3fae49", { exts: [".cs", ".csx"], aliases: ["cs", "c#"], ...C_STYLE }),
    L("css", "CSS", "#", "#8a5cc7", { exts: [".css"], blockComment: ["/*", "*/"], indentAfter: C_INDENT, outdent: C_OUTDENT, wordChars: DASH_WORD }),
    L("csv", "CSV", "CSV", "#3a9a5b", { exts: [".csv"], autoClose: [['"', '"']], fold: "indent" }),
    L("dart", "Dart", "DT", "#00b4ab", { exts: [".dart"], ...C_STYLE }),
    L("diff", "Diff", "+-", "#56b6c2", { exts: [".diff", ".patch", ".rej"], aliases: ["patch", "udiff"], autoClose: [], fold: "indent" }),
    L("dockerfile", "Dockerfile", "DK", "#2496ed", { exts: [".dockerfile"], names: ["Dockerfile", "Containerfile"], aliases: ["docker", "containerfile"], ...HASH_STYLE, fold: "indent" }),
    L("go", "Go", "GO", "#00add8", { exts: [".go"], aliases: ["golang"], ...C_STYLE, autoClose: AC_TICK, useTabs: true, tabSize: 4 }),
    L("groovy", "Groovy", "GR", "#4298b8", { exts: [".groovy", ".gradle", ".gvy", ".gy", ".gsh"], names: ["Jenkinsfile"], aliases: ["gradle", "jenkins"], ...C_STYLE }),
    L("haskell", "Haskell", "HS", "#8a7bc0", { exts: [".hs", ".lhs"], aliases: ["hs"], lineComment: "--", blockComment: ["{-", "-}"], fold: "indent", indentAfter: /(\b(where|do|of|let|in)|=|->)\s*$/ }),
    L("html", "HTML", "<>", "#e34c26", { exts: [".html", ".htm", ".xhtml", ".xht", ".shtml", ".hta"], aliases: ["htm", "xhtml"], blockComment: ["<!--", "-->"],
      brackets: [["<", ">"], ...BR], fold: "indent", indentAfter: TAG_INDENT, outdent: TAG_OUTDENT, wordChars: DASH_WORD }),
    L("ignore", "Ignore", "GI", "#f05133", { names: [".gitignore", ".dockerignore", ".npmignore", ".eslintignore", ".prettierignore", ".hgignore", ".vscodeignore", ".stylelintignore", ".helmignore", ".gcloudignore", ".slugignore"], aliases: ["gitignore", "ignore"], lineComment: "#", autoClose: [], fold: "indent" }),
    L("ini", "INI", "INI", "#8d99a6", { exts: [".ini", ".cfg", ".conf", ".cnf", ".properties", ".desktop", ".service", ".socket", ".timer", ".target", ".mount", ".path", ".slice", ".inf", ".reg", ".url", ".theme", ".directory"],
      names: [".editorconfig", ".gitconfig", ".gitmodules", ".npmrc", ".pypirc", ".hgrc", ".flake8", ".pylintrc", "pylintrc", "setup.cfg", "tox.ini", "php.ini", "my.cnf", "desktop.ini", ".wgetrc", ".curlrc"],
      aliases: ["conf", "properties", "editorconfig", "systemd", "desktop", "registry"], lineComment: ";", autoClose: AC_DQ, fold: "indent" }),
    L("java", "Java", "J", "#b07219", { exts: [".java", ".jav"], ...C_STYLE }),
    L("javascript", "JavaScript", "JS", "#f1e05a", { exts: [".js", ".mjs", ".cjs", ".jsx", ".es6", ".jsm"], names: [".eslintrc.js", ".prettierrc.js"], aliases: ["js", "jsx", "node", "mjs", "cjs", "ecmascript"], ...C_STYLE, autoClose: AC_TICK }),
    L("json", "JSON", "{}", "#cbcb41", { exts: [".json", ".webmanifest", ".code-workspace", ".geojson", ".har", ".jsonl", ".ndjson", ".map", ".gltf", ".ipynb"], names: [".prettierrc", ".watchmanconfig", "composer.lock", "package-lock.json"], aliases: ["json5"], autoClose: AC_DQ, indentAfter: C_INDENT, outdent: C_OUTDENT }),
    L("jsonc", "JSON with Comments", "{}", "#cbcb41", { exts: [".jsonc"], names: ["tsconfig.json", "jsconfig.json", ".eslintrc", ".eslintrc.json", ".babelrc", ".babelrc.json", ".jshintrc", ".jscsrc", ".swcrc", ".devcontainer.json", "devcontainer.json", "settings.json", "keybindings.json", "launch.json", "tasks.json", "extensions.json", "argv.json", "typedoc.json", "api-extractor.json", "language-configuration.json"],
      aliases: ["json-with-comments"], ...C_STYLE, autoClose: AC_DQ }),
    L("kotlin", "Kotlin", "KT", "#a97bff", { exts: [".kt", ".kts", ".ktm"], aliases: ["kt", "kts"], ...C_STYLE }),
    L("latex", "LaTeX", "TEX", "#7aa64a", { exts: [".tex", ".sty", ".cls", ".ltx", ".dtx", ".ins"], aliases: ["tex"], lineComment: "%", autoClose: [["{", "}"], ["[", "]"], ["(", ")"], ["$", "$"]], fold: "indent", indentAfter: /\\begin\{[^}]*\}[^\\]*$|\{\s*$/, outdent: /^\s*\\end\{[^}]*\}$/ }),
    L("log", "Log", "LOG", "#8fa0b5", { exts: [".log"], aliases: ["logs"], autoClose: [], fold: "indent" }),
    L("lua", "Lua", "LUA", "#6b7ff0", { exts: [".lua", ".luau", ".rockspec"], lineComment: "--", blockComment: ["--[[", "]]"], fold: "indent",
      indentAfter: /^\s*((local\s+)?function\b.*|.*\b(then|do|repeat)|.*\bfunction\s*\([^)]*\)|else)\s*(--.*)?$|[{[(]\s*$/, outdent: /^\s*(end|else|elseif\b.*|until\b.*|\})$/ }),
    L("makefile", "Makefile", "MK", "#6d9b3a", { exts: [".mk", ".mak", ".make"], names: ["Makefile", "makefile", "GNUmakefile", "BSDmakefile", "Kbuild", "Makefile.am", "Makefile.in"], aliases: ["make", "mk", "bsdmake"], lineComment: "#", autoClose: AC_DQ, fold: "indent", useTabs: true }),
    L("markdown", "Markdown", "MD", "#519aba", { exts: [".md", ".markdown", ".mdown", ".mkd", ".mkdn", ".mdwn", ".mdx", ".ronn", ".workbook"], aliases: ["md", "mdown", "gfm"], blockComment: ["<!--", "-->"],
      autoClose: [["(", ")"], ["[", "]"], ["{", "}"], ["`", "`"]], fold: "markdown" }),
    L("objective-c", "Objective-C", "OC", "#438eff", { exts: [".m", ".mm"], aliases: ["objc", "obj-c", "objectivec", "objective-cpp"], ...C_STYLE }),
    L("perl", "Perl", "PL", "#39a5d5", { exts: [".pl", ".pm", ".t", ".pod", ".perl", ".psgi"], aliases: ["pl", "pm"], ...HASH_STYLE }),
    L("php", "PHP", "PHP", "#7a86b8", { exts: [".php", ".phtml", ".php3", ".php4", ".php5", ".php7", ".phps", ".ctp"], ...C_STYLE }),
    L("powershell", "PowerShell", "PS", "#5391fe", { exts: [".ps1", ".psm1", ".psd1", ".pssc", ".psrc"], aliases: ["ps1", "pwsh", "ps", "posh"], lineComment: "#", blockComment: ["<#", "#>"], indentAfter: HASH_INDENT, outdent: C_OUTDENT, wordChars: DASH_WORD }),
    L("python", "Python", "PY", "#3572a5", { exts: [".py", ".pyw", ".pyi", ".pyx", ".pxd", ".gyp", ".gypi", ".rpy", ".pyt"], names: ["SConstruct", "SConscript", "BUILD.bazel", "WORKSPACE", ".pythonrc", "Snakefile"], aliases: ["py", "python3", "py3", "gyp"],
      lineComment: "#", fold: "indent", indentAfter: /:\s*(#.*)?$|[{[(]\s*$/, outdent: /^\s*[}\])]$/ }),
    L("r", "R", "R", "#198ce7", { exts: [".r", ".rhistory", ".rprofile"], names: [".Rprofile"], aliases: ["rscript", "splus"], ...HASH_STYLE }),
    L("ruby", "Ruby", "RB", "#cc342d", { exts: [".rb", ".rake", ".gemspec", ".ru", ".rbw", ".podspec", ".jbuilder", ".thor"], names: ["Rakefile", "Gemfile", "Guardfile", "Podfile", "Vagrantfile", "Brewfile", "Capfile", "Fastfile", ".irbrc", ".pryrc"], aliases: ["rb", "jruby"],
      lineComment: "#", blockComment: ["=begin", "=end"], fold: "indent",
      indentAfter: /^\s*((def|class|module|if|unless|while|until|for|case|begin|else|elsif|when|rescue|ensure)\b(?!.*\bend\s*$).*|.*\bdo(\s*\|[^|]*\|)?)\s*(#.*)?$|[{[(]\s*$/, outdent: /^\s*(end|else|elsif\b.*|when\b.*|rescue\b.*|ensure|\})$/ }),
    L("rust", "Rust", "RS", "#dea584", { exts: [".rs"], aliases: ["rs"], ...C_STYLE }),
    L("scala", "Scala", "SC", "#c22d40", { exts: [".scala", ".sc", ".sbt"], ...C_STYLE }),
    L("scss", "SCSS", "S", "#c6538c", { exts: [".scss", ".less"], aliases: ["less", "sass"], ...C_STYLE, wordChars: DASH_WORD }),
    L("shell", "Shell Script", "SH", "#89e051", { exts: [".sh", ".bash", ".zsh", ".ksh", ".command", ".bats", ".tmux", ".ebuild", ".eclass"],
      names: [".bashrc", ".bash_profile", ".bash_logout", ".bash_aliases", ".bash_login", ".zshrc", ".zprofile", ".zshenv", ".zlogin", ".zlogout", ".profile", ".kshrc", ".xinitrc", ".xprofile", "PKGBUILD", "APKBUILD", ".env", ".envrc", "configure", "gradlew", "bashrc", "zshrc", "profile"],
      aliases: ["sh", "bash", "zsh", "ksh", "console", "shellscript", "shell-session", "terminal", "env"], lineComment: "#", autoClose: AC_TICK, fold: "indent",
      indentAfter: /(\b(then|do|else|in)\s*(#.*)?$)|(^\s*(function\s+)?[\w.:-]+\s*\(\)\s*\{?\s*$)|[{(]\s*$/, outdent: /^\s*(fi|done|esac|else|elif\b.*|\}|\)|;;)$/ }),
    L("sql", "SQL", "SQL", "#e38c00", { exts: [".sql", ".ddl", ".dml", ".pgsql", ".psql", ".mysql", ".sqlite", ".hql", ".cql", ".prc", ".tab.sql"], aliases: ["mysql", "postgres", "postgresql", "plsql", "tsql", "sqlite", "pgsql"],
      lineComment: "--", blockComment: ["/*", "*/"], indentAfter: /[([]\s*(--.*)?$|\b(begin|then|else)\s*$/i }),
    L("swift", "Swift", "SW", "#f05138", { exts: [".swift"], ...C_STYLE }),
    L("toml", "TOML", "TML", "#c0703f", { exts: [".toml"], names: ["Cargo.lock", "Pipfile", "poetry.lock", "uv.lock", "Gopkg.lock", "pdm.lock", ".cargo-ok"], lineComment: "#", fold: "indent", indentAfter: /[[{]\s*(#.*)?$/ }),
    L("tsv", "TSV", "TSV", "#3a9a5b", { exts: [".tsv", ".tab"], autoClose: [['"', '"']], fold: "indent" }),
    L("typescript", "TypeScript", "TS", "#3178c6", { exts: [".ts", ".tsx", ".mts", ".cts", ".d.ts", ".d.mts", ".d.cts"], aliases: ["ts", "tsx", "mts", "cts"], ...C_STYLE, autoClose: AC_TICK }),
    L("vb", "Visual Basic", "VB", "#945db7", { exts: [".vb", ".vbs", ".bas", ".vba", ".frm", ".vbhtml"], aliases: ["vbnet", "vbscript", "vba", "visualbasic", "basic"], lineComment: "'", autoClose: AC_DQ, fold: "indent",
      indentAfter: /^\s*(((Public|Private|Protected|Friend|Shared|Overrides|Overridable|Overloads|Static|Async|Partial|MustOverride|NotOverridable|ReadOnly|WriteOnly|Default)\s+)*(Sub|Function|Property|Class|Module|Structure|Interface|Enum|Namespace|Select|With|Try|Using|SyncLock|While|For|Do|Get|Set)\b(?!.*\bEnd\b).*|.*\bThen|Else|ElseIf\b.*|Case\b.*|Catch\b.*|Finally)\s*('.*)?$/i,
      outdent: /^\s*(End\b.*|Else|ElseIf\b.*|Next\b.*|Loop\b.*|Wend|Case\b.*|Catch\b.*|Finally)$/i }),
    L("xml", "XML", "<>", "#3f8fd6", { exts: [".xml", ".svg", ".xaml", ".csproj", ".vbproj", ".fsproj", ".vcxproj", ".proj", ".props", ".targets", ".nuspec", ".plist", ".xsd", ".xsl", ".xslt", ".rss", ".atom", ".ui", ".glade", ".kml", ".gpx", ".resx", ".manifest", ".wsdl", ".xlf", ".xliff", ".dtd", ".config", ".tmLanguage", ".tmTheme", ".mxml", ".fxml", ".storyboard", ".xib", ".sublime-snippet", ".metainfo.xml"],
      names: ["packages.config", "web.config", "app.config", "nuget.config", ".classpath", ".project"], aliases: ["svg", "xsl", "xaml", "rss", "plist"], blockComment: ["<!--", "-->"],
      brackets: [["<", ">"], ...BR], fold: "indent", indentAfter: TAG_INDENT, outdent: TAG_OUTDENT, wordChars: DASH_WORD }),
    L("yaml", "YAML", "YML", "#cb171e", { exts: [".yml", ".yaml", ".clang-format", ".clang-tidy", ".gemrc", ".sublime-syntax"], names: [".clang-format", ".clang-tidy", ".yamllint", "pnpm-lock.yaml", "CITATION.cff", ".gitlab-ci.yml"], aliases: ["yml"],
      lineComment: "#", fold: "indent", indentAfter: /(:|^\s*-|[|>][-+0-9]*)\s*(#.*)?$|[[{]\s*$/ }),
  ];
  const languages = [LANGS[0], ...LANGS.slice(1).sort((a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }))];
  const BY_ID = new Map(languages.map((l) => [l.id, l]));
  const BY_EXT = new Map();
  const BY_NAME = new Map();
  const ALIAS = new Map();
  for (const l of languages) {
    for (const e of l.exts) if (!BY_EXT.has(e.toLowerCase())) BY_EXT.set(e.toLowerCase(), l.id);
    for (const nm of l.names) BY_NAME.set(nm.toLowerCase(), l.id);
  }
  for (const l of languages) {
    ALIAS.set(l.id, l.id);
    ALIAS.set(l.name.toLowerCase(), l.id);
  }
  for (const l of languages) for (const a of l.aliases) if (!ALIAS.has(a)) ALIAS.set(a, l.id);
  for (const l of languages) for (const e of l.exts) { const k = e.slice(1).toLowerCase(); if (!ALIAS.has(k)) ALIAS.set(k, l.id); }
  /** The language id for a Markdown fence's info string ("js", "c++", "Python", …), or "". */
  function aliasOf(name) {
    return ALIAS.get(String(name).toLowerCase()) || "";
  }

  // ------------------------------------------------------------------ detection
  const INTERPRETERS = {
    python: "python", pypy: "python", bash: "shell", sh: "shell", zsh: "shell", ksh: "shell", dash: "shell", ash: "shell", mksh: "shell",
    node: "javascript", nodejs: "javascript", deno: "typescript", bun: "javascript", "ts-node": "typescript", tsx: "typescript", perl: "perl",
    ruby: "ruby", php: "php", lua: "lua", luajit: "lua", pwsh: "powershell", powershell: "powershell", rscript: "r", make: "makefile",
    groovy: "groovy", kotlin: "kotlin", scala: "scala", swift: "swift", runghc: "haskell", runhaskell: "haskell", dart: "dart",
  };
  function detect(fileName, firstLine) {
    const base = String(fileName || "").split(/[\\/]/).pop();
    const lower = base.toLowerCase();
    if (lower && !/^untitled-\d+$/.test(lower)) {
      const named = BY_NAME.get(lower);
      if (named) return named;
      if (/^(docker|container)file[.-]/.test(lower)) return "dockerfile";
      if (/^(ts|js)config\..*\.json$/.test(lower)) return "jsonc";
      if (/^makefile\./.test(lower)) return "makefile";
      if (/^\.env(\.|$)/.test(lower)) return "shell";
      if (/^\.?(bash|zsh)rc\./.test(lower)) return "shell";
      for (let k = lower.indexOf(".", 1); k >= 0; k = lower.indexOf(".", k + 1)) {
        const id = BY_EXT.get(lower.slice(k));
        if (id) return id;
      }
      if (lower.startsWith(".")) {
        const id = BY_EXT.get(lower);
        if (id) return id;
      }
    }
    const fl = String(firstLine || "");
    const sb = /^#!\s*(?:\S*\/)?(?:env\s+(?:-\S+\s+)*)?([\w.+-]+)/.exec(fl);
    if (sb) {
      const name = sb[1].toLowerCase().replace(/[\d.]+$/, "");
      if (INTERPRETERS[name]) return INTERPRETERS[name];
    }
    if (/^﻿?\s*<\?xml\b/.test(fl)) return "xml";
    if (/^﻿?\s*<!doctype\s+html\b|^﻿?\s*<html\b/i.test(fl)) return "html";
    if (/^﻿?\s*<\?php\b/.test(fl)) return "php";
    if (/^diff --git |^--- \S/.test(fl)) return "diff";
    if (/^\s*@echo\s+off\b/i.test(fl)) return "batch";
    return "plaintext";
  }

  function byId(id) {
    return BY_ID.get(id) || BY_ID.get("plaintext");
  }

  function tokenize(id, text, state) {
    const fn = T[id];
    if (!fn) return { tokens: [], state: null };
    try {
      text = String(text);
      const r = fn(text, state === undefined ? null : state);
      let tokens = r.tokens;
      // Whitespace-only runs (blank lines inside comments, strings or code blocks) carry no colour.
      for (let k = 0; k < tokens.length; k++) {
        const c = text.charCodeAt(tokens[k][0]);
        if ((c === 32 || c === 9) && !/\S/.test(text.slice(tokens[k][0], tokens[k][1]))) {
          tokens = tokens.filter((t) => /\S/.test(text.slice(t[0], t[1])));
          break;
        }
      }
      return { tokens, state: r.state === undefined ? null : r.state };
    } catch (e) {
      return { tokens: [], state: null };
    }
  }

  return { languages, byId, detect, tokenize, types: TYPES, aliasOf };
})();

