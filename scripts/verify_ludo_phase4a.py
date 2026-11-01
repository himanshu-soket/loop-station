#!/usr/bin/env python3
"""Phase 4.1 — Dynamic player-count configuration (2 / 3 / 4 players).

Focused suite covering:
  • engine seat semantics per configuration
  • selector UI state + selected-state styling hooks
  • turn rotation skips inactive seats
  • inactive seats never roll / move / win
  • dice ownership
  • capture + safe squares across configurations
  • winner detection among active seats only
  • restart preserves the chosen count
  • back-nav and re-entry preserve the choice
  • repeated initialisation leaves no stale listeners/timers/rAFs
  • legacy four-player behaviour unchanged

Run with the dashboard on 127.0.0.1:8908 (see scripts/verify_ludo.py header).
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8908"
PASS, FAIL = 0, 0

def check(name, ok, detail=""):
    global PASS, FAIL
    print(("PASS  " if ok else "FAIL  ") + name + ("  [" + str(detail) + "]" if detail else ""))
    if ok: PASS += 1
    else: FAIL += 1


def main():
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
        # Phase 5.5: the view mounts into a setup lobby and the seat
        # selectors now live inside it. Keep the lobby OPEN here — this
        # suite drives the selectors directly; each config change is
        # confirmed by pressing Start (helper below).
        page.wait_for_timeout(300)

        def start_game():
            """Phase 5.5: apply the lobby configuration to a live game."""
            if page.evaluate("window.LUG.lobby"):
                page.evaluate("window.luLobbyStart()")
                page.wait_for_timeout(400)

        def open_lobby():
            """Phase 5.5: bring the setup lobby back (selectors live there)."""
            if not page.evaluate("window.LUG.lobby"):
                page.evaluate("window.luLobbyShow()")
                page.wait_for_timeout(300)
        start_game()
        page.wait_for_timeout(500)
        page.evaluate("window.LUD.fast()")

        def sel_state():
            return page.evaluate("""(() => ({
              count: window.LUG.seatCount,
              on: [...document.querySelectorAll('#luSeatPick .lu-sp-btn')]
                   .filter(b => b.classList.contains('on')).map(b => b.dataset.seats),
              aria: [...document.querySelectorAll('#luSeatPick .lu-sp-btn')]
                   .map(b => b.getAttribute('aria-checked')),
              seats: window.LudoEngine.activeSeats(window.LUG.game),
              flags: window.LUG.game.players.map(P => P.active)}))()""")

        def pick_legal():
            css = page.evaluate("""(() => {
              const stacks = window.__luBuildStacks();
              const mv = window.LUG.moves[0]; if (!mv) return null;
              const pt = window.__luTokenPoint(window.LUG.game.turn, mv.token, stacks);
              if (!pt) return null;
              const r = document.getElementById('luBoard').getBoundingClientRect();
              return {x: r.left + pt.x*(r.width/window.LUG.w),
                      y: r.top + pt.y*(r.height/window.LUG.h)};
            })()""")
            if css: page.mouse.click(css["x"], css["y"])
            return bool(css)

        # ---------- 1: engine seat semantics ----------
        r = page.evaluate("""(() => {
          const E = window.LudoEngine, out = {};
          for (const n of [2, 3, 4]) {
            const seats = {2:[0,2], 3:[0,1,2], 4:[0,1,2,3]}[n];
            const g = E.createGame(['human','ai','ai','ai'], {seats});
            out['cfg'+n] = {seats: g.seats, flags: g.players.map(P=>P.active),
                            act: E.activeSeats(g)};
          }
          const bad = E.createGame(['human','ai','ai','ai'], {seats:[1,2]});
          out.badNoZero = bad.seats;
          const legacy = E.createGame(['human','ai','ai','ai']);
          out.legacy = {seats: legacy.seats, flags: legacy.players.map(P=>P.active)};
          return out;
        })()""")
        check("engine/seats2P", r["cfg2"]["act"] == [0, 2] and r["cfg2"]["flags"] == [True, False, True, False], json.dumps(r["cfg2"]))
        check("engine/seats3P", r["cfg3"]["act"] == [0, 1, 2] and r["cfg3"]["flags"] == [True, True, True, False], json.dumps(r["cfg3"]))
        check("engine/seats4P", r["cfg4"]["act"] == [0, 1, 2, 3] and all(r["cfg4"]["flags"]), json.dumps(r["cfg4"]))
        check("engine/invalidFallsBack", r["badNoZero"] == [0, 1, 2, 3], json.dumps(r["badNoZero"]))
        check("engine/legacyUnchanged", r["legacy"]["seats"] == [0, 1, 2, 3] and all(r["legacy"]["flags"]), json.dumps(r["legacy"]))

        r = page.evaluate("""(() => {
          const E = window.LudoEngine, out = {};
          const g2 = E.createGame(['human','ai','ai','ai'], {seats:[0,2]});
          const s2 = []; for (let i=0;i<6;i++){ s2.push(g2.turn); E.advanceTurn(g2); } out.rot2 = s2;
          const g3 = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2]});
          const s3 = []; for (let i=0;i<7;i++){ s3.push(g3.turn); E.advanceTurn(g3); } out.rot3 = s3;
          const g4 = E.createGame(['human','ai','ai','ai']);
          const s4 = []; for (let i=0;i<8;i++){ s4.push(g4.turn); E.advanceTurn(g4); } out.rot4 = s4;
          const gw = E.createGame(['human','ai','ai','ai'], {seats:[0,2]});
          gw.players[1].home = 4; gw.players[0].home = 4;
          out.winActive = E.checkWinner(gw);
          const gw2 = E.createGame(['human','ai','ai','ai'], {seats:[0,2]});
          gw2.players[1].home = 4;
          out.winInactiveOnly = E.checkWinner(gw2);
          return out;
        })()""")
        check("engine/rotation2P", r["rot2"] == [0, 2, 0, 2, 0, 2], json.dumps(r["rot2"]))
        check("engine/rotation3P", r["rot3"] == [0, 1, 2, 0, 1, 2, 0], json.dumps(r["rot3"]))
        check("engine/rotation4P", r["rot4"] == [0, 1, 2, 3, 0, 1, 2, 3], json.dumps(r["rot4"]))
        check("engine/winnerActiveOnly", r["winActive"] == 0, json.dumps(r["winActive"]))
        check("engine/inactiveNeverWins", r["winInactiveOnly"] is None, json.dumps(r["winInactiveOnly"]))

        # ---------- 2: default is 4P ----------
        s = sel_state()
        check("ui/defaultIs4P", s["count"] == 4 and s["seats"] == [0, 1, 2, 3] and s["on"] == ["4"], json.dumps(s))

        # ---------- 3: 2P config end-to-end ----------
        open_lobby(); page.click("#luSeatPick [data-seats='2']")
        start_game()
        page.wait_for_timeout(600)
        s = sel_state()
        blurb = page.evaluate("document.getElementById('luBlurb').textContent")
        check("ui/select2P", s["count"] == 2 and s["seats"] == [0, 2] and s["on"] == ["2"], json.dumps(s))
        check("ui/blurbNamesSeats", "You" in blurb and "Yellow" in blurb and "Green" not in blurb, blurb)
        # rotation through real play
        seen = set(); t0 = time.time()
        while time.time() - t0 < 15 and seen != {0, 2}:
            st = page.evaluate("({t: window.LUG.game.turn, ph: window.LUG.phase})")
            seen.add(st["t"])
            if st["ph"] == "idle":
                page.evaluate("window.LUD.force(6)")
                page.wait_for_timeout(150)
                if page.evaluate("window.LUG.phase") == "awaiting": pick_legal()
            page.wait_for_timeout(150)
        check("ui/rotation2PPlay", seen == {0, 2}, sorted(seen))
        # restart keeps 2P
        page.click("#luRestartBtn"); start_game()
        page.wait_for_timeout(500)
        s = sel_state()
        check("ui/restartKeeps2P", s["count"] == 2 and s["seats"] == [0, 2], json.dumps(s))

        # ---------- 4: 3P config end-to-end ----------
        open_lobby(); page.click("#luSeatPick [data-seats='3']")
        start_game()
        page.wait_for_timeout(600)
        s = sel_state()
        check("ui/select3P", s["count"] == 3 and s["seats"] == [0, 1, 2] and s["on"] == ["3"], json.dumps(s))
        seen = set(); t0 = time.time()
        while time.time() - t0 < 20 and seen != {0, 1, 2}:
            st = page.evaluate("""({t: window.LUG.game.turn, ph: window.LUG.phase,
                 k: window.LUG.game.players[window.LUG.game.turn].kind})""")
            seen.add(st["t"])
            if st["ph"] == "idle":
                if st["k"] == "human":
                    page.evaluate("window.LUD.force(6)")
                    page.wait_for_timeout(150)
                    if page.evaluate("window.LUG.phase") == "awaiting": pick_legal()
                else:
                    page.wait_for_timeout(150)
            elif st["ph"] == "awaiting" and st["k"] == "human":
                pick_legal()
            else:
                page.wait_for_timeout(100)
        check("ui/rotation3PPlay", seen == {0, 1, 2}, sorted(seen))

        # ---------- 5: 4P unchanged ----------
        open_lobby(); page.click("#luSeatPick [data-seats='4']")
        start_game()
        page.wait_for_timeout(600)
        s = sel_state()
        check("ui/select4P", s["count"] == 4 and s["seats"] == [0, 1, 2, 3] and s["on"] == ["4"], json.dumps(s))

        # ---------- 6: same-count click does not restart ----------
        # Phase 5.5 semantics: opening the lobby intentionally resets the
        # board (that IS the restart), but clicking the ALREADY-SELECTED
        # option must be a pure no-op — no config change, no forced
        # Start, no second reset. Verified on the configuration object.
        start_game()
        page.evaluate("window.LUG.game.rolls = 5")   # marker
        open_lobby()
        cfg_before = page.evaluate(
          "JSON.stringify([LUG.seatCount, LUG.humanSeats, LUG.aiDifficulty])")
        game_before = page.evaluate("window.LUG.game.rolls")
        page.click("#luSeatPick [data-seats='4']")
        page.wait_for_timeout(300)
        cfg_after = page.evaluate(
          "JSON.stringify([LUG.seatCount, LUG.humanSeats, LUG.aiDifficulty])")
        game_after = page.evaluate("window.LUG.game.rolls")
        check("ui/sameCountNoRestart",
              cfg_before == cfg_after and game_before == game_after,
              f"cfg {cfg_before}→{cfg_after} rolls {game_before}→{game_after}")

        # ---------- 7: capture + safe squares in a 2P game ----------
        open_lobby(); page.click("#luSeatPick [data-seats='2']")
        start_game()
        page.wait_for_timeout(500)
        # stage: red rel 4, yellow rel 44 → both global 5; dice 1 → red captures
        page.evaluate("""(() => {
          window.LUG.clock++; window.luCancelDiceSpin(); window.luCancelAnim();
          const g = window.LUG.game;
          g.winner = null; g.turn = 0; g.chain = 0;
          g.players.forEach(P => P.tokens.forEach(t => { t.pos = -1; delete t._viz; }));
          g.players[0].tokens[0].pos = 4;
          // yellow rel 44 = global (26+44)%52 = 18; want global 5 → rel (5-26+52)%52 = 31
          g.players[2].tokens[0].pos = 31;
          window.LUG.phase = 'idle'; window.LUG.dice = null; window.LUG.moves = [];
          window.LUG.anim = null; window.LUG.settleUntil = 0; window.LUG.nudge = null;
          window.LUD.force(1);
        })()""")
        dl = time.time() + 3
        while time.time() < dl:
            if page.evaluate("window.LUG.phase") == "awaiting": break
            page.wait_for_timeout(30)
        pick_legal()
        dl = time.time() + 3
        while time.time() < dl:
            s = page.evaluate("({r: window.LUG.game.players[0].tokens[0].pos, ph: window.LUG.phase})")
            if s["r"] == 5 and s["ph"] != "moving": break
            page.wait_for_timeout(30)
        st = page.evaluate("""(() => ({
          red: window.LUG.game.players[0].tokens[0].pos,
          yellow: window.LUG.game.players[2].tokens[0].pos}))()""")
        check("play/capture2P", st["red"] == 5 and st["yellow"] == -1, json.dumps(st))

        # safe square: global 8 star. red rel 7 +1 → 8. yellow rel (8-26+52)%52=34.
        page.evaluate("""(() => {
          window.LUG.clock++; window.luCancelDiceSpin(); window.luCancelAnim();
          const g = window.LUG.game;
          g.winner = null; g.turn = 0; g.chain = 0;
          g.players.forEach(P => P.tokens.forEach(t => { t.pos = -1; delete t._viz; }));
          g.players[0].tokens[0].pos = 7;
          g.players[2].tokens[0].pos = 34;
          window.LUG.phase = 'idle'; window.LUG.dice = null; window.LUG.moves = [];
          window.LUG.anim = null; window.LUG.settleUntil = 0; window.LUG.nudge = null;
          window.LUD.force(1);
        })()""")
        dl = time.time() + 3
        while time.time() < dl:
            if page.evaluate("window.LUG.phase") == "awaiting": break
            page.wait_for_timeout(30)
        pick_legal()
        dl = time.time() + 3
        while time.time() < dl:
            s = page.evaluate("({r: window.LUG.game.players[0].tokens[0].pos, ph: window.LUG.phase})")
            if s["r"] == 8 and s["ph"] != "moving": break
            page.wait_for_timeout(30)
        st = page.evaluate("""(() => ({
          red: window.LUG.game.players[0].tokens[0].pos,
          yellow: window.LUG.game.players[2].tokens[0].pos}))()""")
        check("play/safeSquare2P", st["red"] == 8 and st["yellow"] == 34, json.dumps(st))

        # ---------- 8: winner detection in 2P ----------
        page.evaluate("""(() => {
          window.LUG.clock++; window.luCancelDiceSpin(); window.luCancelAnim();
          const g = window.LUG.game;
          g.winner = null; g.turn = 0; g.chain = 0;
          g.players.forEach(P => { P.tokens.forEach(t => { t.pos = -1; delete t._viz; }); P.home = 0; });
          g.players[0].tokens.forEach(t => t.pos = 56);
          g.players[0].home = 4;
          window.LudoEngine.checkWinner(g);
          window.LUG.phase = 'over';
        })()""")
        w = page.evaluate("window.LUG.game.winner")
        check("play/winner2P", w == 0, f"winner={w}")

        # ---------- 9: back-nav & re-entry keep the choice ----------
        open_lobby(); page.click("#luSeatPick [data-seats='3']")
        start_game()
        page.wait_for_timeout(500)
        page.evaluate("location.hash = '#funtime'")
        page.wait_for_timeout(400)
        page.evaluate("location.hash = '#funtime/lu'")
        page.wait_for_selector("#luBoard")
        page.wait_for_timeout(600)
        s = sel_state()
        check("ui/navReentryKeeps3P", s["count"] == 3 and s["seats"] == [0, 1, 2], json.dumps(s))

        # ---------- 10: churn → no stale listeners/timers ----------
        for _ in range(4):
            open_lobby(); page.click("#luSeatPick [data-seats='2']")
            start_game()
            page.wait_for_timeout(250)
            open_lobby(); page.click("#luSeatPick [data-seats='4']")
            start_game()
            page.wait_for_timeout(250)
        page.wait_for_timeout(800)
        st = page.evaluate("""(() => ({
      diceRaf: window.LUG.diceRaf, anim: window.LUG.anim === null,
      nudge: window.LUG.nudge === null,
      settleOpen: window.LUG.settleUntil > performance.now(),
      phase: window.LUG.phase,
      seats: window.LudoEngine.activeSeats(window.LUG.game)}))()""")
        r7 = page.evaluate("""(() => {
          let calls = 0;
          const orig = window.luResize;
          window.luResize = function() { calls++; return orig.apply(this, arguments); };
          window.dispatchEvent(new Event('resize'));
          window.luResize = orig;
          return calls;
        })()""")
        check("life/noStaleTimersAfterChurn",
              st["diceRaf"] == 0 and st["anim"] and st["nudge"] and not st["settleOpen"],
              json.dumps(st))
        check("life/singleResizeListener", r7 == 1, f"calls={r7}")
        check("browser/noConsoleErrors", not errors, errors[:3])

        browser.close()
    print(f"\n==== {PASS}/{PASS + FAIL} passed ====")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
