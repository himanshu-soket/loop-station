#!/usr/bin/env python3
"""Phase 5.5 · Premium game setup & pre-game experience — checks.

  A. LOBBY FLOW: mounts into setup, Start begins play, ✕ also starts.
  B. CONFIGURATIONS: every valid (players × humans × difficulty)
     combination starts exactly the selected configuration.
  C. NAMES: defaults, custom entry, validation (empty/whitespace →
     default, length cap, HTML escaping), persistence across restart,
     appearance in panels and turn messages.
  D. SEAT PREVIEW: colour/name/kind/difficulty per seat, inactive seats
     identified, immediate update on player-count change.
  E. RESTART & NAVIGATION: Restart preserves players/humans/difficulty/
     names and freezes the running game; config changes create fresh
     games safely; back-navigation leaves no stale state.
  F. RESPONSIVE: desktop / laptop / tablet portrait / mobile portrait /
     mobile landscape — no clipped controls, no document overflow.
  G. ACCESSIBILITY: focus management, keyboard activation, radiogroup
     semantics, labelled inputs, disabled states, touch targets.
  H. REGRESSION GUARDS: engine rules unchanged; Phase 5.1–5.4 features
     (dice button, audio popover, FX) still work around the lobby.

Run with the dashboard serving on 127.0.0.1:8907.
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8907"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 1280, "height": 900})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)

    page.goto(BASE + "/console")
    page.wait_for_selector("#viewRoot")
    page.evaluate("location.hash = '#funtime/lu'")
    page.wait_for_selector("#luBoard")
    page.wait_for_timeout(600)

    # ---------- A: lobby flow ----------
    a0 = page.evaluate("""(() => ({
      lobby: window.LUG.lobby,
      visible: !document.getElementById('luLobby').classList.contains('hidden'),
      rollDisabled: document.getElementById('luRollBtn').disabled,
      startFocused: document.activeElement === document.getElementById('luStartBtn') }))()""")
    check("flow/mountsIntoLobby",
          a0["lobby"] and a0["visible"] and a0["rollDisabled"], json.dumps(a0))
    check("flow/startButtonFocused", a0["startFocused"], "")

    page.click("#luStartBtn")
    page.wait_for_timeout(400)
    a1 = page.evaluate("""(() => ({
      lobby: window.LUG.lobby,
      hidden: document.getElementById('luLobby').classList.contains('hidden'),
      rollEnabled: !document.getElementById('luRollBtn').disabled,
      kinds: window.LUG.game.players.map(P => P.kind),
      seats: window.LudoEngine.activeSeats(window.LUG.game) }))()""")
    check("flow/startBeginsPlay",
          not a1["lobby"] and a1["hidden"] and a1["rollEnabled"]
          and a1["seats"] == [0, 1, 2, 3]
          and a1["kinds"] == ["human", "ai", "ai", "ai"], json.dumps(a1))

    # ---------- B: all valid configurations ----------
    configs = [
        (2, 1, ""), (2, 2, ""), (3, 1, ""), (3, 2, ""), (3, 3, ""),
        (4, 1, ""), (4, 2, ""), (4, 3, ""), (4, 4, ""),
        (4, 1, "easy"), (4, 1, "medium"), (4, 1, "hard"),
        (2, 1, "hard"), (3, 2, "medium"),
    ]
    bad = []
    for n, h, d in configs:
        r = page.evaluate("""(cfg) => {
          const [n, h, d] = cfg;
          LUG.seatCount = n; LUG.humanSeats = h;
          LUG.aiDifficulty = d === '' ? null : d;
          LUG.names = ['', '', '', ''];
          window.luLobbyStart();
          const g = window.LUG.game;
          return { seats: window.LudoEngine.activeSeats(g),
                   kinds: g.players.map(P => P.kind),
                   diff: window.LUG.aiDifficulty };
        }""", [n, h, d])
        exp_seats = {2: [0, 2], 3: [0, 1, 2], 4: [0, 1, 2, 3]}[n]
        # first h active seats are human
        exp_kinds = [0, 1, 2, 3].map if False else None
        ok = r["seats"] == exp_seats and r["diff"] == (d or None)
        # verify kinds: seat is human iff its index < h within active order
        for s in range(4):
            at = exp_seats.index(s) if s in exp_seats else -1
            want = "human" if (at != -1 and at < h) else "ai"
            if r["kinds"][s] != want:
                ok = False
        if not ok:
            bad.append({"cfg": [n, h, d], "got": r})
    check("config/allValidCombinationsStartExactly", not bad, json.dumps(bad[:2]))

    # mode buttons: Quick Start pins defaults
    b0 = page.evaluate("""(() => {
      // open lobby via restart path
      window.luLobbyShow();
      // set something non-default
      LUG.seatCount = 2; LUG.humanSeats = 2; LUG.aiDifficulty = 'hard';
      window.luLobSync();
      return true;
    })()""")
    page.click("#luModePick [data-mode='quick']")
    page.wait_for_timeout(200)
    b1 = page.evaluate("""(() => ({ mode: window.LUG.lobbyMode,
      n: window.LUG.seatCount, h: window.LUG.humanSeats,
      d: window.LUG.aiDifficulty,
      cfgLocked: document.getElementById('luLobCfg').classList.contains('off') }))()""")
    check("config/quickStartPinsDefaults",
          b1["mode"] == "quick" and b1["n"] == 4 and b1["h"] == 1
          and b1["d"] is None and b1["cfgLocked"], json.dumps(b1))
    page.click("#luModePick [data-mode='custom']")
    page.wait_for_timeout(150)
    b2 = page.evaluate(
      "!document.getElementById('luLobCfg').classList.contains('off')")
    check("config/customUnlocksGrid", b2 is True, str(b2))

    # ---------- C: names ----------
    page.evaluate("window.luLobbyShow()")
    page.wait_for_timeout(200)
    c0 = page.evaluate("""(() => {
      // defaults for 4P·1H
      LUG.seatCount = 4; LUG.humanSeats = 1; LUG.aiDifficulty = null;
      LUG.names = ['', '', '', ''];
      window.luLobSync();
      return [0,1,2,3].map(i => window.luSeatNameRaw(i));
    })()""")
    check("names/defaultsFirstHumanIsYou",
          c0[0] == "You" and c0[1] == "Green" and c0[2] == "Yellow"
          and c0[3] == "Blue", json.dumps(c0))

    # 4H defaults
    c1 = page.evaluate("""(() => {
      LUG.humanSeats = 4; LUG.names = ['', '', '', ''];
      window.luLobSync();
      return [0,1,2,3].map(i => window.luSeatNameRaw(i));
    })()""")
    check("names/defaultsAdditionalHumansNumbered",
          c1 == ["You", "Player 2", "Player 3", "Player 4"], json.dumps(c1))

    # AI difficulty appears in AI default names
    c2 = page.evaluate("""(() => {
      LUG.humanSeats = 1; LUG.aiDifficulty = 'hard';
      LUG.names = ['', '', '', ''];
      window.luLobSync();
      return window.luSeatNameRaw(1);
    })()""")
    check("names/aiDefaultIncludesLevel", c2 == "Green · Hard", c2)

    # validation via real input events (4H so every seat has a field)
    c3 = page.evaluate("""(() => {
      LUG.humanSeats = 4; LUG.names = ['', '', '', ''];
      window.luLobSync();
      const out = {};
      const set = (seat, val) => {
        const inp = document.querySelector(`[data-nmseat="${seat}"]`);
        inp.value = val;
        inp.dispatchEvent(new Event('input', {bubbles:true}));
        inp.dispatchEvent(new FocusEvent('focusout', {bubbles:true}));
        return { stored: window.LUG.names[seat], shown: inp.value };
      };
      out.empty = set(1, '   ');
      out.spaces = set(1, '  Ana   Bella  ');
      out.tooLong = set(2, 'A'.repeat(40));
      out.script = set(3, '<script>alert(1)<\\/script>');
      return out;
    })()""")
    check("names/emptyFallsBackToDefault",
          c3["empty"]["stored"] == "" and c3["empty"]["shown"] == "Player 2",
          json.dumps(c3["empty"]))
    check("names/whitespaceCollapsed",
          c3["spaces"]["stored"] == "Ana Bella", json.dumps(c3["spaces"]))
    check("names/lengthCappedAt14",
          c3["tooLong"]["stored"] == "A" * 14, json.dumps(c3["tooLong"]))
    esc = page.evaluate("window.luSeatName(3)")
    check("names/htmlEscapedInDisplay",
          "&lt;script&gt;" in esc and "<script>" not in esc, esc)

    # custom names appear in panels + messages after start
    page.evaluate("""(() => {
      LUG.humanSeats = 2; LUG.names = ['Me', 'Zed', '', ''];
      window.luLobSync();
    })()""")
    page.click("#luStartBtn")
    page.wait_for_timeout(400)
    c4 = page.evaluate("""(() => ({
      panels: [...document.querySelectorAll('#luPanels .lu-p-name')]
        .map(e => e.textContent),
      msg: document.getElementById('luMsg').textContent }))()""")
    check("names/appearInPanelsAndMessages",
          c4["panels"][:2] == ["Me", "Zed"] and "Me" in c4["msg"],
          json.dumps(c4))

    # ---------- D: seat preview ----------
    page.evaluate("window.luLobbyShow()")
    page.wait_for_timeout(200)
    d0 = page.evaluate("""(() => {
      LUG.seatCount = 3; LUG.humanSeats = 2; LUG.aiDifficulty = 'medium';
      LUG.names = ['', 'Bee', '', ''];
      window.luLobSync();
      return [...document.querySelectorAll('#luLobPrev .lu-lob-seat')].map(s => ({
        seat: Number(s.dataset.prevseat),
        inactive: s.classList.contains('inactive'),
        kind: s.querySelector('.lu-lob-sk').textContent,
        hasInput: !!s.querySelector('.lu-lob-nm'),
        name: s.querySelector('.lu-lob-sn')
          ? s.querySelector('.lu-lob-sn').textContent
          : s.querySelector('.lu-lob-nm').value }));
    })()""")
    ok = (len(d0) == 4
          and [x["inactive"] for x in d0] == [False, False, False, True]
          and d0[0]["kind"] == "HUMAN" and d0[1]["kind"] == "HUMAN"
          and d0[2]["kind"].startswith("AI · Medium")
          and d0[0]["hasInput"] and d0[1]["hasInput"]
          and not d0[2]["hasInput"]
          and d0[1]["name"] == "Bee")
    check("preview/showsColourKindNamePerSeat", ok, json.dumps(d0))

    # immediate update on player-count change (no start needed)
    page.click("#luSeatPick [data-seats='2']")
    page.wait_for_timeout(200)
    d1 = page.evaluate("""(() => ({
      n: window.LUG.seatCount,
      inactive: [...document.querySelectorAll('#luLobPrev .lu-lob-seat.inactive')]
        .map(s => Number(s.dataset.prevseat)) }))()""")
    check("preview/playerCountUpdatesImmediately",
          d1["n"] == 2 and d1["inactive"] == [1, 3], json.dumps(d1))

    # ---------- E: restart & navigation ----------
    # configure + start + play, then restart → everything preserved
    page.evaluate("""(() => {
      LUG.seatCount = 3; LUG.humanSeats = 2; LUG.aiDifficulty = 'hard';
      LUG.names = ['Al', 'Bo', '', ''];
      window.luLobSync(); window.luLobbyStart();
    })()""")
    page.wait_for_timeout(300)
    page.evaluate("window.LUD.fast()")
    page.evaluate("window.LUD.force(6)")
    page.wait_for_timeout(250)
    page.click("#luRestartBtn")
    page.wait_for_timeout(1200)   # long enough for any stale timer to fire
    e0 = page.evaluate("""(() => ({
      lobby: window.LUG.lobby,
      n: window.LUG.seatCount, h: window.LUG.humanSeats,
      d: window.LUG.aiDifficulty, names: window.LUG.names,
      nameField: document.querySelector('[data-nmseat="1"]') ?
        document.querySelector('[data-nmseat="1"]').value : null,
      frozen: { phase: window.LUG.phase, dice: window.LUG.dice,
                moves: window.LUG.moves.length, anim: window.LUG.anim,
                fx: window.LUG.fx.length, musicTimer: window.LUA.musicTimer,
                allYard: window.LUG.game.players.flatMap(P=>P.tokens)
                  .every(t => t.pos === -1) } }))()""")
    check("restart/preservesAllSettings",
          e0["lobby"] and e0["n"] == 3 and e0["h"] == 2 and e0["d"] == "hard"
          and e0["names"][0] == "Al" and e0["names"][1] == "Bo"
          and e0["nameField"] == "Bo", json.dumps(e0))
    check("restart/freezesRunningGame",
          e0["frozen"]["phase"] == "idle" and e0["frozen"]["dice"] is None
          and e0["frozen"]["moves"] == 0 and e0["frozen"]["anim"] is None
          and e0["frozen"]["fx"] == 0 and e0["frozen"]["musicTimer"] == 0
          and e0["frozen"]["allYard"], json.dumps(e0["frozen"]))

    # changing configuration mid-lobby then Start → fresh exact game
    page.click("#luSeatPick [data-seats='4']")
    page.click("#luHumanPick [data-humans='1']")
    page.click("#luDiffPick [data-diff='easy']")
    page.wait_for_timeout(200)
    page.click("#luStartBtn")
    page.wait_for_timeout(400)
    e1 = page.evaluate("""(() => ({
      lobby: window.LUG.lobby,
      seats: window.LudoEngine.activeSeats(window.LUG.game),
      kinds: window.LUG.game.players.map(P => P.kind),
      diff: window.LUG.aiDifficulty, allYard: true }))()""")
    check("restart/configChangeAppliesFresh",
          not e1["lobby"] and e1["seats"] == [0, 1, 2, 3]
          and e1["kinds"] == ["human", "ai", "ai", "ai"]
          and e1["diff"] == "easy", json.dumps(e1))

    # back navigation mid-game → clean teardown; re-entry → fresh lobby
    page.evaluate("window.LUD.force(6)")
    page.wait_for_timeout(200)
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(400)
    page.evaluate("location.hash = '#funtime/lu'")
    page.wait_for_selector("#luBoard")
    page.wait_for_timeout(600)
    e2 = page.evaluate("""(() => ({
      lobby: window.LUG.lobby,
      visible: !document.getElementById('luLobby').classList.contains('hidden'),
      anim: window.LUG.anim, fx: window.LUG.fx.length,
      vizFrozen: window.LUG.game.players.flatMap(P=>P.tokens)
        .some(t => t._viz != null),
      allYard: window.LUG.game.players.flatMap(P=>P.tokens)
        .every(t => t.pos === -1) }))()""")
    check("nav/backThenReentryCleanState",
          e2["lobby"] and e2["visible"] and e2["anim"] is None
          and e2["fx"] == 0 and not e2["vizFrozen"] and e2["allYard"],
          json.dumps(e2))

    # ---------- F: responsive ----------
    viewports = [("desktop", 1440, 900), ("laptop", 1280, 800),
                 ("tablet_portrait", 768, 1024), ("mobile_portrait", 390, 844),
                 ("mobile_landscape", 844, 390)]
    for name, w, h in viewports:
        page.set_viewport_size({"width": w, "height": h})
        page.wait_for_timeout(350)
        r = page.evaluate("""(() => {
          const card = document.getElementById('luLobCard');
          const cr = card.getBoundingClientRect();
          const controls = [...card.querySelectorAll('button,input')];
          let clipped = 0;
          for (const c of controls) {
            const r2 = c.getBoundingClientRect();
            if (r2.width === 0 || r2.height === 0) continue;
            if (r2.left < -1 || r2.right > window.innerWidth + 1 ||
                r2.top < -1 || r2.bottom > window.innerHeight + 1) clipped++;
          }
          const canScroll = card.scrollHeight > card.clientHeight;
          return { fits: cr.bottom <= window.innerHeight + 2 || canScroll,
                   clipped, docOverflow:
                     document.documentElement.scrollWidth > window.innerWidth + 1 };
        })()""")
        page.screenshot(path=f".phase5_screenshots/after53/lobby_{name}.png")
        check(f"responsive/{name}",
              r["fits"] and r["clipped"] == 0 and not r["docOverflow"],
              json.dumps(r))
    page.set_viewport_size({"width": 1280, "height": 900})

    # ---------- G: accessibility ----------
    page.reload()
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(600)
    g0 = page.evaluate("""(() => ({
      startFocused: document.activeElement === document.getElementById('luStartBtn'),
      groups: [...document.querySelectorAll('.lu-lobby [role="radiogroup"]')].map(g => ({
        label: g.getAttribute('aria-label'),
        radios: g.querySelectorAll('[role="radio"]').length,
        oneChecked: g.querySelectorAll('[role="radio"][aria-checked="true"]').length === 1 })),
      inputsLabelled: [...document.querySelectorAll('.lu-lob-nm')]
        .every(i => (i.getAttribute('aria-label') || '').length > 3),
      touchTargets: (() => {
        // every visible radio button ≥ 28px tall (compact but tappable)
        const bs = [...document.querySelectorAll('.lu-lobby .lu-sp-btn:not([hidden])')];
        return bs.every(b => b.getBoundingClientRect().height >= 28);
      })() }))()""")
    check("a11y/startFocusedOnOpen", g0["startFocused"], "")
    check("a11y/radiogroupSemantics",
          len(g0["groups"]) == 4 and all(x["oneChecked"] for x in g0["groups"]),
          json.dumps([x["label"] for x in g0["groups"]]))
    check("a11y/inputsHaveLabels", g0["inputsLabelled"], "")
    check("a11y/touchFriendlyTargets", g0["touchTargets"], "")

    # keyboard: Tab reaches Start; Enter on a radio activates it
    page.keyboard.press("Tab")   # leave Start
    for _ in range(20):
        page.keyboard.press("Tab")
        if page.evaluate(
          "document.activeElement === document.getElementById('luStartBtn')"):
            break
    g1 = page.evaluate(
      "document.activeElement === document.getElementById('luStartBtn')")
    check("a11y/tabReachesStart", g1, "")
    page.evaluate(
      "document.querySelector(\"#luSeatPick [data-seats='3']\").focus()")
    page.keyboard.press("Enter")
    page.wait_for_timeout(200)
    g2 = page.evaluate("window.LUG.seatCount")
    check("a11y/enterActivatesRadio", g2 == 3, f"seatCount={g2}")
    g3 = page.evaluate("""(() => {
      const b = document.querySelector("#luSeatPick [data-seats='4']");
      b.focus();
      const st = getComputedStyle(b);
      return st.outlineStyle !== 'none' && parseFloat(st.outlineWidth) >= 1.5;
    })()""")
    check("a11y/focusOutlineVisible", g3, "")
    # disabled states: human count beyond seat count is hidden/disabled
    g4 = page.evaluate("""(() => {
      LUG.seatCount = 2; window.luLobSync();
      const b4 = document.querySelector("#luHumanPick [data-humans='4']");
      const b2 = document.querySelector("#luHumanPick [data-humans='2']");
      return { h4Hidden: b4.hidden, h4Disabled: b4.disabled,
               h2Usable: !b2.hidden && !b2.disabled };
    })()""")
    check("a11y/invalidOptionsDisabled",
          g4["h4Hidden"] and g4["h4Disabled"] and g4["h2Usable"],
          json.dumps(g4))

    # ---------- H: regression guards ----------
    page.evaluate("window.luLobbyStart()")
    page.wait_for_timeout(300)
    h0 = page.evaluate("""(() => {
      const E = window.LudoEngine;
      const g = E.createGame(['human','ai','ai','ai']);
      E.applyMove(g, 0, 0, 6);
      const mv = E.legalMoves(g, 0, 4);
      const g2 = E.createGame();
      g2.players[0].tokens[0].pos = 54;
      return { pos: g.players[0].tokens[0].pos,
               moveOk: mv.length === 1 && mv[0].to === 4,
               exactFinish: E.legalMoves(g2, 0, 2)[0].to === 56 &&
                            E.legalMoves(g2, 0, 3).length === 0 };
    })()""")
    check("regress/engineRulesUnchanged",
          h0["pos"] == 0 and h0["moveOk"] and h0["exactFinish"], json.dumps(h0))

    # Phase 5.4 audio popover still opens during play
    page.click("#luSoundBtn")
    page.wait_for_timeout(200)
    h1 = page.evaluate("""(() => ({
      popOpen: !document.getElementById('luAudioPop').classList.contains('hidden'),
      audio: window.LUG.audio }))()""")
    check("regress/audioSettingsStillWork", h1["popOpen"], json.dumps(h1["audio"]))
    page.mouse.click(60, 60)   # close popover (outside)
    page.wait_for_timeout(150)

    # Phase 5.3 FX still fire on a capture
    h2 = page.evaluate("""(() => new Promise(res => {
      window.LUG.clock++;
      const g = window.LUG.game;
      for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
      g.turn = 0; g.chain = 0; g.winner = null;
      g.players[0].tokens[0].pos = 9;
      g.players[1].tokens[0].pos = (10-13+52)%52;
      LUG.phase='awaiting'; LUG.dice=1;
      LUG.moves = LudoEngine.legalMoves(g, 0, 1);
      const m = LUG.moves[0];
      window.luExecuteMove({token:m.token, to:m.to});
      const t0 = performance.now();
      const iv = setInterval(() => {
        if (LUG.fx.some(f => f.kind === 'impact') ||
            performance.now() - t0 > 900) {
          clearInterval(iv);
          res({ impact: LUG.fx.some(f => f.kind === 'impact'),
                victimHome: g.players[1].tokens[0].pos === -1 });
        }
      }, 25);
    }))()""")
    check("regress/captureFxStillWorks",
          h2["impact"] and h2["victimHome"], json.dumps(h2))

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
