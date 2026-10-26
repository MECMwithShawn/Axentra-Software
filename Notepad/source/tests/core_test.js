// Editor core tests: gjs tests/core_test.js
const GLib = imports.gi.GLib;
const System = imports.system;
const self = GLib.canonicalize_filename(System.programPath || System.programInvocationName, GLib.get_current_dir());
const root = GLib.path_get_dirname(GLib.path_get_dirname(self));
function load(rel) {
  const [, bytes] = GLib.file_get_contents(GLib.build_filenamev([root, rel]));
  (0, eval)(new TextDecoder().decode(bytes));
}
load("ui/core.js");
let haveLang = GLib.file_test(GLib.build_filenamev([root, "ui/lang.js"]), GLib.FileTest.EXISTS);
if (haveLang) load("ui/lang.js");

const C = globalThis.AxnCore;
const { P, S } = C;
let failed = 0, passed = 0;
function eq(a, b, msg) {
  const ja = JSON.stringify(a), jb = JSON.stringify(b);
  if (ja === jb) passed++;
  else {
    failed++;
    print(`FAIL ${msg}\n  got:  ${ja}\n  want: ${jb}`);
  }
}
function ok(v, msg) { eq(!!v, true, msg); }
const sels = (b) => b.sels.map((s) => [s.anchor.line, s.anchor.ch, s.head.line, s.head.ch]);
function buf(text, cursors) {
  const b = new C.Buffer(text, haveLang ? { tokenize: AxnLang.tokenize } : {});
  if (cursors) b.setSels(cursors.map((c) => (c.length === 2 ? S(P(c[0], c[1])) : S(P(c[0], c[1]), P(c[2], c[3])))));
  return b;
}
const JS = haveLang ? AxnLang.byId("javascript") : { autoClose: [["(", ")"], ["{", "}"], ['"', '"']], brackets: [["(", ")"], ["[", "]"], ["{", "}"]], indentAfter: /[{[(]\s*$/, lineComment: "//", blockComment: ["/*", "*/"] };
const O = { lang: JS, tabSize: 4, insertSpaces: true, autoClose: true, autoIndent: true };

// ---------------------------------------------------------------- document
{
  const d = new C.Doc("ab\ncd\nef");
  const c = d.replace(P(0, 1), P(2, 1), "X\nY");
  eq(d.lines, ["aX", "Yf"], "replace across lines");
  eq(c.removed, "b\ncd\ne", "removed text");
  eq(c.newTo, P(1, 1), "newTo");
  eq(d.get(P(0, 1), P(1, 1)), "X\nY", "get range");
  const big = new C.Doc("");
  big.replace(P(0, 0), P(0, 0), Array.from({ length: 200000 }, (_, i) => "l" + i).join("\n"));
  eq(big.size, 200000, "large insert doesn't overflow the stack");
  big.replace(P(1, 0), P(199999, 0), "");
  eq(big.size, 2, "large delete");
}
// ---------------------------------------------------------------- mapping
{
  const c = { from: P(1, 2), to: P(1, 4), newTo: P(2, 1) };
  eq(C.mapPos(P(0, 9), c), P(0, 9), "before stays");
  eq(C.mapPos(P(1, 6), c), P(2, 3), "same line after shifts");
  eq(C.mapPos(P(3, 0), c), P(4, 0), "later line shifts");
  eq(C.mapPos(P(1, 3), c, -1), P(1, 2), "inside, assoc -1");
  eq(C.mapPos(P(1, 3), c, 1), P(2, 1), "inside, assoc 1");
  const ins = { from: P(0, 1), to: P(0, 1), newTo: P(0, 4) };
  eq(C.mapPos(P(0, 1), ins, 1), P(0, 4), "insert at pos, assoc 1");
  eq(C.mapPos(P(0, 1), ins, -1), P(0, 1), "insert at pos, assoc -1");
}
// ---------------------------------------------------------------- typing, undo, multi-cursor
{
  const b = buf("one\ntwo\nthree", [[0, 3], [1, 3], [2, 5]]);
  C.insertText(b, "!");
  eq(b.doc.lines, ["one!", "two!", "three!"], "multi-cursor insert");
  eq(sels(b), [[0, 4, 0, 4], [1, 4, 1, 4], [2, 6, 2, 6]], "cursors after insert");
  C.insertText(b, "?");
  eq(b.undoStack.length, 1, "typing coalesces");
  ok(b.dirty, "dirty after typing");
  b.undo();
  eq(b.doc.lines, ["one", "two", "three"], "undo restores");
  ok(!b.dirty, "clean after undo to start");
  b.redo();
  eq(b.doc.lines, ["one!?", "two!?", "three!?"], "redo");
  b.markClean();
  ok(!b.dirty, "markClean");
  C.insertText(b, "x");
  ok(b.dirty, "edit after save is dirty (no merge into saved step)");
  eq(b.undoStack.length, 2, "new undo step after save");
}
{
  const b = buf("hello", [[0, 5]]);
  for (const ch of "ab cd") C.typeChar(b, ch, O);
  eq(b.doc.lines, ["helloab cd"], "typed");
  eq(b.undoStack.length, 2, "space after a word starts a new undo step");
}
{
  const b = buf("ab", [[0, 1], [0, 2]]);
  C.backspace(b, O);
  eq(b.doc.lines, [""], "two cursors backspace");
  const c = buf("abc", [[0, 1], [0, 1]]);
  eq(c.sels.length, 1, "duplicate cursors merge");
}
// ---------------------------------------------------------------- auto-close, surround, overtype, newline
{
  const b = buf("", [[0, 0]]);
  C.typeChar(b, "(", O);
  eq([b.doc.lines, sels(b)], [["()"], [[0, 1, 0, 1]]], "auto-close paren");
  C.typeChar(b, ")", O);
  eq([b.doc.lines, sels(b)], [["()"], [[0, 2, 0, 2]]], "overtype closer");
  const q = buf("it", [[0, 2]]);
  C.typeChar(q, '"', O);
  eq(q.doc.lines, ['it"'], "no quote pair after a word");
  const s = buf("word", [[0, 0, 0, 4]]);
  C.typeChar(s, "(", O);
  eq([s.doc.lines, sels(s)], [["(word)"], [[0, 1, 0, 5]]], "surround selection");
  const bs = buf("()", [[0, 1]]);
  C.backspace(bs, O);
  eq(bs.doc.lines, [""], "backspace deletes empty pair");
}
{
  const b = buf("  if (x) {}", [[0, 10]]);
  C.newline(b, O);
  eq(b.doc.lines, ["  if (x) {", "      ", "  }"], "enter between braces");
  eq(sels(b), [[1, 6, 1, 6]], "cursor on the indented middle line");
  const c = buf("    foo", [[0, 7]]);
  C.newline(c, O);
  eq(c.doc.lines, ["    foo", "    "], "keeps indentation");
  C.newline(c, O);
  eq(c.doc.lines, ["    foo", "", "    "], "trims whitespace-only line");
  const d = buf("  {\n    x\n    ", [[2, 4]]);
  C.typeChar(d, "}", O);
  eq(d.doc.lines, ["  {", "    x", "  }"], "closer outdents to its opener");
}
{
  const b = buf("    x", [[0, 4]]);
  C.backspace(b, O);
  eq(b.doc.lines, ["x"], "backspace removes a tab stop of spaces");
  const t = buf("ab", [[0, 1]]);
  C.insertTab(t, O);
  eq(t.doc.lines, ["a   b"], "tab to next tab stop");
}
// ---------------------------------------------------------------- line operations
{
  const b = buf("a\nb\nc", [[1, 0]]);
  C.moveLines(b, -1);
  eq(b.doc.lines, ["b", "a", "c"], "move line up");
  eq(sels(b), [[0, 0, 0, 0]], "cursor follows");
  C.moveLines(b, 1);
  C.moveLines(b, 1);
  eq(b.doc.lines, ["a", "c", "b"], "move line down twice");
  C.copyLines(b, 1);
  eq([b.doc.lines, sels(b)], [["a", "c", "b", "b"], [[3, 0, 3, 0]]], "copy line down");
  C.deleteLines(b);
  eq(b.doc.lines, ["a", "c", "b"], "delete last line");
  b.setSels([S(P(0, 0), P(1, 1))]);
  C.indentLines(b, O, 1);
  eq(b.doc.lines, ["    a", "    c", "b"], "indent selected lines");
  C.indentLines(b, O, -1);
  eq(b.doc.lines, ["a", "c", "b"], "outdent");
  C.toggleLineComment(b, JS);
  eq(b.doc.lines, ["// a", "// c", "b"], "comment lines");
  C.toggleLineComment(b, JS);
  eq(b.doc.lines, ["a", "c", "b"], "uncomment lines");
}
{
  const b = buf("pear\napple\n\nfig\napple", [[0, 0]]);
  C.transformLines(b, (l) => [...l].sort());
  eq(b.doc.lines, ["", "apple", "apple", "fig", "pear"], "sort whole document");
  C.transformLines(b, (l) => l.filter((x, i) => l.indexOf(x) === i));
  eq(b.doc.lines, ["", "apple", "fig", "pear"], "remove duplicates");
  b.setSels([S(P(1, 0), P(3, 4))]);
  C.joinLines(b);
  eq(b.doc.lines, ["", "apple fig pear"], "join lines");
  const u = buf("hello world", [[0, 2]]);
  C.transformText(u, (t) => t.toUpperCase());
  eq(u.doc.lines, ["HELLO world"], "upper-case word at cursor");
  const bc = buf("x = 1", [[0, 0, 0, 5]]);
  C.toggleBlockComment(bc, JS);
  eq(bc.doc.lines, ["/* x = 1 */"], "block comment");
  C.toggleBlockComment(bc, JS);
  eq(bc.doc.lines, ["x = 1"], "block uncomment");
}
// ---------------------------------------------------------------- occurrences and search
{
  const b = buf("foo bar foo\nfoofoo foo", [[0, 1]]);
  C.addNextOccurrence(b, { wholeWord: true });
  eq(sels(b), [[0, 0, 0, 3]], "Ctrl+D selects word");
  C.addNextOccurrence(b, { wholeWord: true });
  C.addNextOccurrence(b, { wholeWord: true });
  eq(sels(b), [[0, 0, 0, 3], [0, 8, 0, 11], [1, 7, 1, 10]], "Ctrl+D whole-word occurrences");
  C.insertText(b, "baz");
  eq(b.doc.lines, ["baz bar baz", "foofoo baz"], "edit all occurrences");
  const a = buf("a1 a2 a1", [[0, 0]]);
  eq(C.selectAllOccurrences(a, {}), 2, "select all occurrences");
}
{
  const d = new C.Doc("Alpha beta\nalpha BETA\ngamma");
  const cq = C.compileQuery("alpha", {});
  eq(C.findAll(d, cq).map((m) => [m.from.line, m.from.ch]), [[0, 0], [1, 0]], "case-insensitive");
  eq(C.findAll(d, C.compileQuery("alpha", { caseSensitive: true })).length, 1, "case-sensitive");
  eq(C.findAll(d, C.compileQuery("a.p", {})).length, 0, "literal dot is escaped");
  eq(C.findAll(d, C.compileQuery("(\\w+) (\\w+)", { regex: true })).length, 2, "regex");
  ok(C.compileQuery("(", { regex: true }).error, "invalid regex reports an error");
  eq(C.findAll(d, C.compileQuery("beta\nalpha", {})).map((m) => [m.from, m.to]), [[P(0, 6), P(1, 5)]], "multi-line literal");
  eq(C.findAll(d, C.compileQuery("eta", { wholeWord: true })).length, 0, "whole word");
  eq(C.findAll(new C.Doc("é é x"), C.compileQuery("é", { wholeWord: true })).length, 2, "whole word with accents");
  const m = C.findAll(new C.Doc("john smith"), C.compileQuery("(\\w+) (\\w+)", { regex: true }))[0];
  eq(C.expandReplace("$2, \\u$1 $$ \\U$0\\E!", m.m), "smith, John $ JOHN SMITH!", "replacement expansion");
  const b = buf("a-b-c", [[0, 0]]);
  eq(C.replaceAll(b, C.findAll(b.doc, C.compileQuery("-", {})), "+"), 2, "replace all count");
  eq(b.doc.lines, ["a+b+c"], "replace all");
  const z = new C.Doc("ab\n\ncd");
  eq(C.findAll(z, C.compileQuery("^", { regex: true })).length, 3, "zero-length matches advance");
}
// ---------------------------------------------------------------- widths, wrapping, layout
{
  eq(C.vwidth("a\tb", 0, 3, 4), 5, "tab width");
  eq(C.vwidth("日本", 0, 2, 4), 4, "wide chars");
  eq(C.vwidth("é", 0, 2, 4), 1, "combining mark");
  eq(C.vwidth("😀x", 0, 3, 4), 3, "emoji surrogate pair");
  eq(C.offsetAt("a\tb", 0, 3, 3.9, 4), 2, "click inside a tab rounds to the nearer side");
  eq(C.wrapBreaks("aaaa bbbb cccc", 10, 4), [0, 10], "wrap at a space");
  eq(C.wrapBreaks("abcdefghijklmnop", 8, 4), [0, 8], "hard wrap a long word");
  eq(C.wrapBreaks("日本語日本語", 8, 4), [0, 4], "wrap wide text");
}
{
  const b = buf("short\n" + "word ".repeat(10).trim() + "\nend", [[0, 0]]);
  const L = new C.Layout(b);
  L.configure({ wrap: true, cols: 20, tab: 4 });
  eq(L.totalRows(), 5, "wrapped rows");
  eq(L.rowLine(3), { line: 1, sub: 2 }, "row to line");
  eq(L.posRow(P(1, 25)).row, 2, "pos to row");
  b.setSels([S(P(0, 3))]);
  let m = C.moveVertical(L, b.main, 1);
  eq(m.pos, P(1, 3), "down into a wrapped line");
  m = C.moveVertical(L, { head: m.pos, goal: m.goal }, 1);
  eq(m.pos, P(1, 23), "down to the second row keeps the column");
  const e = C.endPos(L, S(P(1, 1)));
  eq([e.pos, e.bias], [P(1, 20), -1], "End stops at the end of the wrapped row");
  eq(L.posRow(e.pos, e.bias).row, 1, "bias keeps the cursor on the upper row");
  C.insertText(b, "\n");
  eq(L.totalRows(), 6, "rows follow edits");
  L.configure({ wrap: false });
  eq(L.totalRows(), 4, "unwrapped rows");
}
{
  const b = buf("a {\n  b\n  c\n}\nd", [[0, 0]]);
  const L = new C.Layout(b);
  const r = C.foldRange(b, 0, "brackets", 4);
  eq(r, { start: 0, end: 2 }, "bracket fold keeps the closing line");
  L.fold(r.start, r.end);
  eq(L.totalRows(), 3, "folded rows");
  eq(L.rowLine(1), { line: 3, sub: 0 }, "row after the fold");
  const m = C.moveVertical(L, S(P(0, 1)), 1);
  eq(m.pos.line, 3, "moving down skips folded lines");
  b.setSels([S(P(4, 1))]);
  C.insertText(b, "\nx");
  eq([L.folds, L.totalRows()], [[{ start: 0, end: 2 }], 4], "edit below keeps the fold");
  b.setSels([S(P(0, 0))]);
  C.insertText(b, "\n");
  eq(L.folds, [{ start: 1, end: 3 }], "edit above shifts the fold");
  b.setSels([S(P(2, 1))]);
  C.insertText(b, "!");
  eq(L.folds, [], "edit inside unfolds");
  const py = buf("def f():\n    x = 1\n\n    y = 2\nz = 3", [[0, 0]]);
  eq(C.foldRange(py, 0, "indent", 4), { start: 0, end: 3 }, "indent fold");
  const md = buf("# A\ntext\n## B\nmore\n# C", [[0, 0]]);
  eq(C.foldRange(md, 0, "markdown", 4), { start: 0, end: 3 }, "markdown heading fold");
  const rg = buf("// #region x\na\n// #endregion\nb", [[0, 0]]);
  eq(C.foldRange(rg, 0, "brackets", 4), { start: 0, end: 2 }, "region fold");
}
// ---------------------------------------------------------------- brackets, words, indentation, box selection
{
  const b = buf("f(a, [1, 2])\n{\n}", [[0, 1]]);
  eq(C.matchBracket(b, P(0, 1)), { a: P(0, 1), b: P(0, 11) }, "bracket match forward");
  eq(C.matchBracket(b, P(0, 11)), { a: P(0, 10), b: P(0, 5) }, "bracket match before cursor");
  eq(C.matchBracket(b, P(1, 0)).b, P(2, 0), "multi-line match");
  eq(C.moveWord(b.doc, P(0, 0), 1), P(0, 1), "word right");
  eq(C.moveWord(new C.Doc("foo.bar  baz"), P(0, 12), -1), P(0, 9), "word left");
  eq(C.detectIndent(new C.Doc("a\n  b\n    c\n  d\n")), { insertSpaces: true, tabSize: 2 }, "detect 2 spaces");
  eq(C.detectIndent(new C.Doc("a\n\tb\n\t\tc\n")), { insertSpaces: false }, "detect tabs");
  eq(C.boxSels(new C.Doc("abcdef\nab\nabcdef"), 4, 0, 1, 2, 4).map((s) => [s.anchor.ch, s.head.ch]), [[1, 4], [1, 2], [1, 4]], "box selection");
}
// ---------------------------------------------------------------- highlighting cache
if (haveLang) {
  let calls = 0;
  const tok = (id, t, s) => { calls++; return AxnLang.tokenize(id, t, s); };
  const b = new C.Buffer(Array.from({ length: 2000 }, (_, i) => `let x${i} = ${i}; // line`).join("\n"), { lang: "javascript", tokenize: tok });
  b.hl.ensure(2000);
  calls = 0;
  b.setSels([S(P(10, 0))]);
  C.insertText(b, "q");
  b.hl.ensure(2000);
  ok(calls < 5, `editing one line re-tokenizes only that line (${calls})`);
  b.setSels([S(P(10, 0))]);
  C.insertText(b, "/*");
  b.hl.ensure(2000);
  ok(calls > 1900, "opening a block comment re-tokenizes below");
  eq(b.hl.typeAt(1500, 2), "comment", "lines below are inside the comment");
  b.undo();
  b.hl.ensure(2000);
  eq(b.hl.typeAt(1500, 0), "keyword", "undo restores highlighting");
}

print(`core: ${passed} passed, ${failed} failed`);
if (failed) System.exit(1);
