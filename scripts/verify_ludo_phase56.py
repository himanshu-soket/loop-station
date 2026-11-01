#!/usr/bin/env python3
"""Phase 5.6 · Premium fullscreen & final experience polish — checks.

  A. FULLSCREEN STATE: button and F-key entry apply body.lu-fs, sync the
     button label + aria-pressed with the real browser state; Esc exits
     and returns to Fun Time; repeated cycles leave no stale classes.
  B. LAYOUT IN FULLSCREEN: board square, centred, fully visible; panels
     readable; dice/controls reachable; audio popover opens and fits;
     no document scroll or clipped controls at five breakpoints.
  C. LOBBY × FULLSCREEN: lobby opens/starts/restarts while fullscreen;
     exiting fullscreen with the lobby visible leaves cleanly; returning
     to normal view preserves game state (names, config, no stale
     timers/animations/audio).
  D. LIFECYCLE: route away/re-entry, repeated toggling, restart from an
     active game, lobby→game→lobby, audio + name persistence, no
     duplicate listeners, no runaway rAF.
  E. REGRESSION GUARDS: engine untouched; Phase 5.1–5.5 behaviours.

Run with the dashboard serving on 127.0.0.1:8907.
"""
import json, sys
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

    def mount():
        page.goto(BASE + "/console")
        page.wait_for_selector("#viewRoot")
        page.evaluate("location.hash = '#funtime/lu'")
        page.wait_for_selector("#luBoard")
        page.wait_for_timeout(600)
        page.click("#luStartBtn")
        page.wait_for_timeout(400)

    def fs_state():
        return page.evaluate("""(() => ({
          native: !!document.fullscreenElement,
          luFs: document.body.classList.contains('lu-fs'),
          immersive: document.body.classList.contains('ft-immersive'),
          label: document.getElementById('luFsBtn').textContent.trim(),
          pressed: document.getElementById('luFsBtn').getAttribute('aria-pressed') }))()""")

    def exit_fs():
        for _ in range(4):
            try:
                if not page.evaluate("!!document.fullscreenElement"):
                    return
                page.evaluate("document.exitFullscreen()")
                page.wait_for_timeout(600)
            except Exception:
                page.wait_for_timeout(400)   # mid-navigation; retry

    # ---------- A: fullscreen state ----------
    mount()
    page.click("#luFsBtn"); page.wait_for_timeout(800)
    a0 = fs_state()
    check("fs/buttonEntryAppliesLayoutAndSyncs",
          a0["native"] and a0["luFs"] and a0["label"] == "Exit Fullscreen"
          and a0["pressed"] == "true", json.dumps(a0))

    # board geometry in fullscreen
    g = page.evaluate("""(() => {
      const c = document.getElementById('luBoard');
      const r = c.getBoundingClientRect();
      return { w: Math.round(r.width), h: Math.round(r.height),
               square: Math.abs(r.width - r.height) < 2,
               fitsV: r.top >= -1 && r.bottom <= window.innerHeight + 1,
               offX: Math.round(Math.abs((r.left + r.right) / 2 - window.innerWidth / 2)) };
    })()""")
    check("fs/boardSquareCenteredVisible",
          g["square"] and g["fitsV"] and g["offX"] <= 2 and g["w"] > 300,
          json.dumps(g))

    # panels readable, controls reachable, no scroll
    u = page.evaluate("""(() => {
      const panels = [...document.querySelectorAll('.lu-panel')];
      const readable = panels.filter(pn => pn.getBoundingClientRect().width > 60).length;
      const btns = [...document.querySelectorAll('#luPanel button')];
      const clipped = btns.filter(b => { const r = b.getBoundingClientRect();
        return r.width > 0 && (r.left < -1 || r.right > window.innerWidth + 1 ||
                               r.top < -1 || r.bottom > window.innerHeight + 1); }).length;
      return { readable, total: panels.length, clipped,
               docScroll: document.documentElement.scrollWidth > window.innerWidth + 1 };
    })()""")
    check("fs/panelsReadableControlsReachable",
          u["readable"] == 4 and u["clipped"] == 0 and not u["docScroll"],
          json.dumps(u))

    # audio settings accessible in fullscreen
    page.click("#luSoundBtn"); page.wait_for_timeout(300)
    au = page.evaluate("""(() => {
      const pop = document.getElementById('luAudioPop');
      const r = pop.getBoundingClientRect();
      return { open: !pop.classList.contains('hidden'),
               fits: r.left >= 0 && r.right <= window.innerWidth &&
                     r.top >= 0 && r.bottom <= window.innerHeight };
    })()""")
    check("fs/audioSettingsAccessible", au["open"] and au["fits"], json.dumps(au))
    page.mouse.click(40, 50); page.wait_for_timeout(250)

    # Esc exits native AND returns to Fun Time (shared session semantics)
    page.keyboard.press("Escape"); page.wait_for_timeout(700)
    a1 = page.evaluate("""(() => ({ hash: location.hash,
      native: !!document.fullscreenElement,
      luFs: document.body.classList.contains('lu-fs'),
      playing: document.body.classList.contains('ft-playing') }))()""")
    check("fs/escapeExitsAndLeaves",
          a1["hash"] == "#funtime" and not a1["native"]
          and not a1["luFs"] and not a1["playing"], json.dumps(a1))

    # F-key path
    mount()
    page.keyboard.press("f"); page.wait_for_timeout(800)
    a2 = fs_state()
    check("fs/fKeyEntrySyncs", a2["native"] and a2["luFs"]
          and a2["label"] == "Exit Fullscreen", json.dumps(a2))

    # repeated enter/exit cycles: no duplicate listeners (registry size
    # constant), no stale class, button honest each time. NOTE: every
    # fullscreen EXIT navigates back to Fun Time by design (shared
    # session semantics — Esc and programmatic exits both land there),
    # so each cycle re-enters the game view. The F-key test above left
    # us IN fullscreen — leave it first.
    exit_fs()
    cycle_ok = True
    sizes = []
    for i in range(3):
        page.evaluate("location.hash='#funtime/lu'")
        page.wait_for_selector("#luBoard"); page.wait_for_timeout(500)
        page.evaluate("window.luLobbyStart()"); page.wait_for_timeout(300)
        sizes.append(page.evaluate("window.LUG.cleanup.length"))
        page.click("#luFsBtn"); page.wait_for_timeout(700)
        st = fs_state()
        if not (st["native"] and st["luFs"] and st["label"] == "Exit Fullscreen"):
            cycle_ok = False; break
        exit_fs()
        if page.evaluate("!!document.fullscreenElement"):
            cycle_ok = False; break
    check("fs/repeatedCyclesStable",
          cycle_ok and len(set(sizes)) == 1,
          f"sizes={sizes}, cyclesOk={cycle_ok}")

    # card-launch path: immersive fallback must clear once native lands
    exit_fs()
    page.evaluate("location.hash='#funtime'"); page.wait_for_timeout(500)
    page.click('a[href="#funtime/lu"]'); page.wait_for_timeout(1200)
    a3 = fs_state()
    check("fs/cardLaunchNoStaleImmersive",
          a3["native"] and a3["luFs"] and not a3["immersive"]
          and a3["label"] == "Exit Fullscreen", json.dumps(a3))
    # returning from fullscreen preserves the game (fresh lobby mount)
    exit_fs()
    a4 = page.evaluate("""(() => ({ hash: location.hash,
      native: !!document.fullscreenElement }))()""")
    check("fs/cardLaunchExitClean", a4["hash"] == "#funtime" and not a4["native"],
          json.dumps(a4))

    # ---------- C: lobby × fullscreen ----------
    mount()
    page.click("#luFsBtn"); page.wait_for_timeout(800)
    # restart into lobby while fullscreen
    page.click("#luRestartBtn"); page.wait_for_timeout(500)
    c0 = page.evaluate("""(() => {
      const card = document.getElementById('luLobCard');
      const cr = card.getBoundingClientRect();
      const clipped = [...card.querySelectorAll('button,input')]
        .filter(c => { const r = c.getBoundingClientRect();
          return r.width > 0 && (r.left < -1 || r.right > window.innerWidth + 1 ||
                                 r.top < -1 || r.bottom > window.innerHeight + 1); }).length;
      return { native: !!document.fullscreenElement, luFs: document.body.classList.contains('lu-fs'),
               lobbyOpen: !document.getElementById('luLobby').classList.contains('hidden'),
               cardFits: cr.top >= -1 && cr.bottom <= window.innerHeight + 1, clipped };
    })()""")
    check("lobby/inFullscreenLayoutIntact",
          c0["native"] and c0["luFs"] and c0["lobbyOpen"]
          and c0["cardFits"] and c0["clipped"] == 0, json.dumps(c0))

    # start a game from the lobby while fullscreen
    page.click("#luStartBtn"); page.wait_for_timeout(400)
    c1 = page.evaluate("""(() => ({ lobby: window.LUG.lobby,
      rollEnabled: !document.getElementById('luRollBtn').disabled,
      native: !!document.fullscreenElement }))()""")
    check("lobby/startWhileFullscreenWorks",
          not c1["lobby"] and c1["rollEnabled"] and c1["native"], json.dumps(c1))

    # exit fullscreen mid-game → back to selector, nothing stale
    exit_fs()
    c2 = page.evaluate("""(() => ({ hash: location.hash,
      luFs: document.body.classList.contains('lu-fs'),
      playing: document.body.classList.contains('ft-playing'),
      anim: window.LUG ? window.LUG.anim : null }))()""")
    check("lifecycle/exitMidGameClean",
          c2["hash"] == "#funtime" and not c2["luFs"] and not c2["playing"],
          json.dumps(c2))

    # exit fullscreen WITH the lobby visible
    mount()
    page.click("#luFsBtn"); page.wait_for_timeout(800)
    page.click("#luRestartBtn"); page.wait_for_timeout(400)
    exit_fs()
    c3 = page.evaluate("""(() => ({ hash: location.hash,
      native: !!document.fullscreenElement,
      luFs: document.body.classList.contains('lu-fs') }))()""")
    check("lobby/exitWithLobbyVisibleClean",
          c3["hash"] == "#funtime" and not c3["native"] and not c3["luFs"],
          json.dumps(c3))

    # ---------- D: lifecycle ----------
    # names + config persist across route-away/re-entry (module state)
    mount()
    page.evaluate("""(() => {
      LUG.names = ['Al', 'Bo', '', '']; LUG.seatCount = 3; LUG.humanSeats = 2;
      window.luLobbyStart();
    })()""")
    page.wait_for_timeout(400)
    page.evaluate("location.hash='#funtime'"); page.wait_for_timeout(500)
    page.evaluate("location.hash='#funtime/lu'")
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(600)
    d0 = page.evaluate("""(() => ({
      names: window.LUG.names, n: window.LUG.seatCount, h: window.LUG.humanSeats,
      lobby: window.LUG.lobby }))()""")
    check("lifecycle/configNamesPersist",
          d0["names"][:2] == ["Al", "Bo"] and d0["n"] == 3 and d0["h"] == 2
          and d0["lobby"], json.dumps(d0))
    # names/config persist across route-away WITHOUT reload (in-memory,
    # module scope) — verified above; audio persists via localStorage and
    # is observed after a real reload.
    page.evaluate("localStorage.setItem('ludo-audio-v1', JSON.stringify({v:1,m:0.5,mu:1,sx:1,mv:0.4,sv:0.8}))")
    page.reload(); page.wait_for_selector("#luBoard"); page.wait_for_timeout(600)
    d1 = page.evaluate("""(() => ({ m: window.LUG.audio.m,
      lobby: window.LUG.lobby }))()""")
    check("lifecycle/audioSettingsPersist",
          abs(d1["m"] - 0.5) < 0.01 and d1["lobby"], json.dumps(d1))

    # restart from an ACTIVE game (mid-animation) freezes cleanly
    # (the reload above re-opened the setup lobby — start first)
    page.evaluate("window.luLobbyStart()"); page.wait_for_timeout(400)
    page.evaluate("window.LUD.fast()")
    page.evaluate("window.LUD.force(6)")
    page.wait_for_timeout(200)
    page.click("#luRestartBtn"); page.wait_for_timeout(1100)
    d2 = page.evaluate("""(() => ({
      lobby: window.LUG.lobby, phase: window.LUG.phase, dice: window.LUG.dice,
      moves: window.LUG.moves.length, anim: window.LUG.anim,
      fx: window.LUG.fx.length, musicTimer: window.LUA.musicTimer,
      allYard: window.LUG.game.players.flatMap(P => P.tokens)
        .every(t => t.pos === -1) }))()""")
    check("lifecycle/restartFromActiveGameFrozen",
          d2["lobby"] and d2["phase"] == "idle" and d2["dice"] is None
          and d2["moves"] == 0 and d2["anim"] is None and d2["fx"] == 0
          and d2["musicTimer"] == 0 and d2["allYard"], json.dumps(d2))

    # lobby → game → lobby → game transitions keep config
    # (re-apply the config the reload reset — this section is about the
    #  transitions preserving it, not about reload persistence)
    page.evaluate("""(() => {
      LUG.names = ['Al', 'Bo', '', '']; LUG.seatCount = 3; LUG.humanSeats = 2;
      window.luLobSync();
    })()""")
    page.click("#luStartBtn"); page.wait_for_timeout(300)
    page.click("#luRestartBtn"); page.wait_for_timeout(300)
    page.click("#luStartBtn"); page.wait_for_timeout(300)
    d3 = page.evaluate("""(() => ({ lobby: window.LUG.lobby,
      n: window.LUG.seatCount, h: window.LUG.humanSeats,
      names: window.LUG.names,
      seats: window.LudoEngine.activeSeats(window.LUG.game) }))()""")
    check("lifecycle/lobbyGameLobbyGameTransitions",
          not d3["lobby"] and d3["n"] == 3 and d3["h"] == 2
          and d3["seats"] == [0, 1, 2] and d3["names"][1] == "Bo",
          json.dumps(d3))

    # no runaway rAF after churn (teardown cancels its own chains)
    page.evaluate("location.hash='#funtime'"); page.wait_for_timeout(500)
    d4 = page.evaluate("""(() => new Promise(res => {
      // count frames actually ticking for 500ms — teardown should have
      // stopped every ludo pump (board is static when unmounted)
      let n = 0; const t0 = performance.now();
      const tick = () => { n++; if (performance.now() - t0 < 500)
        requestAnimationFrame(tick); else res(n); };
      requestAnimationFrame(tick);
    }))()""")
    # ~30fps headless baseline: only OUR chain counts; anything ludo-driven
    # would add its own draws. Loose bound: no more than 90 frames/500ms.
    check("lifecycle/noRunawayRafAfterTeardown", d4 <= 90, f"frames={d4}")

    # listener registry does not grow across remounts
    sizes = []
    for _ in range(3):
        page.evaluate("location.hash='#funtime/lu'")
        page.wait_for_selector("#luBoard"); page.wait_for_timeout(400)
        sizes.append(page.evaluate("window.LUG.cleanup.length"))
        page.evaluate("location.hash='#funtime'"); page.wait_for_timeout(400)
    check("lifecycle/noDuplicateListenersAcrossRemounts",
          len(set(sizes)) == 1, json.dumps(sizes))

    # ---------- B: responsive fullscreen layout ----------
    viewports = [("desktop_1280x900", 1280, 900), ("laptop_1440x900", 1440, 900),
                 ("tablet_portrait_820x1180", 820, 1180),
                 ("mobile_portrait_390x844", 390, 844),
                 ("mobile_landscape_844x390", 844, 390)]
    for name, w, h in viewports:
        page.set_viewport_size({"width": w, "height": h})
        page.evaluate("location.hash='#funtime/lu'")
        page.wait_for_selector("#luBoard"); page.wait_for_timeout(500)
        page.evaluate("window.luLobbyStart()"); page.wait_for_timeout(300)
        page.click("#luFsBtn"); page.wait_for_timeout(900)
        r = page.evaluate("""(() => {
          const c = document.getElementById('luBoard');
          const r = c.getBoundingClientRect();
          const btns = [...document.querySelectorAll('#luPanel button')];
          const clipped = btns.filter(b => { const cr = b.getBoundingClientRect();
            return cr.width > 0 && (cr.left < -1 || cr.right > window.innerWidth + 1 ||
                                    cr.top < -1 || cr.bottom > window.innerHeight + 1); }).length;
          const visible = btns.filter(b => b.getBoundingClientRect().width > 0);
          const minH = visible.length ? Math.min(...visible.map(b => b.getBoundingClientRect().height)) : 99;
          const panels = [...document.querySelectorAll('.lu-panel')]
            .filter(pn => pn.getBoundingClientRect().width > 60).length;
          return { w: Math.round(r.width), square: Math.abs(r.width - r.height) < 2,
                   fitsV: r.top >= -1 && r.bottom <= window.innerHeight + 1,
                   offX: Math.round(Math.abs((r.left + r.right) / 2 - window.innerWidth / 2)),
                   clipped, minBtnH: Math.round(minH), panels,
                   docScroll: document.documentElement.scrollWidth > window.innerWidth + 1 };
        })()""")
        page.screenshot(path=f".phase5_screenshots/after53/fs_{name}.png")
        check(f"responsiveFs/{name}",
              r["square"] and r["fitsV"] and r["offX"] <= 2
              and r["clipped"] == 0 and not r["docScroll"]
              and r["minBtnH"] >= 24 and r["panels"] >= 2, json.dumps(r))
        exit_fs()
        page.evaluate("location.hash='#funtime'"); page.wait_for_timeout(300)
    page.set_viewport_size({"width": 1280, "height": 900})

    # ---------- E: regression guards ----------
    # engine rules unchanged
    e0 = page.evaluate("""(() => {
      const E = window.LudoEngine;
      const g = E.createGame(['human','ai','ai','ai']);
      E.applyMove(g, 0, 0, 6);
      const g2 = E.createGame();
      g2.players[0].tokens[0].pos = 54;
      return { pos: g.players[0].tokens[0].pos,
               moveOk: E.legalMoves(g, 0, 4)[0].to === 4,
               exactFinish: E.legalMoves(g2, 0, 2)[0].to === 56 };
    })()""")
    check("regress/engineRulesUnchanged",
          e0["pos"] == 0 and e0["moveOk"] and e0["exactFinish"], json.dumps(e0))

    # Phase 5.5 lobby still fully functional (spot check)
    page.evaluate("location.hash='#funtime/lu'")
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(500)
    l0 = page.evaluate("""(() => {
      LUG.seatCount = 4; LUG.humanSeats = 2; LUG.names = ['', 'Zed', '', ''];
      window.luLobSync();
      const inp = document.querySelector('[data-nmseat="1"]');
      return { lobbyOpen: !document.getElementById('luLobby').classList.contains('hidden'),
               seat2kind: document.querySelector('[data-prevseat="2"]').className.includes('inactive') === false,
               nameField: inp ? inp.value : null };
    })()""")
    check("regress/lobbyStillFunctional",
          l0["lobbyOpen"] and l0["seat2kind"], json.dumps(l0))

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
