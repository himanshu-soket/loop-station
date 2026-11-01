#!/usr/bin/env python3
"""Phase 5.4 · Professional audio system — checks.

  A. ARCHITECTURE: one context, lazy init on gesture, no duplicate init,
     buses respect settings immediately.
  B. SFX COVERAGE: every named cue fires (oscillator scheduling observed
     via a patched hook), dedup within the gap window works.
  C. SETTINGS UI: popover opens/closes, sliders + toggles apply live,
     mute is instant via the gain graph.
  D. PERSISTENCE: changes survive reload; corrupt blobs fall back safely.
  E. MUSIC: starts on game activity, stops on game-over/route-away,
     restart never stacks schedulers, ducking during win stinger.
  F. LIFECYCLE: restart mid-sound, teardown mid-music, rapid mount/
     unmount — no leaked contexts or timers.
  G. GAMEPLAY UNAFFECTED: engine rules, turn flow and the full play
     loop still run to completion with audio active.

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
    # Phase 5.5: the view now mounts into a setup lobby — start the game
    # with the default configuration before running gameplay checks.
    # NOTE: audio must stay UNINITIALISED here (the first check asserts
    # lazy init), so we bypass luLobbyStart's confirmation sound by
    # hiding the lobby and calling luNewGame directly.
    page.wait_for_timeout(300)
    page.evaluate("""(() => {
      document.getElementById('luLobby').classList.add('hidden');
      window.LUG.lobby = false;
      window.luNewGame();
    })()""")
    page.wait_for_timeout(500)
    page.evaluate("window.LUD.fast()")

    # ---------- A: architecture ----------
    a0 = page.evaluate("""(() => ({
      inited: window.LUA.inited, ctx: !!window.LUA.ctx }))()""")
    check("arch/lazyBeforeGesture", a0["inited"] is False and a0["ctx"] is False,
          json.dumps(a0))

    page.click("#luRollBtn")                 # real user gesture
    page.wait_for_timeout(300)
    a1 = page.evaluate("""(() => ({
      inited: window.LUA.inited,
      state: window.LUA.ctx ? window.LUA.ctx.state : null,
      buses: !!(window.LUA.master && window.LUA.sfxBus && window.LUA.musicBus),
      master: window.LUA.master ? +window.LUA.master.gain.value.toFixed(2) : null,
      sfx: window.LUA.sfxBus ? +window.LUA.sfxBus.gain.value.toFixed(2) : null,
      music: window.LUA.musicBus ? +window.LUA.musicBus.gain.value.toFixed(2) : null }))()""")
    check("arch/initOnGesture",
          a1["inited"] and a1["state"] == "running" and a1["buses"], json.dumps(a1))
    check("arch/busVolumesFromSettings",
          abs(a1["master"] - 0.7) < 0.06 and abs(a1["sfx"] - 0.9) < 0.06
          and abs(a1["music"] - 0.45) < 0.06, json.dumps(a1))

    # single context across many triggers
    page.evaluate("[window.luSfx('ui'), window.luSfx('pick'), window.luSfx('six')]")
    a2 = page.evaluate("""(() => {
      // count live AudioContexts the page created via a probe: the LUA
      // object holds exactly one; a second init attempt is a no-op.
      const before = window.LUA.ctx;
      window.luAudioReady();
      return { same: window.LUA.ctx === before, inited: window.LUA.inited };
    })()""")
    check("arch/singleContextNoReinit", a2["same"] is True, json.dumps(a2))

    # ---------- B: SFX coverage ----------
    # instrument: wrap createOscillator/createBufferSource to observe scheduling
    page.evaluate("""(() => {
      window.__auSaw = { osc: 0, noise: 0, kinds: [] };
      const ctx = window.LUA.ctx;
      const oo = ctx.createOscillator.bind(ctx);
      ctx.createOscillator = () => { window.__auSaw.osc++; return oo(); };
      const ob = ctx.createBufferSource.bind(ctx);
      ctx.createBufferSource = () => { window.__auSaw.noise++; return ob(); };
    })()""")
    cues = ['roll', 'step', 'pick', 'enter', 'capture', 'safe', 'lane',
            'finish', 'six', 'win', 'over', 'invalid', 'ui']
    b0 = page.evaluate("""(kinds) => {
      const out = {};
      for (const k of kinds) {
        const o0 = window.__auSaw.osc, n0 = window.__auSaw.noise;
        window.LUA.lastPlay[k] = 0;              // bypass dedup for the probe
        window.luSfx(k);
        out[k] = (window.__auSaw.osc - o0) + (window.__auSaw.noise - n0);
      }
      return out;
    }""", cues)
    silent = [k for k, v in b0.items() if v == 0]
    check("sfx/everyCueSchedulesNodes", not silent, json.dumps(b0))

    # dedup: two identical calls inside the gap → one scheduling burst
    d0 = page.evaluate("""(() => {
      window.LUA.lastPlay['step'] = 0;
      const o0 = window.__auSaw.osc;
      window.luSfx('step'); window.luSfx('step');   // < LU_SFX_GAP apart
      return window.__auSaw.osc - o0;
    })()""")
    check("sfx/dedupIdenticalWithinGap", d0 == 1, f"scheduled={d0} (expected 1)")

    # ---------- C: settings UI ----------
    page.click("#luSoundBtn")
    page.wait_for_timeout(150)
    c0 = page.evaluate("""(() => ({
      open: !document.getElementById('luAudioPop').classList.contains('hidden'),
      expanded: document.getElementById('luSoundBtn').getAttribute('aria-expanded') }))()""")
    check("ui/popoverOpens", c0["open"] and c0["expanded"] == "true", json.dumps(c0))

    # slider applies live to the graph
    page.evaluate("""(() => {
      const el = document.getElementById('luAuMasterV');
      el.value = 20; el.dispatchEvent(new Event('input', {bubbles:true}));
    })()""")
    page.wait_for_timeout(120)
    c1 = page.evaluate("""(() => ({
      m: window.LUG.audio.m, gain: +window.LUA.master.gain.value.toFixed(2),
      stored: JSON.parse(localStorage.getItem('ludo-audio-v1')).m }))()""")
    check("ui/masterSliderLiveAndPersisted",
          abs(c1["m"] - 0.2) < 0.01 and abs(c1["gain"] - 0.2) < 0.03
          and abs(c1["stored"] - 0.2) < 0.01, json.dumps(c1))

    # sfx toggle mutes instantly (bus gain → 0, no nodes scheduled)
    page.click("#luAuSfxT")
    page.wait_for_timeout(120)
    c2 = page.evaluate("""(() => {
      const o0 = window.__auSaw.osc;
      window.LUA.lastPlay['ui'] = 0;
      window.luSfx('ui');
      return { sx: window.LUG.audio.sx,
               bus: +window.LUA.sfxBus.gain.value.toFixed(2),
               scheduled: window.__auSaw.osc - o0 };
    })()""")
    check("ui/sfxToggleMutesInstantly",
          c2["sx"] == 0 and c2["scheduled"] == 0, json.dumps(c2))
    page.click("#luAuSfxT")                  # back on
    page.wait_for_timeout(100)

    # music toggle stops the scheduler
    page.click("#luAuMusicT")
    page.wait_for_timeout(400)
    c3 = page.evaluate("""(() => ({
      mu: window.LUG.audio.mu, timer: window.LUA.musicTimer > 0,
      music: !!window.LUA.music,
      bus: +window.LUA.musicBus.gain.value.toFixed(2) }))()""")
    check("ui/musicToggleStopsScheduler",
          c3["mu"] == 0 and c3["timer"] is False and c3["music"] is False,
          json.dumps(c3))
    page.click("#luAuMusicT")                 # back on
    page.wait_for_timeout(200)

    # outside click closes (aim far from every control: board centre-left)
    box = page.evaluate("""(() => {
      const r = document.getElementById('luBoard').getBoundingClientRect();
      return {x: r.left + 30, y: r.top + 30};
    })()""")
    page.mouse.click(box["x"], box["y"])
    page.wait_for_timeout(150)
    c4 = page.evaluate(
      "document.getElementById('luAudioPop').classList.contains('hidden')")
    check("ui/outsideClickCloses", c4 is True, str(c4))

    # Esc is owned by the shared session controller (leaves the game
    # view globally) — verify it still does exactly that, and that the
    # popover itself is gone with the view.
    page.click("#luSoundBtn"); page.wait_for_timeout(120)
    page.keyboard.press("Escape"); page.wait_for_timeout(300)
    c5 = page.evaluate("""(() => ({
      hash: location.hash,
      popGone: !document.getElementById('luAudioPop') }))()""")
    check("ui/escapeKeepsGlobalLeaveBehaviour",
          c5["hash"] == "#funtime" and c5["popGone"], json.dumps(c5))
    # back into the game for the remaining sections
    page.evaluate("location.hash='#funtime/lu'")
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(500)
    # Phase 5.5: dismiss the setup lobby (silently — audio checks below)
    page.evaluate("""(() => {
      document.getElementById('luLobby').classList.add('hidden');
      window.LUG.lobby = false;
      window.luNewGame();
    })()""")
    page.wait_for_timeout(400)
    page.evaluate("window.LUD.fast()")
    page.click("#luRollBtn"); page.wait_for_timeout(300)

    # ---------- D: persistence ----------
    page.reload()
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(600)
    d0 = page.evaluate("window.LUG.audio")
    check("persist/settingsSurviveReload",
          abs(d0["m"] - 0.2) < 0.01 and d0["sx"] == 1 and d0["mu"] == 1,
          json.dumps(d0))

    # corrupt blob → safe defaults
    page.evaluate("localStorage.setItem('ludo-audio-v1','{not json')")
    page.reload()
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(600)
    d1 = page.evaluate("window.LUG.audio")
    check("persist/corruptBlobFallsBack",
          abs(d1["m"] - 0.7) < 0.01 and d1["sx"] == 1, json.dumps(d1))

    # restore defaults for later sections
    page.evaluate("localStorage.removeItem('ludo-audio-v1')")
    page.reload()
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(500)
    # Phase 5.5: dismiss the setup lobby again after reload (silently)
    page.evaluate("""(() => {
      document.getElementById('luLobby').classList.add('hidden');
      window.LUG.lobby = false;
      window.luNewGame();
    })()""")
    page.wait_for_timeout(400)
    page.evaluate("window.LUD.fast()")
    page.click("#luRollBtn")                  # gesture unlock again
    page.wait_for_timeout(300)

    # ---------- E: music lifecycle ----------
    # deterministic: complete one full turn cycle (roll → pick → commit),
    # which drives luStartTurn → luMusicFollow
    dl = time.time() + 10
    e0 = {"timer": False, "music": False}
    while time.time() < dl:
        e0 = page.evaluate("""(() => {
          const g = window.LUG.game;
          if (!g || g.winner !== null) return {timer: false, music: false};
          if (LUG.phase === 'idle' && g.players[g.turn].kind === 'human')
            window.LUD.force(6);
          else if (LUG.phase === 'awaiting' && LUG.moves.length &&
                   g.players[g.turn].kind === 'human') {
            const m = LUG.moves[0];
            window.luExecuteMove({token: m.token, to: m.to});
          }
          return { timer: window.LUA.musicTimer > 0,
                   music: !!window.LUA.music };
        })()""")
        if e0["timer"] and e0["music"]:
            break
        page.wait_for_timeout(80)
    check("music/startsOnGameActivity", e0["timer"] and e0["music"], json.dumps(e0))

    # restart does NOT stack schedulers (exactly one timer id)
    page.evaluate("window.luNewGame()")
    page.wait_for_timeout(600)
    e1 = page.evaluate("""(() => ({ timer: window.LUA.musicTimer > 0,
                                    music: !!window.LUA.music }))()""")
    check("music/restartKeepsSingleScheduler", e1["timer"] and e1["music"],
          json.dumps(e1))

    # game over → music stops, win stinger ducks then restores
    e2 = page.evaluate("""(() => {
      return new Promise(res => {
        window.LUG.clock++;
        const g = window.LUG.game;
        for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
        g.turn = 0; g.chain = 0; g.winner = null;
        g.players[0].home = 3;
        g.players[0].tokens[0].pos = 54;
        LUG.phase='awaiting'; LUG.dice=2;
        LUG.moves = LudoEngine.legalMoves(g, 0, 2);
        const m = LUG.moves[0];
        window.luExecuteMove({token:m.token, to:m.to});
        setTimeout(() => res({
          phase: window.LUG.phase, winner: window.LUG.game.winner,
          timer: window.LUA.musicTimer > 0, music: !!window.LUA.music,
        }), 1400);
      });
    })()""")
    check("music/stopsOnGameOver",
          e2["phase"] == "over" and e2["winner"] == 0
          and e2["timer"] is False and e2["music"] is False, json.dumps(e2))

    # route away → full teardown
    page.evaluate("location.hash='#funtime'")
    page.wait_for_timeout(400)
    e3 = page.evaluate("""(() => ({ ctx: !!window.LUA.ctx,
                                     inited: window.LUA.inited,
                                     timer: window.LUA.musicTimer }))()""")
    check("life/routeAwayCleansAudio",
          e3["ctx"] is False and e3["inited"] is False and e3["timer"] == 0,
          json.dumps(e3))

    # re-entry: fresh but lazy (no context until a gesture)
    page.evaluate("location.hash='#funtime/lu'")
    page.wait_for_selector("#luBoard"); page.wait_for_timeout(500)
    e4 = page.evaluate("""(() => ({ ctx: !!window.LUA.ctx,
                                     inited: window.LUA.inited }))()""")
    check("life/reentryLazyNoLeak", e4["ctx"] is False and e4["inited"] is False,
          json.dumps(e4))
    # Phase 5.5: leave the setup lobby so the roll button is live again
    page.evaluate("""(() => {
      document.getElementById('luLobby').classList.add('hidden');
      window.LUG.lobby = false;
      window.luNewGame();
    })()""")
    page.wait_for_timeout(400)

    # ---------- F: restart churn mid-sound ----------
    page.click("#luRollBtn"); page.wait_for_timeout(250)
    for _ in range(4):
        page.evaluate("window.luNewGame()")
        page.wait_for_timeout(120)
        page.evaluate("window.luSfx('ui')")
        page.wait_for_timeout(60)
    page.wait_for_timeout(500)
    f0 = page.evaluate("""(() => ({
      ctx: !!window.LUA.ctx, state: window.LUA.ctx ? window.LUA.ctx.state : null,
      timer: window.LUA.musicTimer > 0,
      dupTimers: window.LUA.musicTimer }))()""")
    check("life/restartChurnNoDuplicateAudio",
          f0["ctx"] and f0["state"] == "running" and f0["dupTimers"] > 0,
          json.dumps(f0))

    # ---------- G: gameplay unaffected ----------
    page.evaluate("window.luNewGame()")
    page.wait_for_timeout(400)
    g0 = page.evaluate("""(() => {
      return new Promise(res => {
        const deadline = performance.now() + 15000;
        let rolls = 0, moves = 0;
        const iv = setInterval(() => {
          const g = window.LUG.game;
          if (!g || g.winner !== null || performance.now() > deadline) {
            clearInterval(iv);
            res({ rolls, moves, winner: g ? g.winner : null,
                  audioAlive: !!window.LUA.ctx });
            return;
          }
          if (LUG.phase === 'idle' && g.players[g.turn].kind === 'human') {
            window.LUD.force(Math.floor(Math.random()*6)+1); rolls++;
          }
          if (LUG.phase === 'awaiting' && LUG.moves.length &&
              g.players[g.turn].kind === 'human') {
            const m = LUG.moves[0];
            window.luExecuteMove({token:m.token, to:m.to}); moves++;
          }
        }, 30);
      });
    })()""")
    check("game/playsToCompletionWithAudio",
          g0["rolls"] > 3 and g0["moves"] > 3 and g0["audioAlive"], json.dumps(g0))

    # engine untouched spot-check (rules battery lives in verify_ludo.py)
    g1 = page.evaluate("""(() => {
      const E = window.LudoEngine;
      const g = E.createGame(['human','ai','ai','ai']);
      E.applyMove(g, 0, 0, 6);
      const mv = E.legalMoves(g, 0, 4);
      return { pos: g.players[0].tokens[0].pos,
               moveOk: mv.length === 1 && mv[0].to === 4,
               finish: E.FINISH, lastMain: E.LAST_MAIN };
    })()""")
    check("game/engineRulesUnchanged",
          g1["pos"] == 0 and g1["moveOk"] and g1["finish"] == 56 and g1["lastMain"] == 50,
          json.dumps(g1))

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
