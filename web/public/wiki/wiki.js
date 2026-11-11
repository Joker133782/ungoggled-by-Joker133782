/* ungoggled help center: navigation drawer, "on this page", anchor jumps and
   a small offline full-text search. No libraries; the page reads fine without
   this file, it only adds conveniences. */
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.remove("no-js");
  root.classList.add("js");

  var $ = function (sel, el) {
    return (el || document).querySelector(sel);
  };
  var $$ = function (sel, el) {
    return Array.prototype.slice.call((el || document).querySelectorAll(sel));
  };
  var reduceMotion =
    window.matchMedia &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ---------- ids for every sub-heading (search + "on this page") -------- */
  function slug(s) {
    return s
      .toLowerCase()
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "")
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48);
  }
  var docs = $$("section.doc");
  docs.forEach(function (sec) {
    $$("h3, h4", sec).forEach(function (h) {
      if (!h.id) {
        var base = sec.id + "-" + slug(h.textContent);
        var id = base;
        var n = 2;
        while (document.getElementById(id)) id = base + "-" + n++;
        h.id = id;
      }
    });
  });

  /* ---------- drawer ---------- */
  var body = document.body;
  var sidebar = $("#sidebar");
  var menuBtn = $("#menu-btn");
  var closeBtn = $("#nav-close");
  var backdrop = $("#backdrop");
  var desktop = window.matchMedia("(min-width: 1024px)");
  function openNav() {
    body.classList.add("nav-open");
    menuBtn.setAttribute("aria-expanded", "true");
    sidebar.setAttribute("aria-modal", "true");
    sidebar.setAttribute("role", "dialog");
    var cur = $('a[aria-current="true"]', sidebar) || $("a", sidebar);
    setTimeout(function () {
      (closeBtn || cur).focus();
      if (cur && cur.scrollIntoView) cur.scrollIntoView({ block: "center" });
    }, 30);
  }
  function closeNav(focusBack) {
    if (!body.classList.contains("nav-open")) return;
    body.classList.remove("nav-open");
    menuBtn.setAttribute("aria-expanded", "false");
    sidebar.removeAttribute("aria-modal");
    sidebar.removeAttribute("role");
    if (focusBack) menuBtn.focus();
  }
  if (menuBtn) {
    menuBtn.addEventListener("click", function () {
      body.classList.contains("nav-open") ? closeNav(true) : openNav();
    });
  }
  if (closeBtn)
    closeBtn.addEventListener("click", function () {
      closeNav(true);
    });
  if (backdrop)
    backdrop.addEventListener("click", function () {
      closeNav(true);
    });
  sidebar.addEventListener("click", function (e) {
    if (e.target.closest("a") && !desktop.matches) closeNav(false);
  });
  sidebar.addEventListener("keydown", function (e) {
    if (e.key === "Escape") closeNav(true);
    if (e.key === "Tab" && body.classList.contains("nav-open")) {
      var f = $$("a, button", sidebar);
      var first = f[0];
      var last = f[f.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  });
  var onDesk = function () {
    if (desktop.matches) closeNav(false);
  };
  if (desktop.addEventListener) desktop.addEventListener("change", onDesk);

  /* ---------- anchors: open closed <details>, flash the target ---------- */
  function reveal(el) {
    var d = el.closest("details");
    while (d) {
      d.open = true;
      d = d.parentElement && d.parentElement.closest("details");
    }
    if (el.tagName === "DETAILS") el.open = true;
  }
  function flash(el) {
    var t = el;
    if (t.tagName === "SECTION" && t.classList.contains("doc")) {
      t = t;
    } else if (t.tagName === "DETAILS") {
      t = t.querySelector("summary") || t;
    }
    t.classList.remove("flash");
    void t.offsetWidth;
    t.classList.add("flash");
    setTimeout(function () {
      t.classList.remove("flash");
    }, 2400);
  }
  function goToHash(doFlash) {
    var id = decodeURIComponent(location.hash.slice(1));
    if (!id) return;
    var el = document.getElementById(id);
    if (!el) return;
    reveal(el);
    // Short hops glide, long jumps (often 20 000 px on a phone) land at once.
    var dist = Math.abs(el.getBoundingClientRect().top);
    el.scrollIntoView({
      block: "start",
      behavior: reduceMotion || dist > window.innerHeight * 2 ? "auto" : "smooth",
    });
    if (doFlash) flash(el);
    spy();
  }
  window.addEventListener("hashchange", function () {
    goToHash(true);
  });
  if (location.hash) {
    if ("scrollRestoration" in history) history.scrollRestoration = "manual";
    // After layout settles (fonts, images), so the jump lands exactly.
    var initial = function () {
      var id = decodeURIComponent(location.hash.slice(1));
      var el = id && document.getElementById(id);
      if (!el) return;
      reveal(el);
      el.scrollIntoView({ block: "start" });
      flash(el);
      spy();
    };
    if (document.readyState === "complete") setTimeout(initial, 0);
    else window.addEventListener("load", initial);
  }
  function jump(id) {
    if (location.hash === "#" + id) goToHash(true);
    else location.hash = id;
  }

  /* ---------- scroll spy: sidebar highlight + "on this page" ---------- */
  var navLinks = {};
  $$('a[href^="#"]', sidebar).forEach(function (a) {
    navLinks[a.getAttribute("href").slice(1)] = a;
  });
  var tocBox = $("#toc");
  var currentDoc = null;
  function buildToc(sec) {
    if (!tocBox) return;
    var hs = $$("h3", sec);
    var title = $(".doc-head h2", sec);
    var html =
      '<h2>On this page</h2><p class="toc-title">' +
      esc(title ? title.textContent : "") +
      "</p>";
    if (hs.length) {
      html += "<ul>";
      hs.forEach(function (h) {
        html +=
          '<li><a href="#' + h.id + '" data-id="' + h.id + '">' + esc(h.textContent) + "</a></li>";
      });
      html += "</ul>";
    }
    tocBox.innerHTML = html;
  }
  function setCurrent(sec) {
    if (sec === currentDoc) return;
    currentDoc = sec;
    Object.keys(navLinks).forEach(function (k) {
      navLinks[k].removeAttribute("aria-current");
    });
    var link = navLinks[sec.id];
    if (link) link.setAttribute("aria-current", "true");
    buildToc(sec);
  }
  function spy() {
    var line = window.innerHeight * 0.3;
    var found = docs[0];
    for (var i = 0; i < docs.length; i++) {
      if (docs[i].getBoundingClientRect().top - line <= 0) found = docs[i];
      else break;
    }
    if (found) setCurrent(found);
    if (tocBox && currentDoc) {
      var active = null;
      $$("h3", currentDoc).forEach(function (h) {
        if (h.getBoundingClientRect().top - line <= 0) active = h.id;
      });
      $$("a", tocBox).forEach(function (a) {
        a.classList.toggle("active", a.getAttribute("data-id") === active);
      });
    }
  }
  var spyQueued = false;
  window.addEventListener(
    "scroll",
    function () {
      if (spyQueued) return;
      spyQueued = true;
      requestAnimationFrame(function () {
        spyQueued = false;
        spy();
      });
    },
    { passive: true },
  );
  spy();

  /* ---------- pinout legend filter ---------- */
  var pw = $("#pinout-wrap");
  $$(".pin-legend button").forEach(function (b) {
    b.addEventListener("click", function () {
      var on = b.getAttribute("aria-pressed") === "true";
      $$(".pin-legend button").forEach(function (x) {
        x.setAttribute("aria-pressed", "false");
      });
      if (on) pw.removeAttribute("data-show");
      else {
        b.setAttribute("aria-pressed", "true");
        pw.setAttribute("data-show", b.getAttribute("data-show"));
      }
    });
  });

  /* ---------- copy buttons on command boxes ---------- */
  if (navigator.clipboard && window.isSecureContext) {
    $$("pre").forEach(function (pre) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "copy-btn";
      b.textContent = "Copy";
      b.addEventListener("click", function () {
        var code = pre.querySelector("code") || pre;
        navigator.clipboard.writeText(code.textContent.trim()).then(
          function () {
            b.textContent = "Copied";
            setTimeout(function () {
              b.textContent = "Copy";
            }, 1500);
          },
          function () {},
        );
      });
      pre.appendChild(b);
    });
  }

  /* ---------- search ---------- */
  function esc(s) {
    return s.replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }
  function norm(s) {
    return s
      .toLowerCase()
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "");
  }
  // "Wi-Fi" -> wi, fi, wifi; "192.168.50.1" -> parts + joined.
  function tokens(s) {
    var out = [];
    var m = norm(s).match(/[a-z0-9]+(?:[-'.’/][a-z0-9]+)*/g) || [];
    m.forEach(function (t) {
      if (/[-'.’/]/.test(t)) {
        t.split(/[-'.’/]/).forEach(function (p) {
          if (p) out.push(p);
        });
        out.push(t.replace(/[-'.’/]/g, ""));
      } else out.push(t);
    });
    return out;
  }
  function queryTokens(s) {
    var m = norm(s).match(/[a-z0-9]+(?:[-'.’/][a-z0-9]+)*/g) || [];
    return m
      .map(function (t) {
        return t.replace(/[-'.’/]/g, "");
      })
      .filter(function (t) {
        return t.length > 0 && STOP.indexOf(t) < 0;
      });
  }
  var STOP = ["the", "a", "an", "of", "to", "and", "or", "is", "in", "on", "how", "do", "i", "my", "what", "with", "for", "can"];

  var entries = [];
  function textOf(node) {
    if (node.nodeType === 3) return node.nodeValue;
    if (node.nodeType !== 1) return "";
    if (node.hasAttribute("data-noindex") || node.tagName === "SCRIPT" || node.tagName === "BUTTON") return "";
    if (node.tagName === "svg" || node.tagName === "SVG") return "";
    var t = "";
    for (var c = node.firstChild; c; c = c.nextSibling) t += textOf(c);
    if (/^(P|LI|DT|DD|TD|TH|TR|H3|H4|PRE|DIV|SUMMARY|FIGCAPTION|BR)$/.test(node.tagName)) t = " " + t + " ";
    return t;
  }
  function addEntry(sec, head, anchor, text, kw) {
    var docTitle = $(".doc-head h2", sec).textContent.trim();
    var group = sec.getAttribute("data-group") || "";
    entries.push({
      id: anchor,
      doc: docTitle,
      group: group,
      head: head,
      text: text.replace(/\s+/g, " ").trim(),
      kw: kw || "",
    });
  }
  function buildIndex() {
    docs.forEach(function (sec) {
      var docKw = sec.getAttribute("data-keywords") || "";
      var head = null;
      var anchor = sec.id;
      var buf = "";
      var kw = docKw;
      var flush = function () {
        if (buf.trim() || head) addEntry(sec, head, anchor, buf, kw);
        buf = "";
      };
      var walk = function (parent) {
        for (var el = parent.firstElementChild; el; el = el.nextElementSibling) {
          if (el.classList.contains("doc-head")) continue;
          if (el.hasAttribute("data-noindex")) continue;
          if (/^H[34]$/.test(el.tagName)) {
            flush();
            head = el.textContent.trim();
            anchor = el.id;
            kw = el.getAttribute("data-keywords") || "";
            continue;
          }
          if (el.tagName === "DETAILS" && el.id) {
            flush();
            var sum = el.querySelector("summary");
            addEntry(
              sec,
              sum ? sum.textContent.trim() : "",
              el.id,
              textOf(el).replace(sum ? sum.textContent : "", ""),
              el.getAttribute("data-keywords") || "",
            );
            continue;
          }
          if (el.hasAttribute("data-chunk")) {
            walk(el);
            continue;
          }
          if (el.id && el.tagName !== "SECTION" && el.hasAttribute("data-anchor")) {
            flush();
            head = el.getAttribute("data-anchor");
            anchor = el.id;
            kw = el.getAttribute("data-keywords") || "";
          }
          buf += " " + textOf(el);
        }
      };
      walk(sec);
      flush();
    });
    entries.forEach(function (e) {
      e.tDoc = tokens(e.doc);
      e.tHead = tokens(e.head || "");
      e.tKw = tokens(e.kw);
      e.tText = tokens(e.text);
      e.vocab = {};
      var add = function (list, w) {
        list.forEach(function (t) {
          e.vocab[t] = Math.max(e.vocab[t] || 0, w);
        });
      };
      add(e.tText, 1);
      // A chapter's own keywords count most on its introduction.
      add(e.tKw, e.head ? 3 : 3.5);
      add(e.tDoc, e.head ? 2 : 6);
      add(e.tHead, 4);
      e.freq = {};
      e.tText.forEach(function (t) {
        e.freq[t] = (e.freq[t] || 0) + 1;
      });
    });
  }

  function lev(a, b, max) {
    if (Math.abs(a.length - b.length) > max) return max + 1;
    // Optimal string alignment: a swapped pair ("pinuot") costs 1.
    var prev2 = null;
    var prev = [];
    var i, j;
    for (j = 0; j <= b.length; j++) prev[j] = j;
    for (i = 1; i <= a.length; i++) {
      var cur = [i];
      var best = i;
      for (j = 1; j <= b.length; j++) {
        var cost = a[i - 1] === b[j - 1] ? 0 : 1;
        cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + cost);
        if (prev2 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1])
          cur[j] = Math.min(cur[j], prev2[j - 2] + 1);
        if (cur[j] < best) best = cur[j];
      }
      if (best > max) return max + 1;
      prev2 = prev;
      prev = cur;
    }
    return prev[b.length];
  }
  // How well does query term q match an indexed word w (0 = not at all)?
  function termMatch(q, w) {
    if (w === q) return 1;
    if (q.length >= 2 && w.indexOf(q) === 0) return 0.75;
    if (q.length >= 4 && w.indexOf(q) > 0) return 0.4;
    if (q.length >= 4 && w.length >= 3) {
      var max = q.length >= 7 ? 2 : 1;
      if (lev(q, w, max) <= max) return 0.5;
      // typo inside a prefix: "pasword" vs "passwords"
      if (w.length > q.length && lev(q, w.slice(0, q.length + 1), max) <= max) return 0.4;
    }
    return 0;
  }
  var matchCache = {};
  function search(q) {
    var qs = queryTokens(q);
    if (!qs.length) return { results: [], terms: [] };
    var allTerms = [];
    var scored = entries.map(function (e) {
      var total = 0;
      var hits = 0;
      var terms = [];
      qs.forEach(function (qt) {
        var best = 0;
        var bestWord = null;
        var key = qt;
        var words = Object.keys(e.vocab);
        for (var i = 0; i < words.length; i++) {
          var w = words[i];
          var ck = key + "|" + w;
          var m = matchCache[ck];
          if (m === undefined) m = matchCache[ck] = termMatch(qt, w);
          if (!m) continue;
          var s = m * e.vocab[w] + Math.min(e.freq[w] || 0, 4) * 0.08 * m;
          if (s > best) {
            best = s;
            bestWord = w;
          }
          if (m >= 0.4) terms.push(w);
        }
        if (best > 0) {
          hits++;
          total += best;
        }
      });
      if (!e.head && hits) total += 0.3;
      return { e: e, score: total, hits: hits, terms: terms };
    });
    var need = qs.length;
    var res = scored.filter(function (r) {
      return r.hits === need;
    });
    if (!res.length && need > 1)
      res = scored.filter(function (r) {
        return r.hits >= Math.max(1, need - 1);
      });
    res.sort(function (a, b) {
      return b.score - a.score;
    });
    // One result per anchor, at most 12.
    var seen = {};
    var out = [];
    for (var i = 0; i < res.length && out.length < 12; i++) {
      if (seen[res[i].e.id]) continue;
      seen[res[i].e.id] = 1;
      out.push(res[i]);
      res[i].terms.forEach(function (t) {
        if (allTerms.indexOf(t) < 0) allTerms.push(t);
      });
    }
    return { results: out, terms: allTerms };
  }
  function termRegex(terms) {
    var parts = terms
      .filter(function (t) {
        return t.length > 1;
      })
      .sort(function (a, b) {
        return b.length - a.length;
      })
      .map(function (t) {
        return t
          .split("")
          .map(function (c) {
            return c.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
          })
          .join("[-.'’/]?");
      });
    if (!parts.length) return null;
    return new RegExp("(" + parts.join("|") + ")", "gi");
  }
  function highlight(s, re) {
    if (!re) return esc(s);
    var out = "";
    var last = 0;
    s.replace(re, function (m, _g, off) {
      out += esc(s.slice(last, off)) + "<mark>" + esc(m) + "</mark>";
      last = off + m.length;
      return m;
    });
    return out + esc(s.slice(last));
  }
  function snippet(text, re) {
    var start = 0;
    if (re) {
      re.lastIndex = 0;
      var m = re.exec(text);
      re.lastIndex = 0;
      if (m) start = Math.max(0, m.index - 50);
    }
    var s = text.slice(start, start + 170);
    if (start > 0) s = "…" + s.replace(/^\S*\s/, "");
    if (start + 170 < text.length) s = s.replace(/\s\S*$/, "") + "…";
    return s;
  }

  var dlg = $("#search");
  var input = $("#search-input");
  var list = $("#search-results");
  var empty = $("#search-empty");
  var popular = $("#search-popular");
  var status = $("#search-status");
  var sel = -1;
  var built = false;
  var lastFocus = null;

  function render() {
    var q = input.value;
    if (!q.trim()) {
      list.innerHTML = "";
      list.hidden = true;
      empty.hidden = true;
      popular.hidden = false;
      status.textContent = "";
      sel = -1;
      input.removeAttribute("aria-activedescendant");
      return;
    }
    var r = search(q);
    popular.hidden = true;
    if (!r.results.length) {
      list.hidden = true;
      list.innerHTML = "";
      empty.hidden = false;
      $("#search-empty-q").textContent = q.trim();
      status.textContent = "No results";
      sel = -1;
      return;
    }
    empty.hidden = true;
    list.hidden = false;
    var re = termRegex(r.terms);
    list.innerHTML = r.results
      .map(function (x, i) {
        var e = x.e;
        var title = e.head || e.doc;
        var path = e.group + (e.head ? " › " + e.doc : "");
        return (
          '<li role="presentation"><a role="option" id="sr-' + i + '" href="#' + e.id + '" data-id="' + e.id + '" aria-selected="false">' +
          '<span class="r-path">' + esc(path) + "</span>" +
          '<span class="r-title">' + highlight(title, re) + "</span>" +
          '<span class="r-snip">' + highlight(snippet(e.text, re), re) + "</span></a></li>"
        );
      })
      .join("");
    status.textContent = r.results.length + (r.results.length === 1 ? " result" : " results");
    select(0);
  }
  function select(i) {
    var opts = $$("a[role=option]", list);
    if (!opts.length) return;
    sel = (i + opts.length) % opts.length;
    opts.forEach(function (o, j) {
      o.setAttribute("aria-selected", j === sel ? "true" : "false");
    });
    input.setAttribute("aria-activedescendant", opts[sel].id);
    var o = opts[sel];
    var top = o.offsetTop;
    var box = list.parentElement;
    if (top < box.scrollTop) box.scrollTop = top - 8;
    else if (top + o.offsetHeight > box.scrollTop + box.clientHeight)
      box.scrollTop = top + o.offsetHeight - box.clientHeight + 8;
  }
  function openSearch(prefill) {
    if (!built) {
      buildIndex();
      built = true;
    }
    closeNav(false);
    lastFocus = document.activeElement;
    if (typeof dlg.showModal === "function") {
      if (!dlg.open) dlg.showModal();
    } else dlg.setAttribute("open", "");
    if (typeof prefill === "string") input.value = prefill;
    render();
    input.focus();
    input.select();
  }
  function closeSearch() {
    if (dlg.open) {
      if (typeof dlg.close === "function") dlg.close();
      else dlg.removeAttribute("open");
    }
  }
  dlg.addEventListener("close", function () {
    if (lastFocus && lastFocus.focus && !pendingJump) lastFocus.focus();
  });
  var pendingJump = null;
  function choose(id) {
    pendingJump = id;
    closeSearch();
    setTimeout(function () {
      jump(id);
      var el = document.getElementById(id);
      if (el) {
        if (!el.hasAttribute("tabindex") && !/^(A|BUTTON|INPUT|SUMMARY)$/.test(el.tagName))
          el.setAttribute("tabindex", "-1");
        el.focus({ preventScroll: true });
      }
      pendingJump = null;
    }, 10);
  }
  input.addEventListener("input", render);
  input.addEventListener("keydown", function (e) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      select(sel + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      select(sel - 1);
    } else if (e.key === "Enter") {
      var o = $$("a[role=option]", list)[sel];
      if (o) {
        e.preventDefault();
        choose(o.getAttribute("data-id"));
      }
    }
  });
  dlg.addEventListener("click", function (e) {
    if (e.target === dlg) {
      closeSearch();
      return;
    }
    var a = e.target.closest("a[href^='#']");
    if (a) {
      e.preventDefault();
      choose(a.getAttribute("href").slice(1));
    }
  });
  $("#search-close").addEventListener("click", closeSearch);
  $$("[data-open-search]").forEach(function (b) {
    b.addEventListener("click", function () {
      openSearch();
    });
  });
  document.addEventListener("keydown", function (e) {
    var t = e.target;
    var typing = t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
    if ((e.key === "k" || e.key === "K") && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      dlg.open ? input.focus() : openSearch();
    } else if (e.key === "/" && !typing && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      openSearch();
    } else if (e.key === "Escape" && body.classList.contains("nav-open")) {
      closeNav(true);
    }
  });
  // ?q=... opens the search prefilled (e.g. a bookmark).
  var qm = /[?&]q=([^&#]*)/.exec(location.search);
  if (qm) openSearch(decodeURIComponent(qm[1].replace(/\+/g, " ")));

  // Exposed for testing in the browser console only.
  window.ugSearch = function (q) {
    if (!built) {
      buildIndex();
      built = true;
    }
    return search(q).results.map(function (r) {
      return r.e.id + " (" + r.score.toFixed(2) + ")";
    });
  };
})();
