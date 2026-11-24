// Syntax highlighting tests: gjs tests/lang_test.js
const GLib = imports.gi.GLib;
const System = imports.system;
const self = GLib.canonicalize_filename(System.programPath || System.programInvocationName, GLib.get_current_dir());
const root = GLib.path_get_dirname(GLib.path_get_dirname(self));
{
  const [, bytes] = GLib.file_get_contents(GLib.build_filenamev([root, "ui/lang.js"]));
  (0, eval)(new TextDecoder().decode(bytes));
}
const L = globalThis.AxnLang;

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

/** Tokens of one line as [text, type] pairs, which read better than offsets. */
function spans(id, line, state = null) {
  return L.tokenize(id, line, state).tokens.map((t) => [line.slice(t[0], t[1]), t[2]]);
}
/** Runs lines through a language and returns [spans, state] per line. */
function run(id, lines) {
  let st = null;
  return lines.map((line) => {
    const r = L.tokenize(id, line, st);
    st = r.state;
    return [r.tokens.map((t) => [line.slice(t[0], t[1]), t[2]]), st];
  });
}
const has = (id, line, span, state = null) => ok(spans(id, line, state).some((s) => s[0] === span[0] && s[1] === span[1]), `${id}: ${JSON.stringify(line)} has ${JSON.stringify(span)}`);

// ---------------------------------------------------------------- metadata
eq(L.languages[0].id, "plaintext", "plaintext first");
eq(L.languages[0].name, "Plain Text", "plaintext name");
{
  const rest = L.languages.slice(1).map((l) => l.name);
  eq(rest, [...rest].sort((a, b) => a.localeCompare(b, "en", { sensitivity: "base" })), "languages sorted by name");
  const required = "plaintext c cpp csharp java kotlin scala swift go rust javascript typescript json jsonc python ruby php perl lua r dart shell powershell batch sql html xml css scss markdown yaml toml ini dockerfile makefile cmake diff log csv tsv vb haskell groovy ignore".split(" ");
  for (const id of required) eq(L.byId(id).id, id, `language ${id} exists`);
  for (const l of L.languages) {
    ok(l.icon && typeof l.icon.text === "string" && l.icon.text.length <= 3 && /^#[0-9a-f]{6}$/i.test(l.icon.color), `${l.id} icon`);
    ok(Array.isArray(l.brackets) && Array.isArray(l.autoClose) && ["brackets", "indent", "markdown"].includes(l.fold), `${l.id} shape`);
    ok(l.autoClose.every((p) => p[0].length === 1 && p[1].length === 1), `${l.id} autoClose pairs are single characters`);
  }
  eq(L.byId("nope").id, "plaintext", "unknown id falls back to plaintext");
  eq(L.byId("plaintext").autoClose.some((p) => p[0] === "'"), false, "plain text doesn't auto-close apostrophes");
  eq(L.byId("markdown").autoClose.some((p) => p[0] === "'"), false, "markdown doesn't auto-close apostrophes");
  ok(L.byId("markdown").autoClose.some((p) => p[0] === "`"), "markdown auto-closes backticks");
  eq(L.byId("makefile").useTabs, true, "makefiles use tabs");
  ok(L.byId("javascript").indentAfter.test("if (x) {"), "indent after {");
  ok(L.byId("javascript").indentAfter.test("foo(  // why"), "indent after ( with a comment");
  ok(L.byId("python").indentAfter.test("def f(x):  # c"), "python indents after :");
  ok(L.byId("yaml").indentAfter.test("key:"), "yaml indents after key:");
  ok(L.byId("html").indentAfter.test("  <div class=\"a\">"), "html indents after an open tag");
  ok(!L.byId("html").indentAfter.test("<br>"), "no indent after a void tag");
  ok(!L.byId("html").indentAfter.test("<p>text</p>"), "no indent after a closed tag");
  ok(L.byId("ruby").indentAfter.test("items.each do |x|"), "ruby indents after do |x|");
  ok(!L.byId("ruby").indentAfter.test("if x then y end"), "ruby one-line if");
  ok(L.byId("lua").indentAfter.test("local function f(a)"), "lua function");
  ok(L.byId("shell").indentAfter.test("for f in *; do"), "shell do");
  ok(L.byId("c").outdent.test("    }"), "c outdent");
  for (const t of L.types) ok(/^[a-z0-9]+$/.test(t), `type name ${t}`);
}

// ---------------------------------------------------------------- detection
{
  const cases = [
    ["main.c", "", "c"], ["x.h", "", "c"], ["a.cpp", "", "cpp"], ["a.HPP", "", "cpp"], ["Program.cs", "", "csharp"], ["App.java", "", "java"],
    ["build.gradle", "", "groovy"], ["Jenkinsfile", "", "groovy"], ["main.kt", "", "kotlin"], ["a.scala", "", "scala"], ["a.swift", "", "swift"],
    ["main.go", "", "go"], ["lib.rs", "", "rust"], ["app.js", "", "javascript"], ["app.mjs", "", "javascript"], ["View.jsx", "", "javascript"],
    ["a.ts", "", "typescript"], ["types.d.ts", "", "typescript"], ["App.tsx", "", "typescript"], ["package.json", "", "json"],
    ["tsconfig.json", "", "jsonc"], ["tsconfig.build.json", "", "jsonc"], ["x.jsonc", "", "jsonc"], ["settings.json", "", "jsonc"],
    ["a.py", "", "python"], ["Rakefile", "", "ruby"], ["Gemfile", "", "ruby"], ["a.rb", "", "ruby"], ["index.php", "", "php"], ["a.pl", "", "perl"],
    ["init.lua", "", "lua"], ["a.R", "", "r"], ["main.dart", "", "dart"], ["run.sh", "", "shell"], [".bashrc", "", "shell"], [".zshrc", "", "shell"],
    [".profile", "", "shell"], ["PKGBUILD", "", "shell"], [".env", "", "shell"], [".env.local", "", "shell"], ["a.ps1", "", "powershell"],
    ["build.bat", "", "batch"], ["a.CMD", "", "batch"], ["q.sql", "", "sql"], ["index.html", "", "html"], ["a.htm", "", "html"], ["a.svg", "", "xml"],
    ["App.csproj", "", "xml"], ["x.plist", "", "xml"], ["a.css", "", "css"], ["a.scss", "", "scss"], ["a.less", "", "scss"], ["README.md", "", "markdown"],
    ["ci.yml", "", "yaml"], ["a.yaml", "", "yaml"], ["Cargo.toml", "", "toml"], ["Cargo.lock", "", "toml"], ["php.ini", "", "ini"], ["x.desktop", "", "ini"],
    [".editorconfig", "", "ini"], ["Dockerfile", "", "dockerfile"], ["Dockerfile.dev", "", "dockerfile"], ["dockerfile", "", "dockerfile"],
    ["Makefile", "", "makefile"], ["GNUmakefile", "", "makefile"], ["rules.mk", "", "makefile"], ["CMakeLists.txt", "", "cmake"], ["x.cmake", "", "cmake"],
    ["fix.patch", "", "diff"], ["a.diff", "", "diff"], ["syslog.log", "", "log"], ["data.csv", "", "csv"], ["data.tsv", "", "tsv"], ["Module1.vb", "", "vb"],
    ["a.vbs", "", "vb"], ["Main.hs", "", "haskell"], [".gitignore", "", "ignore"], [".dockerignore", "", "ignore"], ["notes.txt", "", "plaintext"],
    ["a.tex", "", "latex"], ["boot.asm", "", "asm"], ["x.m", "", "objective-c"],
    ["script", "#!/usr/bin/env python3", "python"], ["script", "#!/bin/bash", "shell"], ["s", "#!/bin/sh -e", "shell"], ["s", "#!/usr/bin/env node", "javascript"],
    ["s", "#!/usr/bin/perl -w", "perl"], ["s", "#!/usr/bin/env ruby", "ruby"], ["s", "#!/usr/bin/env -S deno run", "typescript"], ["s", "#!/usr/bin/php", "php"],
    ["Untitled-1", "<?xml version=\"1.0\"?>", "xml"], ["", "<!DOCTYPE html>", "html"], ["", "<html lang=en>", "html"], ["", "<?php", "php"],
    ["", "diff --git a/x b/x", "diff"], ["", "@echo off", "batch"], ["", "{", "plaintext"], ["", "[1, 2]", "plaintext"], ["Untitled-3", "", "plaintext"],
    ["C:\\Users\\me\\a.py", "", "python"], ["/home/me/src/Makefile", "", "makefile"], ["LICENSE", "", "plaintext"],
  ];
  for (const [f, first, want] of cases) eq(L.detect(f, first), want, `detect(${JSON.stringify(f)}, ${JSON.stringify(first)})`);
}

// ---------------------------------------------------------------- single lines
eq(L.tokenize("javascript", 'const x = "a\\n" // c', null).tokens, [[0, 5, "keyword"], [8, 9, "operator"], [10, 12, "string"], [12, 14, "escape"], [14, 15, "string"], [16, 20, "comment"]], "js line exact");
eq(L.tokenize("javascript", 'const x = "a\\n" // c', null).state, null, "js line state");
eq(spans("javascript", "if (a) return /x+/g.test(s) / 2;"), [["if", "control"], ["return", "control"], ["/x+/g", "regexp"], ["test", "function"], ["/", "operator"], ["2", "number"]], "js regex vs division");
eq(spans("javascript", "a = b / c / d;"), [["=", "operator"], ["/", "operator"], ["/", "operator"]], "js division after identifiers");
eq(spans("javascript", "class Foo extends Bar { static MAX = 0x1F; }"), [["class", "keyword"], ["Foo", "type"], ["extends", "keyword"], ["Bar", "type"], ["static", "keyword"], ["MAX", "constant"], ["=", "operator"], ["0x1F", "number"]], "js class line");
has("javascript", "class Foo extends Bar {}", ["Foo", "type"]);
has("javascript", "const y = null ?? undefined;", ["undefined", "constant"]);
has("javascript", "@decorator() method() {}", ["@decorator", "meta"]);
has("javascript", "let n = 1_000n + .5e-3;", ["1_000n", "number"]);
has("javascript", "let n = 1_000n + .5e-3;", [".5e-3", "number"]);
has("typescript", "function f(x: number): Promise<void> {}", ["number", "type"]);
has("typescript", "function f(x: number): Promise<void> {}", ["f", "function"]);
has("typescript", "export interface A {}", ["interface", "keyword"]);
eq(spans("python", 'def f(x): return b"\\x00" if x else None  # done'), [["def", "keyword"], ["f", "function"], [":", "operator"], ["return", "control"], ['b"', "string"], ["\\x00", "escape"], ['"', "string"], ["if", "control"], ["else", "control"], ["None", "constant"], ["# done", "comment"]], "python line");
has("python", '@app.route("/")', ["@app.route", "meta"]);
has("python", "x = r'\\d+'", ["r'\\d+'", "string"]);
has("python", "class Model(Base):", ["Model", "type"]);
has("rust", "fn main() -> Result<(), Box<dyn Error>> {", ["main", "function"]);
has("rust", "println!(\"{}\", x);", ["println!", "meta"]);
has("rust", "impl<'a> Foo<'a> for &'a str {}", ["'a", "keyword"]);
has("rust", "let c = '\\n';", ["'", "string"]);
has("rust", "let v: u32 = 1_000u32;", ["1_000u32", "number"]);
has("rust", "#![allow(dead_code)]", ["#![allow(dead_code)]", "meta"]);
has("c", "#include <stdio.h>", ["<stdio.h>", "string"]);
has("c", "#include \"x.h\"", ["\"x.h\"", "string"]);
has("c", "#pragma once", ["#pragma once", "meta"]);
has("c", "unsigned long n = sizeof(int) + EOF;", ["EOF", "constant"]);
has("cpp", "std::unique_ptr<Foo> p = nullptr;", ["nullptr", "constant"]);
has("cpp", "auto s = u8\"x\";", ['u8"x"', "string"]);
has("csharp", "public async Task<int> Run() { await Go(); }", ["await", "control"]);
has("csharp", "#region Helpers", ["#region Helpers", "meta"]);
has("java", "@Override public String toString() {", ["@Override", "meta"]);
has("go", "func (s *Server) Start() error {", ["error", "type"]);
has("go", "x := make([]int, 10)", ["make", "function"]);
has("kotlin", "data class User(val name: String)", ["User", "type"]);
has("swift", "guard let x = y else { return }", ["guard", "control"]);
has("php", "<?php $x = new Foo(); // c", ["$x", "variable"]);
has("php", "<?php $x = new Foo(); // c", ["new", "keyword"]);
has("ruby", "puts :sym, @ivar, $glob", [":sym", "constant"]);
has("ruby", "puts :sym, @ivar, $glob", ["@ivar", "variable"]);
has("perl", 'my @list = ($a, %h);', ["@list", "variable"]);
has("lua", "local t = { x = 1 } -- c", ["-- c", "comment"]);
has("sql", "select * from t where a is not null;", ["select", "keyword"]);
has("sql", "SELECT COUNT(*) FROM [My Table] WHERE id = @id", ["@id", "variable"]);
has("sql", "SELECT COUNT(*) FROM [My Table] WHERE id = @id", ["[My Table]", "variable"]);
has("sql", "INSERT INTO t VALUES ('it''s')", ["'it''s'", "string"]);
has("haskell", "main :: IO ()", ["IO", "type"]);
has("vb", "Public Sub Main() ' go", ["' go", "comment"]);
has("vb", "public sub main()", ["sub", "keyword"]);
has("shell", 'echo "$HOME/${NAME}" $(date) # now', ["$HOME", "variable"]);
has("shell", 'echo "$HOME/${NAME}" $(date) # now', ["${NAME}", "variable"]);
has("shell", 'echo "$HOME/${NAME}" $(date) # now', ["# now", "comment"]);
has("shell", "x=1; if [ $x -eq 1 ]; then echo a#b; fi", ["fi", "control"]);
ok(!spans("shell", "echo a#b").some((s) => s[1] === "comment"), "shell: # inside a word isn't a comment");
has("shell", "export PATH=$PATH:/bin", ["export", "function"]);
has("powershell", "Get-ChildItem -Path $env:HOME | Where-Object { $_.Length -gt 1kb }", ["Where-Object", "function"]);
has("powershell", "Get-ChildItem -Path $env:HOME | Where-Object { $_.Length -gt 1kb }", ["-gt", "operator"]);
has("powershell", "Get-ChildItem -Path $env:HOME | Where-Object { $_.Length -gt 1kb }", ["1kb", "number"]);
has("batch", "for %%i in (*.txt) do echo %%i", ["%%i", "variable"]);
has("batch", "IF NOT EXIST out\\ mkdir out", ["NOT", "control"]);
has("batch", ":: comment", [":: comment", "comment"]);
has("html", '<a href="x.html" disabled>Link &copy;</a>', ['"x.html"', "string"]);
has("html", '<a href="x.html" disabled>Link &copy;</a>', ["disabled", "attr"]);
has("html", '<a href="x.html" disabled>Link &copy;</a>', ["&copy;", "escape"]);
has("html", '<a href="x.html" disabled>Link &copy;</a>', ["</a", "tag"]);
has("xml", '<?xml version="1.0"?><root xmlns:x="u"><x:item/></root>', ['<?xml version="1.0"?>', "meta"]);
has("xml", '<?xml version="1.0"?><root xmlns:x="u"><x:item/></root>', ["<x:item", "tag"]);
has("xml", '<?xml version="1.0"?><root xmlns:x="u"><x:item/></root>', ["xmlns:x", "attr"]);
has("css", ".btn:hover { color: rgb(0 0 0 / 50%); --gap: 4px; }", [".btn", "type"]);
has("css", ".btn:hover { color: rgb(0 0 0 / 50%); --gap: 4px; }", ["rgb", "function"]);
has("css", ".btn:hover { color: rgb(0 0 0 / 50%); --gap: 4px; }", ["--gap", "variable"]);
has("css", ".btn:hover { color: rgb(0 0 0 / 50%); --gap: 4px; }", ["50%", "number"]);
has("scss", "$base: #333; .a { &:hover { color: $base; } } // c", ["$base", "variable"]);
has("scss", "$base: #333; .a { &:hover { color: $base; } } // c", ["// c", "comment"]);
has("scss", "@include mixin-name(2px);", ["mixin-name", "function"]);
eq(spans("json", '{"k": [1.5e3, -2, true, null, "v"]}'), [['"k"', "property"], ["1.5e3", "number"], ["-2", "number"], ["true", "constant"], ["null", "constant"], ['"v"', "string"]], "json line");
has("jsonc", '"a": 1, // note', ["// note", "comment"]);
eq(spans("markdown", "## Heading `code`"), [["## Heading ", "heading"], ["`code`", "code"]], "markdown heading keeps inline code");
eq(spans("markdown", "> quoted **bold**"), [["> ", "quote"], ["quoted ", "quote"], ["**bold**", "strong"]], "markdown quote");
has("markdown", "> quoted **bold**", ["**bold**", "strong"]);
has("markdown", "1. item with [link](http://x.y) and _em_", ["1.", "list"]);
has("markdown", "1. item with [link](http://x.y) and _em_", ["[link]", "link"]);
has("markdown", "1. item with [link](http://x.y) and _em_", ["_em_", "emphasis"]);
ok(!spans("markdown", "snake_case_name here").some((s) => s[1] === "emphasis"), "markdown: intraword underscores aren't emphasis");
has("markdown", "Visit https://example.com/path.", ["https://example.com/path", "link"]);
has("markdown", "---", ["---", "meta"]);
has("markdown", "[ref]: https://x.y", ["https://x.y", "link"]);
has("yaml", "name: &anchor value # c", ["&anchor", "variable"]);
has("yaml", "name: &anchor value # c", ["value", "string"]);
has("yaml", "- port: 8080", ["8080", "number"]);
has("yaml", "enabled: yes", ["yes", "constant"]);
has("yaml", "tag: !!str 12", ["!!str", "type"]);
has("yaml", "list: [a, b: 1]", ["b", "property"]);
has("toml", "[[bin]] # c", ["[[bin]]", "section"]);
has("toml", 'site."google.com" = true', ['site."google.com"', "label"]);
has("toml", "n = +1_000", ["+1_000", "number"]);
has("ini", "[Desktop Entry]", ["[Desktop Entry]", "section"]);
has("ini", "Name[de]=Notizen", ["Name[de]", "label"]);
has("ini", "; comment", ["; comment", "comment"]);
has("ini", "debug = true", ["true", "constant"]);
eq(spans("csv", 'a,"b,""c""",  ,d'), [["a", "col0"], ['"b,""c"""', "col1"], ["d", "col3"]], "csv columns skip blank fields");
eq(spans("tsv", "a\tb\tc\td\te\tf\tg"), [["a", "col0"], ["b", "col1"], ["c", "col2"], ["d", "col3"], ["e", "col4"], ["f", "col5"], ["g", "col0"]], "tsv rainbow cycles");
eq(spans("log", "2026-10-21T10:00:00Z [WARN] disk 0xFF NullPointerException"), [["2026-10-21T10:00:00Z", "number"], ["WARN", "warning"], ["0xFF", "constant"], ["NullPointerException", "error"]], "log line");
has("log", "info: debug mode", ["debug", "debug"]);
eq(spans("diff", "@@ -1,2 +1,3 @@ fn x()"), [["@@ -1,2 +1,3 @@", "meta"]], "diff hunk header");
eq(spans("diff", "diff --git a/b c/d"), [["diff --git a/b c/d", "keyword"]], "diff header");
eq(spans("diff", " context"), [], "diff context");
has("ignore", "!keep/**/*.log", ["!", "operator"]);
has("ignore", "!keep/**/*.log", ["**", "operator"]);
has("dockerfile", "FROM rust:1 AS build", ["AS", "keyword"]);
has("dockerfile", 'ENV PATH="/x:$PATH"', ["$PATH", "variable"]);
has("dockerfile", "# syntax=docker/dockerfile:1", ["# syntax=docker/dockerfile:1", "meta"]);
has("makefile", "include config.mk", ["include", "keyword"]);
has("makefile", "out/%.o: src/%.c | dirs", ["out/%.o", "function"]);
has("makefile", "\techo $$HOME $(shell date)", ["$$", "escape"]);
has("makefile", "\techo $$HOME $(shell date)", ["$(shell date)", "variable"]);
has("cmake", 'set(CMAKE_CXX_STANDARD 17 CACHE STRING "std")', ["set", "function"]);
has("cmake", 'set(CMAKE_CXX_STANDARD 17 CACHE STRING "std")', ['"std"', "string"]);
has("cmake", "endif()", ["endif", "control"]);
has("latex", "\\section{Intro} text \\\\ more", ["\\section", "keyword"]);
has("asm", "start: mov eax, 0x10 ; c", ["start:", "function"]);
has("asm", "section .text", [".text", "meta"]);
eq(L.tokenize("plaintext", "anything at all", null), { tokens: [], state: null }, "plaintext has no tokens");
eq(L.tokenize("bogus", "x = 1", null), { tokens: [], state: null }, "unknown language");
eq(L.tokenize("javascript", "", null), { tokens: [], state: null }, "empty line");

// ---------------------------------------------------------------- multi-line states
{
  const r = run("c", ["int a; /* start", "   middle", "end */ int b;"]);
  eq(r[0][1] !== null, true, "c: comment stays open");
  eq(r[1][0], [["   middle", "comment"]], "c: middle line is comment");
  eq(r[2][0], [["end */", "comment"], ["int", "type"]], "c: comment closes");
  eq(r[2][1], null, "c: state returns to null after the comment");
}
{
  const r = run("python", ['x = """doc', "  body", 'end""" + f(1)']);
  eq(r[1][0], [["  body", "string"]], "python triple string body");
  eq(r[2][0], [['end"""', "string"], ["+", "operator"], ["f", "function"], ["1", "number"]], "python triple string ends");
  eq(r[2][1], null, "python state null after the string");
  const raw = run("python", ["s = r'''a\\", "b''' "]);
  eq(raw[1][0], [["b'''", "string"]], "python raw triple string");
}
{
  const r = run("javascript", ["const s = `outer ${ `inner ${x}` } and ${", "  y + 1 } end` + 2;"]);
  eq(r[0][0], [["const", "keyword"], ["=", "operator"], ["`outer ", "string"], ["${", "keyword"], ["`inner ", "string"], ["${", "keyword"], ["}", "keyword"], ["`", "string"], ["}", "keyword"], [" and ", "string"], ["${", "keyword"]], "js nested template line 1");
  ok(typeof r[0][1] === "string", "js template expression continues");
  eq(r[1][0], [["+", "operator"], ["1", "number"], ["}", "keyword"], [" end`", "string"], ["+", "operator"], ["2", "number"]], "js nested template line 2");
  eq(r[1][1], null, "js template closes");
  const t = run("typescript", ["const q = `line one", "line two ${a}", "three`;"]);
  eq(t[1][0], [["line two ", "string"], ["${", "keyword"], ["}", "keyword"]], "ts multi-line template");
  eq(t[2][1], null, "ts template closes");
}
{
  const r = run("rust", ["/* a /* nested */ still", "comment */ let s = r##\"raw \"# still", "\"## ; let t = \"multi", "line\";"]);
  ok(r[0][1] !== null, "rust nested comment stays open");
  eq(r[1][0][0], ["comment */", "comment"], "rust nested comment closes on the outer */");
  ok(r[1][1] !== null, "rust raw string open");
  eq(r[2][0][0], ['"##', "string"], "rust raw string closes on \"##");
  ok(r[2][1] !== null, "rust ordinary string spans lines");
  eq(r[3][0], [['line"', "string"]], "rust string closes");
  eq(r[3][1], null, "rust state null");
}
{
  const r = run("html", ["<p>x</p><script type=\"module\">", "  const a = `t`; // c", "</script><style>", "  .a { color: red; }", "</style><!-- note", "--> <b>"]);
  eq(r[1][0], [["const", "keyword"], ["=", "operator"], ["`t`", "string"], ["// c", "comment"]], "html script body is javascript");
  eq(r[3][0], [[".a", "type"], ["color", "property"], ["red", "constant"]], "html style body is css");
  eq(r[4][0].at(-1), ["<!-- note", "comment"], "html comment opens");
  eq(r[5][0], [["-->", "comment"], ["<b", "tag"], [">", "tag"]], "html comment closes");
  eq(r[5][1], null, "html state null");
  const tag = run("html", ['<img src="a.png"', '     alt="multi', 'line" />']);
  eq(tag[1][0], [["alt", "attr"], ['"multi', "string"]], "html attribute across lines");
  eq(tag[2][0], [['line"', "string"], ["/>", "tag"]], "html attribute value closes");
  eq(tag[2][1], null, "html tag closes");
}
{
  const r = run("markdown", ["Text", "```js", "const x = /re/; // c", "```", "after *em*", "~~~~python", "def f(): pass", "~~~", "still code", "~~~~", "done"]);
  eq(r[2][0], [["const", "keyword"], ["=", "operator"], ["/re/", "regexp"], ["// c", "comment"]], "markdown fence highlights js");
  eq(r[3][1], null, "markdown fence closes");
  eq(r[4][0], [["*em*", "emphasis"]], "markdown after the fence");
  eq(r[6][0], [["def", "keyword"], ["f", "function"], [":", "operator"], ["pass", "keyword"]], "markdown python fence");
  ok(r[7][1] !== null, "a shorter closing fence doesn't close");
  eq(r[9][1], null, "matching fence closes");
  const unknown = run("markdown", ["```weird", "anything", "```"]);
  eq(unknown[1][0], [["anything", "code"]], "markdown unknown fence is code");
}
{
  const r = run("yaml", ["script: |", "  echo one", "", "  echo two", "next: 1", "- run: >-", "    folded", "  - other"]);
  eq(r[1][0], [["echo one", "string"]], "yaml block scalar body");
  eq(r[2][1], r[1][1], "yaml blank line keeps the block");
  eq(r[3][0], [["echo two", "string"]], "yaml block scalar continues");
  eq(r[4][0], [["next", "property"], [":", "operator"], ["1", "number"]], "yaml block ends at a dedent");
  eq(r[6][0], [["folded", "string"]], "yaml nested block scalar");
  eq(r[7][0][0], ["-", "operator"], "yaml nested block ends");
  const q = run("yaml", ['a: "multi', 'line" # c']);
  eq(q[1][0], [['line"', "string"], ["# c", "comment"]], "yaml quoted scalar across lines");
}
{
  const r = run("csv", ['id,"note', 'more, text",3', "4,5"]);
  eq(r[0][1], "q:1", "csv quoted field opens");
  eq(r[1][0], [['more, text"', "col1"], ["3", "col2"]], "csv quoted field continues in its column");
  eq(r[1][1], null, "csv field closes");
}
{
  const r = run("toml", ['desc = """', "Line # not a comment", 'end""" # comment', "lit = '''", "x'''"]);
  eq(r[1][0], [["Line # not a comment", "string"]], "toml multi-line string body");
  eq(r[2][0], [['end"""', "string"], ["# comment", "comment"]], "toml multi-line string closes");
  eq(r[4][0], [["x'''", "string"]], "toml literal multi-line");
  eq(r[4][1], null, "toml state null");
}
{
  const r = run("lua", ["--[==[ long", "]] still ]==] x = [[a", "b]]"]);
  eq(r[1][0][0], ["]] still ]==]", "comment"], "lua long comment needs its own level");
  eq(r[2][0], [["b]]", "string"]], "lua long string");
  const p = run("powershell", ['$h = @"', "  text $x", '"@']);
  eq(p[1][0], [["  text $x", "string"]], "powershell here-string body");
  eq(p[2][1], null, "powershell here-string closes");
  const s = run("shell", ["cat <<-'END' > out", "\tbody", "\tEND", "echo $x"]);
  eq(s[1][0], [["\tbody", "string"]], "shell heredoc body");
  eq(s[2][1], null, "shell heredoc ends at its delimiter");
  eq(s[3][0], [["echo", "function"], ["$x", "variable"]], "shell after heredoc");
  const cm = run("cmake", ['message("multi', 'line ${X}")']);
  eq(cm[1][0], [["line ", "string"], ["${X}", "variable"], ['"', "string"]], "cmake quoted argument across lines");
  const php = run("php", ["<div><?php", "$a = 'x'; ?>", "<b><?= $b ?></b>"]);
  eq(php[1][0], [["$a", "variable"], ["=", "operator"], ["'x'", "string"], ["?>", "meta"]], "php block across lines");
  eq(php[2][0], [["<b", "tag"], [">", "tag"], ["<?=", "meta"], ["$b", "variable"], ["?>", "meta"], ["</b", "tag"], [">", "tag"]], "php short echo tag");
  eq(php[2][1], null, "php returns to html");
  const cs = run("csharp", ['var s = @"C:\\', 'dir""x";']);
  eq(cs[1][0], [['dir""x"', "string"]], "csharp verbatim string across lines");
  const go = run("go", ["s := `a", "b` + x"]);
  eq(go[1][0], [["b`", "string"], ["+", "operator"]], "go raw string across lines");
  const df = run("dockerfile", ["RUN a && \\", "    b # not a comment? still shell", "COPY x y"]);
  eq(df[2][0], [["COPY", "keyword"]], "dockerfile instruction after continuation");
  const mk = run("makefile", ["define BODY", "  $(X) text", "endef", "all:"]);
  eq(mk[1][0], [["$(X)", "variable"]], "makefile define body");
  eq(mk[3][0], [["all", "function"], [":", "operator"]], "makefile after endef");
}

// ---------------------------------------------------------------- canonical states
{
  const probes = [
    ["c", "x /* open"], ["javascript", "a = `t ${"], ["python", "'''open"], ["rust", 'r#"raw'], ["html", "<div a=\"x"], ["html", "<script>"],
    ["markdown", "```rust"], ["yaml", "k: |"], ["csv", 'a,"b'], ["toml", 'a = """'], ["shell", "cat <<EOF"], ["css", "a { color:"],
    ["powershell", "<# x"], ["latex", "$$ x"], ["cmake", "#[[ c"], ["xml", "<![CDATA[ x"], ["lua", "--[[ c"], ["ruby", "=begin"], ["php", "<?php /* c"],
  ];
  for (const [id, line] of probes) {
    const a = L.tokenize(id, line, null);
    const b = L.tokenize(id, line, null);
    ok(typeof a.state === "string", `${id}: ${JSON.stringify(line)} leaves an open state`);
    eq(a, b, `${id}: same line, same result`);
    const next1 = L.tokenize(id, "continued", a.state);
    const next2 = L.tokenize(id, "continued", b.state);
    eq(next1, next2, `${id}: same state, same next line`);
  }
  // Two different routes into the same situation produce the same state string.
  eq(L.tokenize("c", "/* a", null).state, L.tokenize("c", "int x; /* b", null).state, "c comment state is canonical");
  eq(L.tokenize("c", "x */", L.tokenize("c", "/* a", null).state).state, null, "end of comment returns null");
  eq(L.tokenize("javascript", "`", null).state, L.tokenize("javascript", "x = `abc", null).state, "template state is canonical");
  eq(L.tokenize("html", "<p>", null).state, null, "html text state is null");
  eq(L.tokenize("css", "a { b: c; }", null).state, null, "css closed block is null");
  eq(L.tokenize("css", "a {", null).state, L.tokenize("css", "b, c {", null).state, "css block state is canonical");
  eq(L.tokenize("markdown", "plain", null).state, null, "markdown text state is null");
}

// ---------------------------------------------------------------- invariants (fuzz)
{
  let seed = 12345;
  const rnd = (n) => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) % n);
  const alphabet = [..."abcXYZ019 _\t\"'`\\/*#;:-+=<>{}[]()!@$%^&|?,.~"].concat(["é", "漢", "😀", "\u0301", "<!--", "-->", "*/", "/*", "${", "\"\"\"", "'''", "```", "[[", "]]", "<#", "#>", "<?php", "?>", "<script>", "</script>", "#[", "r#\"", "@\"", "--[[", "|", ">-"]);
  const line = () => { let s = ""; const n = rnd(40); for (let i = 0; i < n; i++) s += alphabet[rnd(alphabet.length)]; return s; };
  const types = new Set(L.types);
  let bad = 0;
  for (const lang of L.languages) {
    let st = null;
    for (let k = 0; k < 300; k++) {
      const text = line();
      let r;
      try { r = L.tokenize(lang.id, text, st); } catch (e) { bad++; print(`THROW ${lang.id} ${JSON.stringify(text)} ${e}`); break; }
      let at = 0;
      for (const t of r.tokens) {
        if (!(Array.isArray(t) && t.length === 3 && t[0] >= at && t[1] > t[0] && t[1] <= text.length && types.has(t[2]) && /\S/.test(text.slice(t[0], t[1])))) {
          if (bad++ < 10) print(`BAD TOKEN ${lang.id} ${JSON.stringify(text)} state=${JSON.stringify(st)} ${JSON.stringify(t)} in ${JSON.stringify(r.tokens)}`);
        }
        at = t[1];
      }
      if (!(r.state === null || typeof r.state === "string")) { bad++; print(`BAD STATE ${lang.id} ${JSON.stringify(r.state)}`); }
      const again = L.tokenize(lang.id, text, st);
      if (JSON.stringify(again) !== JSON.stringify(r)) { bad++; print(`NONDETERMINISTIC ${lang.id} ${JSON.stringify(text)}`); }
      st = r.state;
    }
  }
  eq(bad, 0, "fuzz: tokens sorted, in range, non-empty, known types; states null or strings; deterministic");
}

// ---------------------------------------------------------------- performance
{
  const sample = {
    javascript: [
      'import { readFile } from "node:fs/promises";',
      "export async function load(path, opts = {}) {",
      "  const text = await readFile(path, \"utf8\"); // read it",
      "  const re = /^(\\w+)\\s*=\\s*(.*)$/gm, out = new Map();",
      "  for (const [, k, v] of text.matchAll(re)) out.set(k, v.trim() || null);",
      "  /* block comment with `ticks` */",
      "  return { size: out.size, keys: [...out.keys()].join(`, ${opts.sep ?? \"\"}`) };",
      "}",
    ],
    python: ["@dataclass", "class Point(Base):", '    """A point."""', "    def dist(self, other: 'Point') -> float:", "        return math.hypot(self.x - other.x, self.y - other.y)  # euclid", "    items = [f\"{i:02d}\" for i in range(10) if i % 2 == 0]"],
    rust: ["#[derive(Debug, Clone)]", "pub struct Cache<'a> { map: HashMap<&'a str, Vec<u8>>, hits: u64 }", "impl<'a> Cache<'a> {", "    pub fn get(&mut self, k: &str) -> Option<&Vec<u8>> { self.hits += 1; self.map.get(k) } // lookup", "    fn raw() -> &'static str { r#\"raw \"text\"\"# }", "}"],
    html: ['<div class="card" data-id="42">', "  <h2>Title &amp; more</h2>", '  <a href="/x?y=1" target=_blank>link</a>', "  <!-- comment -->", "  <script>const a = [1, 2, 3].map((x) => x * 2);</script>", "</div>"],
    markdown: ["# Heading", "Some *emphasis*, **strong** and `code` with a [link](https://example.com).", "- item one", "  - nested `x`", "```js", "const x = 1;", "```", "> quote"],
  };
  for (const [id, lines] of Object.entries(sample)) {
    const all = [];
    for (let i = 0; i < 100000; i++) all.push(lines[i % lines.length]);
    let st = null;
    let count = 0;
    const t0 = Date.now();
    for (const text of all) {
      const r = L.tokenize(id, text, st);
      st = r.state;
      count += r.tokens.length;
    }
    const ms = Date.now() - t0;
    print(`perf ${id}: 100000 lines in ${ms} ms (${count} tokens)`);
    if (id === "javascript") ok(ms < 1500, `javascript tokenizes 100k lines quickly (${ms} ms)`);
    else ok(ms < 3000, `${id} tokenizes 100k lines in reasonable time (${ms} ms)`);
  }
}

print(`lang: ${passed} passed, ${failed} failed`);
if (failed) System.exit(1);
