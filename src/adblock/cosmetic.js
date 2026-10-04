// Cosmetic filtering agent. Runs in the main world of every frame before any
// of the page's scripts (src/adblock.zig `frameScript` calls it with the rules
// adblock-rust resolved for this frame), so everything it needs from the page's
// globals is captured here, before the page can replace it. It keeps no
// global of its own.
(function (rules) {
  "use strict";
  var apply = Reflect.apply;
  var XHR = XMLHttpRequest;
  var xhrOpen = XHR.prototype.open;
  var xhrSend = XHR.prototype.send;
  var addListener = EventTarget.prototype.addEventListener;
  var Observer = MutationObserver;
  var Sheet = CSSStyleSheet;
  var adoptedSet = Object.getOwnPropertyDescriptor(Document.prototype, "adoptedStyleSheets");
  var stringify = JSON.stringify;
  var parse = JSON.parse;
  var encode = encodeURIComponent;
  var doc = document;

  var sheet = null;
  var hidden = new Set();
  var pending = [];

  function addSelectors(list) {
    for (var i = 0; i < list.length; i++) {
      var sel = list[i];
      if (!sel || hidden.has(sel)) continue;
      hidden.add(sel);
      pending.push(sel);
    }
    flushSheet();
  }

  function flushSheet() {
    if (!pending.length) return;
    if (!sheet) {
      try {
        sheet = new Sheet();
      } catch (e) {
        return;
      }
    }
    // One rule per selector: an invalid selector must not take the others
    // down with it, which is what a single grouped rule would do.
    for (var i = 0; i < pending.length; i++) {
      try {
        sheet.insertRule(pending[i] + "{display:none!important}", sheet.cssRules.length);
      } catch (e) {}
    }
    pending = [];
    adopt();
  }

  function adopt() {
    if (!sheet || !adoptedSet) return;
    var current = adoptedSet.get.call(doc);
    if (current.indexOf(sheet) === -1) adoptedSet.set.call(doc, current.concat([sheet]));
  }

  // ---- procedural filters -------------------------------------------------

  function textMatcher(arg) {
    var m = /^\/(.*)\/([imsu]*)$/.exec(arg);
    if (m) {
      try {
        var re = new RegExp(m[1], m[2]);
        return function (t) { return re.test(t); };
      } catch (e) {
        return function () { return false; };
      }
    }
    return function (t) { return t.indexOf(arg) !== -1; };
  }

  function step(nodes, op) {
    var out = [];
    var i, n;
    switch (op.type) {
      case "css-selector":
        for (i = 0; i < nodes.length; i++) {
          try {
            var found = nodes[i].querySelectorAll(":scope " + op.arg);
            for (n = 0; n < found.length; n++) out.push(found[n]);
          } catch (e) {}
        }
        return out;
      case "has-text": {
        var match = textMatcher(op.arg);
        return nodes.filter(function (el) { return match(el.textContent || ""); });
      }
      case "min-text-length": {
        var min = parseInt(op.arg, 10) || 0;
        return nodes.filter(function (el) { return (el.textContent || "").length >= min; });
      }
      case "matches-css":
      case "matches-css-before":
      case "matches-css-after": {
        var pseudo = op.type === "matches-css-before" ? "::before" : op.type === "matches-css-after" ? "::after" : null;
        var colon = op.arg.indexOf(":");
        if (colon < 0) return [];
        var prop = op.arg.slice(0, colon).trim();
        var want = textMatcher(op.arg.slice(colon + 1).trim());
        return nodes.filter(function (el) {
          return want(getComputedStyle(el, pseudo).getPropertyValue(prop));
        });
      }
      case "matches-attr": {
        var eq = op.arg.indexOf("=");
        var name = textMatcher(eq < 0 ? op.arg : op.arg.slice(0, eq).replace(/^"|"$/g, ""));
        var value = eq < 0 ? null : textMatcher(op.arg.slice(eq + 1).replace(/^"|"$/g, ""));
        return nodes.filter(function (el) {
          for (var a = 0; a < el.attributes.length; a++) {
            var at = el.attributes[a];
            if (name(at.name) && (!value || value(at.value))) return true;
          }
          return false;
        });
      }
      case "matches-path": {
        var path = textMatcher(op.arg);
        return path(location.pathname + location.search) ? nodes : [];
      }
      case "upward": {
        var count = parseInt(op.arg, 10);
        for (i = 0; i < nodes.length; i++) {
          var el = nodes[i];
          if (count > 0) {
            for (n = 0; n < count && el; n++) el = el.parentElement;
          } else {
            el = el.parentElement ? el.parentElement.closest(op.arg) : null;
          }
          if (el && out.indexOf(el) === -1) out.push(el);
        }
        return out;
      }
      case "xpath":
        for (i = 0; i < nodes.length; i++) {
          try {
            var r = doc.evaluate(op.arg, nodes[i], null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
            for (n = 0; n < r.snapshotLength; n++) {
              var node = r.snapshotItem(n);
              if (node.nodeType === 1) out.push(node);
            }
          } catch (e) {}
        }
        return out;
      default:
        return [];
    }
  }

  function applyAction(el, action) {
    if (!action) {
      el.style.setProperty("display", "none", "important");
      return;
    }
    switch (action.type) {
      case "remove":
        el.remove();
        break;
      case "style":
        el.setAttribute("style", (el.getAttribute("style") || "") + ";" + action.arg);
        break;
      case "remove-attr":
        el.removeAttribute(action.arg);
        break;
      case "remove-class":
        el.classList.remove(action.arg);
        break;
    }
  }

  var procedural = [];
  function parseProcedural(list) {
    for (var i = 0; i < list.length; i++) {
      try {
        var f = parse(list[i]);
        // A plain selector with a style action needs no DOM walk at all.
        if (f.selector.length === 1 && f.selector[0].type === "css-selector" && !f.action) {
          addSelectors([f.selector[0].arg]);
        } else {
          procedural.push(f);
        }
      } catch (e) {}
    }
  }

  function runProcedural() {
    if (!doc.documentElement) return;
    for (var i = 0; i < procedural.length; i++) {
      var f = procedural[i];
      var nodes = [doc.documentElement];
      var first = f.selector[0];
      if (first && first.type === "css-selector") {
        try {
          nodes = Array.prototype.slice.call(doc.querySelectorAll(first.arg));
        } catch (e) {
          continue;
        }
      } else {
        nodes = step(nodes, first);
      }
      for (var s = 1; s < f.selector.length && nodes.length; s++) nodes = step(nodes, f.selector[s]);
      for (var k = 0; k < nodes.length; k++) applyAction(nodes[k], f.action);
    }
  }

  // ---- generic class / id rules and blocked boxes ----------------------------

  var seenClasses = new Set();
  var seenIds = new Set();
  var seenSources = new Set();
  var newClasses = [];
  var newIds = [];
  var newSources = [];
  var boxes = new Map();
  var exceptions = rules.exceptions || [];
  var generic = !rules.generichide;
  var sourceKinds = { IMG: "image", IFRAME: "sub_frame", FRAME: "sub_frame", EMBED: "object", OBJECT: "object", VIDEO: "media", AUDIO: "media" };

  function collect(el) {
    if (el.id && !seenIds.has(el.id)) {
      seenIds.add(el.id);
      newIds.push(el.id);
    }
    var cl = el.classList;
    if (cl) {
      for (var i = 0; i < cl.length; i++) {
        if (!seenClasses.has(cl[i])) {
          seenClasses.add(cl[i]);
          newClasses.push(cl[i]);
        }
      }
    }
    var kind = sourceKinds[el.tagName];
    if (kind) {
      var src = el.tagName === "OBJECT" ? el.data : el.src;
      if (src && (src.indexOf("http:") === 0 || src.indexOf("https:") === 0)) {
        var list = boxes.get(src);
        if (!list) boxes.set(src, (list = []));
        if (list.indexOf(el) === -1) list.push(el);
        if (!seenSources.has(src)) {
          seenSources.add(src);
          newSources.push({ url: src, kind: kind });
        }
      }
    }
  }

  function scan(root) {
    if (root.nodeType !== 1) return;
    collect(root);
    var all = root.querySelectorAll("[id],[class],img[src],iframe[src],frame[src],embed[src],object[data],video[src],audio[src]");
    for (var i = 0; i < all.length; i++) collect(all[i]);
  }

  // uBO collapses the box of an element whose resource it blocked, so a
  // blocked ad leaves no hole in the layout.
  function collapse(urls) {
    for (var i = 0; i < urls.length; i++) {
      var list = boxes.get(urls[i]);
      if (!list) continue;
      for (var n = 0; n < list.length; n++) list[n].style.setProperty("display", "none", "important");
    }
  }

  function report() {
    var classes = generic ? newClasses : [];
    var ids = generic ? newIds : [];
    var sources = newSources;
    newClasses = [];
    newIds = [];
    newSources = [];
    if (!classes.length && !ids.length && !sources.length) return;
    var msg = { url: location.href, classes: classes, ids: ids, exceptions: exceptions, sources: sources };
    var x = new XHR();
    apply(xhrOpen, x, ["GET", "nd-adblock://generic/?q=" + encode(stringify(msg)), true]);
    apply(addListener, x, ["load", function () {
      try {
        var answer = parse(x.responseText);
        if (answer.selectors && answer.selectors.length) addSelectors(answer.selectors);
        if (answer.blocked && answer.blocked.length) collapse(answer.blocked);
      } catch (e) {}
    }]);
    try {
      apply(xhrSend, x, []);
    } catch (e) {}
  }

  // ---- scheduling -------------------------------------------------------------

  var queued = false;
  var dirty = [];
  function schedule() {
    if (queued) return;
    queued = true;
    setTimeout(function () {
      queued = false;
      var roots = dirty;
      dirty = [];
      for (var i = 0; i < roots.length; i++) scan(roots[i]);
      report();
      if (procedural.length) runProcedural();
      adopt();
    }, 50);
  }

  function start() {
    if (doc.documentElement) scan(doc.documentElement);
    report();
    if (procedural.length) runProcedural();
    new Observer(function (records) {
      for (var i = 0; i < records.length; i++) {
        var r = records[i];
        if (r.type === "attributes") dirty.push(r.target);
        else for (var n = 0; n < r.addedNodes.length; n++) dirty.push(r.addedNodes[n]);
      }
      schedule();
    }).observe(doc, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "id", "src", "data"] });
  }

  addSelectors(rules.hide_selectors || []);
  parseProcedural(rules.procedural_actions || []);
  if (doc.readyState === "loading") apply(addListener, doc, ["DOMContentLoaded", start, { once: true }]);
  else start();
})
