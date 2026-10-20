// Axentra shell DOM runtime. Installed once by the shell (kit::dom::install).
// Handles what has to happen synchronously in the page: roving focus in
// composite widgets, the modal focus trap, focus restoration, F6 regions,
// styled tooltips, ribbon key tips, live-region announcements and default
// actions the webview must not run (browser shortcuts).
(function () {
  if (window.axShell) return;
  const ax = (window.axShell = {});
  const doc = document;

  // ------------------------------------------------------------ helpers
  const TEXT_INPUTS = ["text", "search", "email", "url", "tel", "password", "number", "date", "time", "datetime-local", "month", "week"];
  const isEditable = (el) => {
    if (!el || el.nodeType !== 1) return false;
    if (el.isContentEditable) return true;
    const t = el.tagName;
    if (t === "TEXTAREA" || t === "SELECT") return true;
    if (t === "INPUT") return TEXT_INPUTS.includes((el.type || "text").toLowerCase());
    return false;
  };
  const visible = (el) =>
    !!el && el.isConnected && (el.offsetWidth > 0 || el.offsetHeight > 0 || el.getClientRects().length > 0) &&
    getComputedStyle(el).visibility !== "hidden";
  const FOCUSABLE =
    'a[href],button,input:not([type=hidden]),select,textarea,[tabindex],[contenteditable="true"]';
  const focusable = (el) => !el.disabled && !el.closest("[inert]") && visible(el);
  const tabbables = (root) =>
    Array.from(root.querySelectorAll(FOCUSABLE)).filter((el) => el.tabIndex >= 0 && focusable(el));

  // Programmatic focus shows the ring only when the user is on the keyboard.
  ax.focus = (el) => {
    if (!el) return false;
    try {
      el.focus({ focusVisible: doc.documentElement.hasAttribute("data-ax-kbd") });
    } catch (_) {
      el.focus();
    }
    return doc.activeElement === el;
  };
  ax.focusSel = (sel) => ax.focus(doc.querySelector(sel));
  ax.focusFirstIn = (sel, itemSel) => {
    const root = doc.querySelector(sel);
    if (!root) return false;
    const items = Array.from(root.querySelectorAll(itemSel || FOCUSABLE)).filter(focusable);
    return ax.focus(items[0] || root);
  };

  // Same normalisation as kit::keys::chord on the Rust side.
  const chord = (e) => {
    const c = e.code || "";
    let k;
    if (c.startsWith("Key")) k = c.slice(3);
    else if (c.startsWith("Digit")) k = c.slice(5);
    else if (e.key === " ") k = "Space";
    else if (e.key && e.key.length === 1) k = e.key.toUpperCase();
    else k = e.key;
    let s = "";
    if (e.ctrlKey || e.metaKey) s += "Ctrl+";
    if (e.altKey) s += "Alt+";
    if (e.shiftKey) s += "Shift+";
    return s + k;
  };

  // ------------------------------------------------------------ default actions
  // Chords the webview must not act on (tab switching, reload, find...).
  // `always` applies everywhere, `outside` only outside text fields.
  ax.always = new Set();
  ax.outside = new Set();
  ax.setPrevent = (always, outside) => {
    ax.always = new Set(always);
    ax.outside = new Set(outside);
  };

  // ------------------------------------------------------------ keyboard modality
  const root = doc.documentElement;
  const setKbd = (on) => {
    if (on) root.setAttribute("data-ax-kbd", "");
    else root.removeAttribute("data-ax-kbd");
  };

  // ------------------------------------------------------------ roving focus
  // A composite widget marks itself data-ax-roving="horizontal|vertical|both|grid"
  // and its items data-ax-item. data-ax-wrap wraps around, data-ax-typeahead
  // jumps by first letters, data-ax-cols gives the grid width.
  let typeBuf = "";
  let typeTimer = 0;
  const items = (box) =>
    Array.from(box.querySelectorAll("[data-ax-item]")).filter(
      (el) => el.closest("[data-ax-roving]") === box && visible(el)
    );
  const moveTo = (list, el) => {
    list.forEach((it) => {
      if (it !== el && it.tabIndex === 0) it.tabIndex = -1;
    });
    el.tabIndex = 0;
    ax.focus(el);
    el.scrollIntoView({ block: "nearest", inline: "nearest" });
  };
  const roving = (e, ch, editable) => {
    const t = e.target;
    if (!t || !t.closest) return false;
    const box = t.closest("[data-ax-roving]");
    if (!box) return false;
    const current = t.closest("[data-ax-item]");
    if (editable && !(current && current === t)) return false;
    const list = items(box);
    if (!list.length) return false;
    const orient = box.getAttribute("data-ax-roving");
    const wrap = box.hasAttribute("data-ax-wrap");
    const cols = parseInt(box.getAttribute("data-ax-cols") || "1", 10);
    const i = list.indexOf(current);
    const vert = orient !== "horizontal";
    const horiz = orient !== "vertical";
    let step = 0;
    if (ch === "ArrowDown" && vert) step = orient === "grid" ? cols : 1;
    else if (ch === "ArrowUp" && vert) step = orient === "grid" ? -cols : -1;
    else if (ch === "ArrowRight" && horiz) step = 1;
    else if (ch === "ArrowLeft" && horiz) step = -1;
    let next = -1;
    if (step) {
      next = i < 0 ? 0 : i + step;
      if (next >= list.length) next = wrap ? next % list.length : list.length - 1;
      if (next < 0) next = wrap ? list.length + next : 0;
    } else if (ch === "Home") next = 0;
    else if (ch === "End") next = list.length - 1;
    else if (box.hasAttribute("data-ax-typeahead") && /^(Shift\+)?.$/.test(ch) && !editable) {
      clearTimeout(typeTimer);
      typeTimer = setTimeout(() => (typeBuf = ""), 600);
      typeBuf += e.key.toLowerCase();
      const start = typeBuf.length === 1 ? i + 1 : i;
      for (let n = 0; n < list.length; n++) {
        const el = list[(start + n + list.length) % list.length];
        const label = (el.getAttribute("aria-label") || el.textContent || "").trim().toLowerCase();
        if (label.startsWith(typeBuf)) {
          next = list.indexOf(el);
          break;
        }
      }
    }
    if (next < 0 || next === i) {
      if (step || ch === "Home" || ch === "End") e.preventDefault();
      return false;
    }
    e.preventDefault();
    moveTo(list, list[next]);
    return true;
  };
  // Keep exactly one tab stop per composite: the item that last had focus.
  const syncTabStop = (t) => {
    const it = t.closest && t.closest("[data-ax-item]");
    if (!it) return;
    const box = it.closest("[data-ax-roving]");
    if (!box) return;
    items(box).forEach((el) => (el.tabIndex = el === it ? 0 : -1));
  };

  // ------------------------------------------------------------ modal focus trap
  const topModal = () => {
    const m = doc.querySelectorAll('[aria-modal="true"]');
    return m.length ? m[m.length - 1] : null;
  };
  const trapTab = (e) => {
    const m = topModal();
    if (!m) return;
    const els = tabbables(m);
    if (!els.length) {
      e.preventDefault();
      ax.focus(m);
      return;
    }
    const first = els[0];
    const last = els[els.length - 1];
    const a = doc.activeElement;
    if (!m.contains(a)) {
      e.preventDefault();
      ax.focus(e.shiftKey ? last : first);
    } else if (e.shiftKey && (a === first || a === m)) {
      e.preventDefault();
      ax.focus(last);
    } else if (!e.shiftKey && a === last) {
      e.preventDefault();
      ax.focus(first);
    }
  };

  // ------------------------------------------------------------ focus restoration
  // Popups push the element that opened them; when the focused element is
  // removed from the page (menu closed, dialog closed, row deleted) focus
  // goes back to the most recent invoker that still exists, else into the
  // region it was in. Focus never stays on <body>.
  const stack = [];
  let lastFocused = null;
  ax.push = () => {
    const a = doc.activeElement;
    if (a && a !== doc.body) {
      stack.push(a);
      if (stack.length > 20) stack.shift();
    }
  };
  ax.openPopup = (id, itemSel) => {
    ax.push();
    const el = doc.getElementById(id);
    if (!el) return;
    ax.fit(el);
    const list = Array.from(el.querySelectorAll(itemSel || "[data-ax-item]")).filter(focusable);
    const first =
      list.find((x) => x.getAttribute("aria-current") === "page" || x.getAttribute("aria-selected") === "true") ||
      list[0] ||
      tabbables(el)[0];
    ax.focus(first || el);
  };
  // Who opened each dialog, so closing it gives focus back (closeDialog).
  const openers = new Map();
  ax.openDialog = (id, initial) => {
    const opener = doc.activeElement;
    ax.push();
    const el = doc.getElementById(id);
    if (!el) return;
    if (opener && opener !== doc.body && !el.contains(opener)) openers.set(id, opener);
    const body = el.querySelector(".ax-dialog-body") || el;
    // A confirmation with nothing to fill in starts on its default button,
    // not on the title bar's Close.
    const pick =
      (initial && el.querySelector(initial)) ||
      el.querySelector("[data-ax-autofocus]") ||
      tabbables(body)[0] ||
      el.querySelector('[data-ax-default]:not([hidden]):not([aria-disabled="true"])') ||
      tabbables(el)[0];
    ax.focus(pick || el);
  };
  // After the dialog left the page: back to its opener unless focus already
  // went somewhere meaningful (another dialog, a field the module chose).
  ax.closeDialog = (id) => {
    const opener = openers.get(id);
    openers.delete(id);
    setTimeout(() => {
      const a = doc.activeElement;
      const lost = !a || a === doc.body || !a.isConnected;
      const modal = topModal();
      if (!lost || !opener || !opener.isConnected || !focusable(opener)) return;
      if (modal && !modal.contains(opener)) return;
      ax.focus(opener);
    }, 0);
  };
  // Enter in a dialog with a default button ([data-ax-default]) presses it,
  // unless focus is on something that handles Enter itself.
  const ownsEnter = (t) =>
    !t ||
    !t.closest ||
    t.closest(
      'button, a[href], textarea, select, [contenteditable="true"], .ax-menu, ' +
        '[role="button"], [role="menuitem"], [role="option"], [role="tab"], [role="checkbox"], [role="radio"], ' +
        '[role="switch"], [role="combobox"], [role="listbox"], [role="grid"], [role="tree"], ' +
        'input[type="button"], input[type="submit"], input[type="checkbox"], input[type="radio"], input[type="file"]'
    );
  const defaultButton = (e) => {
    const m = topModal();
    if (!m || e.isComposing || ownsEnter(e.target) || !m.contains(e.target)) return false;
    const btn = m.querySelector('[data-ax-default]:not([aria-disabled="true"])');
    if (!btn) return false;
    e.preventDefault();
    e.stopPropagation();
    btn.click();
    return true;
  };
  const restore = () => {
    const a = doc.activeElement;
    if (a && a !== doc.body) return;
    if (!lastFocused || lastFocused.isConnected) return;
    const lostRegion = lastFocused.__axRegion;
    lastFocused = null;
    const modal = topModal();
    while (stack.length) {
      const el = stack.pop();
      if (el.isConnected && focusable(el) && (!modal || modal.contains(el))) {
        ax.focus(el);
        return;
      }
    }
    if (modal) {
      ax.focus(tabbables(modal)[0] || modal);
      return;
    }
    if (lostRegion && lostRegion.isConnected) enter(lostRegion);
    else {
      const app = doc.querySelector(".ax-app");
      if (app) app.focus({ preventScroll: true });
    }
  };

  // ------------------------------------------------------------ F6 regions
  // Header -> app rail -> ribbon -> module panes -> open side pane -> toasts.
  // Modules can mark their own regions with data-ax-region; otherwise each
  // `.ax-panes > .ax-pane` is a region.
  ax.regions = () => {
    const out = [];
    const add = (el) => {
      if (el && visible(el) && !out.includes(el)) out.push(el);
    };
    add(doc.querySelector(".ax-header"));
    add(doc.querySelector(".ax-rail"));
    const mod = doc.querySelector(".ax-module");
    if (mod) {
      add(mod.querySelector(".ax-ribbon"));
      let regs = Array.from(mod.querySelectorAll("[data-ax-region]"));
      if (!regs.length) regs = Array.from(mod.querySelectorAll(".ax-panes > .ax-pane"));
      regs.forEach(add);
    }
    doc.querySelectorAll(".ax-sidepanel").forEach(add);
    const toasts = doc.querySelector(".ax-toasts");
    if (toasts && toasts.querySelector(".ax-toast")) add(toasts);
    return out;
  };
  const regionOf = (el) => {
    const rs = ax.regions();
    let best = null;
    rs.forEach((r) => {
      if (r.contains(el) && (!best || best.contains(r))) best = r;
    });
    return best;
  };
  const enter = (r) => {
    const last = r.__axLast;
    if (last && last.isConnected && r.contains(last) && focusable(last) && ax.focus(last)) return;
    const picks = [
      "[data-ax-default-focus]",
      '[aria-current="page"]',
      '[role="tab"][aria-selected="true"]',
      '[aria-selected="true"]',
      ".ax-selected",
    ];
    for (const sel of picks) {
      const el = Array.from(r.querySelectorAll(sel)).find((x) => focusable(x) && (x.tabIndex >= 0 || x.hasAttribute("tabindex")));
      if (el && ax.focus(el)) return;
    }
    const first = tabbables(r)[0];
    if (first && ax.focus(first)) return;
    if (!r.hasAttribute("tabindex")) r.tabIndex = -1;
    ax.focus(r);
  };
  ax.enterRegion = (sel) => {
    const r = doc.querySelector(sel);
    if (r) enter(r);
  };
  ax.cycle = (dir) => {
    if (topModal()) return;
    const rs = ax.regions();
    if (!rs.length) return;
    const a = doc.activeElement;
    let i = -1;
    rs.forEach((r, n) => {
      if (r.contains(a) && (i < 0 || rs[i].contains(r))) i = n;
    });
    const n = i < 0 ? (dir > 0 ? 0 : rs.length - 1) : (i + dir + rs.length) % rs.length;
    enter(rs[n]);
  };

  // ------------------------------------------------------------ tooltips
  // Any element with a title (or data-ax-tip) gets a styled tooltip after
  // 500 ms of hover, or on keyboard focus for icon-only controls. The native
  // title is moved to data-ax-tip so the browser does not show its own.
  let tipFor = null;
  let tipTimer = 0;
  const overlay = () => doc.getElementById("ax-overlay-root");
  const takeTitle = (el) => {
    if (el.hasAttribute("title")) {
      const t = el.getAttribute("title");
      el.removeAttribute("title");
      if (t) {
        el.setAttribute("data-ax-tip", t);
        if (!el.hasAttribute("aria-label") && !el.textContent.trim()) el.setAttribute("aria-label", t);
      } else {
        el.removeAttribute("data-ax-tip");
      }
    }
    return el.getAttribute("data-ax-tip");
  };
  const tipTarget = (n) => (n && n.closest ? n.closest("[data-ax-tip],[title]") : null);
  const keysLabel = (s) => (s || "").split(" ").map((k) => k.replace(/Control/g, "Ctrl")).join(" or ");
  const hideTip = () => {
    clearTimeout(tipTimer);
    if (tipFor) tipFor.removeAttribute("aria-describedby");
    tipFor = null;
    const old = doc.getElementById("ax-tooltip");
    if (old) old.remove();
  };
  const showTip = (el) => {
    const host = overlay();
    if (!host || !el.isConnected) return;
    const text = el.getAttribute("data-ax-tip");
    if (!text) return;
    const old = doc.getElementById("ax-tooltip");
    if (old) old.remove();
    const tip = doc.createElement("div");
    tip.id = "ax-tooltip";
    tip.className = "ax-tooltip";
    tip.setAttribute("role", "tooltip");
    const keys = el.getAttribute("aria-keyshortcuts");
    tip.textContent = keys && !text.includes("(") ? text + " (" + keysLabel(keys) + ")" : text;
    host.appendChild(tip);
    const r = el.getBoundingClientRect();
    const tr = tip.getBoundingClientRect();
    const pos = el.getAttribute("data-ax-tip-pos") || "below";
    let x, y;
    if (pos === "right") {
      x = r.right + 8;
      y = r.top + r.height / 2 - tr.height / 2;
    } else {
      x = r.left + r.width / 2 - tr.width / 2;
      y = r.bottom + 6;
      if (y + tr.height > innerHeight - 4) y = r.top - tr.height - 6;
    }
    x = Math.max(4, Math.min(x, innerWidth - tr.width - 4));
    y = Math.max(4, Math.min(y, innerHeight - tr.height - 4));
    tip.style.left = x + "px";
    tip.style.top = y + "px";
    el.setAttribute("aria-describedby", "ax-tooltip");
  };
  const scheduleTip = (el, delay) => {
    hideTip();
    if (!el || !el.closest(".ax-app")) return;
    if (!takeTitle(el)) return;
    tipFor = el;
    tipTimer = setTimeout(() => showTip(el), delay);
  };

  // ------------------------------------------------------------ key tips
  // Alt pressed and released alone shows letter badges on the ribbon tabs
  // (data-ax-keytip); typing a tab's letters opens it and shows its
  // commands' badges; typing a command's letters runs it. Esc backs out.
  ax.keytips = true;
  ax.setKeytips = (on) => (ax.keytips = !!on);
  const kt = { level: 0, prev: null, buf: "", targets: [] };
  let altAlone = false;
  const ribbon = () => doc.querySelector(".ax-module .ax-ribbon");
  const clearBadges = () => {
    doc.querySelectorAll(".ax-keytip").forEach((b) => b.remove());
  };
  const showBadges = (targets) => {
    clearBadges();
    const host = overlay();
    kt.targets = targets.filter(visible);
    kt.buf = "";
    if (!host) return;
    kt.targets.forEach((el) => {
      const r = el.getBoundingClientRect();
      const b = doc.createElement("div");
      b.className = "ax-keytip";
      b.setAttribute("aria-hidden", "true");
      b.textContent = el.getAttribute("data-ax-keytip");
      host.appendChild(b);
      const br = b.getBoundingClientRect();
      b.style.left = Math.max(2, r.left + r.width / 2 - br.width / 2) + "px";
      b.style.top = Math.min(innerHeight - br.height - 2, r.bottom - 4) + "px";
    });
  };
  const exitKeytips = (restoreFocus) => {
    clearBadges();
    if (restoreFocus && kt.prev && kt.prev.isConnected) ax.focus(kt.prev);
    kt.level = 0;
    kt.prev = null;
  };
  const tabBadges = () => {
    const rb = ribbon();
    if (!rb) return exitKeytips(false);
    kt.level = 1;
    showBadges(Array.from(rb.querySelectorAll('[role="tab"][data-ax-keytip]')));
  };
  const commandBadges = (tabId, tries) => {
    const rb = ribbon();
    const row = rb && rb.querySelector(".ax-ribbon-commands");
    if (!row || (tabId && row.getAttribute("data-ax-tab") !== tabId)) {
      if (tries > 0) setTimeout(() => commandBadges(tabId, tries - 1), 40);
      return;
    }
    kt.level = 2;
    showBadges(Array.from(row.querySelectorAll("[data-ax-keytip]")));
  };
  const keytipKey = (e, ch) => {
    e.preventDefault();
    e.stopPropagation();
    if (ch === "Escape") {
      if (kt.level === 2) tabBadges();
      else exitKeytips(true);
      return;
    }
    if (e.key === "Alt" || e.key === "Shift" || e.key === "Control") return;
    if (e.key.length !== 1) {
      exitKeytips(false);
      return;
    }
    kt.buf += e.key.toUpperCase();
    const matches = kt.targets.filter((el) => (el.getAttribute("data-ax-keytip") || "").startsWith(kt.buf));
    const exact = matches.find((el) => el.getAttribute("data-ax-keytip") === kt.buf);
    if (exact) {
      if (kt.level === 1) {
        exact.click();
        ax.focus(exact);
        clearBadges();
        commandBadges(exact.getAttribute("data-ax-tab-id"), 25);
      } else {
        exitKeytips(false);
        ax.focus(exact);
        exact.click();
      }
    } else if (!matches.length) {
      kt.buf = "";
    } else {
      doc.querySelectorAll(".ax-keytip").forEach((b) => {
        if (!b.textContent.startsWith(kt.buf)) b.style.visibility = "hidden";
      });
    }
  };

  // ------------------------------------------------------------ live regions
  ax.announce = (text, assertive) => {
    const el = doc.getElementById(assertive ? "ax-live-assertive" : "ax-live-polite");
    if (!el) return;
    el.textContent = "";
    setTimeout(() => (el.textContent = text), 100);
  };

  // ------------------------------------------------------------ positioning
  // Keep a fixed or absolutely positioned popup inside the viewport.
  ax.fit = (elOrId) => {
    const el = typeof elOrId === "string" ? doc.getElementById(elOrId) : elOrId;
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (el.classList.contains("ax-submenu")) {
      if (r.right > innerWidth - 4) {
        el.style.left = "auto";
        el.style.right = "calc(100% + 4px)";
      }
      if (r.bottom > innerHeight - 4) el.style.top = innerHeight - 4 - r.bottom + "px";
    }
  };
  ax.flash = (sel) => {
    const el = doc.querySelector(sel);
    if (!el) return;
    el.scrollIntoView({ block: "center" });
    el.classList.add("ax-flash");
    setTimeout(() => el.classList.remove("ax-flash"), 1300);
  };

  // ------------------------------------------------------------ keys with no focus
  // With nothing focused, key events target <body>, outside the app root
  // that handles global shortcuts; re-send them to the root.
  const forwardFromBody = (e) => {
    if (e.__axForwarded || (e.target !== doc.body && e.target !== root)) return;
    const app = doc.querySelector(".ax-app");
    if (!app) return;
    const f = new KeyboardEvent("keydown", {
      key: e.key, code: e.code, location: e.location, repeat: e.repeat, isComposing: e.isComposing,
      ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, metaKey: e.metaKey, bubbles: true, cancelable: true,
    });
    f.__axForwarded = true;
    app.dispatchEvent(f);
  };
  // Start with focus in the app, not on <body>.
  const app0 = doc.querySelector(".ax-app");
  if (app0 && (!doc.activeElement || doc.activeElement === doc.body)) app0.focus({ preventScroll: true });

  // ------------------------------------------------------------ listeners
  window.addEventListener(
    "keydown",
    (e) => {
      setKbd(true);
      if (e.isComposing) return;
      const ch = chord(e);
      const editable = isEditable(e.target);
      if (e.key === "Alt") {
        altAlone = !e.repeat;
        return;
      }
      altAlone = false;
      if (kt.level) {
        keytipKey(e, ch);
        return;
      }
      if (ax.always.has(ch) || (!editable && ax.outside.has(ch))) e.preventDefault();
      if (ch === "Escape") hideTip();
      if (ch === "Enter" && defaultButton(e)) return;
      if (ch === "Tab" || ch === "Shift+Tab") trapTab(e);
      else if (!roving(e, ch, editable)) forwardFromBody(e);
    },
    true
  );
  window.addEventListener(
    "keyup",
    (e) => {
      if (e.key !== "Alt") return;
      if (altAlone && ax.keytips && !topModal()) {
        e.preventDefault();
        if (kt.level) exitKeytips(true);
        else {
          kt.prev = doc.activeElement;
          tabBadges();
        }
      }
      altAlone = false;
    },
    true
  );
  // Drop targets (data-ax-drop) accept drags; the component handles the drop.
  doc.addEventListener(
    "dragover",
    (e) => {
      if (e.target.closest && e.target.closest("[data-ax-drop]")) e.preventDefault();
    },
    true
  );
  // Toolbar buttons marked data-ax-keep-focus don't take focus from an
  // editor (so formatting applies to its selection).
  doc.addEventListener(
    "mousedown",
    (e) => {
      if (e.target.closest && e.target.closest("[data-ax-keep-focus]")) e.preventDefault();
    },
    true
  );
  window.addEventListener(
    "pointerdown",
    () => {
      setKbd(false);
      altAlone = false;
      hideTip();
      if (kt.level) exitKeytips(false);
    },
    true
  );
  window.addEventListener("blur", () => {
    altAlone = false;
    if (kt.level) exitKeytips(false);
  });
  doc.addEventListener("wheel", hideTip, { capture: true, passive: true });
  doc.addEventListener(
    "pointerover",
    (e) => {
      if (e.pointerType && e.pointerType !== "mouse") return;
      const el = tipTarget(e.target);
      if (el === tipFor) return;
      if (!el) {
        hideTip();
        return;
      }
      scheduleTip(el, 500);
    },
    true
  );
  doc.addEventListener(
    "pointerout",
    (e) => {
      if (tipFor && !(e.relatedTarget && tipFor.contains(e.relatedTarget))) hideTip();
    },
    true
  );
  doc.addEventListener(
    "focusin",
    (e) => {
      const t = e.target;
      lastFocused = t;
      syncTabStop(t);
      const r = regionOf(t);
      if (r) {
        r.__axLast = t;
        t.__axRegion = r;
      }
      if (root.hasAttribute("data-ax-kbd")) {
        const el = tipTarget(t);
        if (el === t && !t.textContent.trim()) scheduleTip(el, 300);
        else hideTip();
      }
    },
    true
  );
  new MutationObserver(restore).observe(doc.body, { childList: true, subtree: true });
})();
