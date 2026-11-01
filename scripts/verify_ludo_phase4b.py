#!/usr/bin/env python3
"""Phase 4.2 — Human + AI seat configuration.

Covers every valid combination for 2P/3P/4P:
  2P: 1H+1AI, 2H          3P: 1H+2AI, 2H+1AI, 3H
  4P: 1H+3AI, 2H+2AI, 3H+1AI, 4H

Checks: seat-kind assignment, inactive seats, turn ownership,
human→human and human→AI transitions, consecutive AI turns, input
rejection during AI turns, restart preservation, back-nav re-entry,
rapid-input timer cleanup, no duplicate AI rolls.

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

        def set_cfg(n, h):
            """Phase 5.5: selectors live in the setup lobby. Open it,
            click both, then Start to apply the configuration."""
            open_lobby()
            page.click(f"#luSeatPick [data-seats='{n}']")
            page.wait_for_timeout(300)
            page.click(f"#luHumanPick [data-humans='{h}']")
            page.wait_for_timeout(300)
            start_game()
            page.wait_for_timeout(300)

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

        def stage_nonsix(pos=10, dice=3):
            """Stage red on track with a non-six roll so the turn MUST pass."""
            page.evaluate("""(() => {
              window.LUG.clock++; window.luCancelDiceSpin(); window.luCancelAnim();
              const g = window.LUG.game;
              g.players[0].tokens[0].pos = %d;
              window.LUD.force(%d);
            })()""" % (pos, dice))
            dl = time.time() + 2
            while time.time() < dl and page.evaluate("window.LUG.phase") != "awaiting":
                page.wait_for_timeout(30)

        # ---------- 1: every valid combination assigns kinds correctly ----------
        r = page.evaluate("""(() => {
          const out = {};
          const combos = [[2,1],[2,2],[3,1],[3,2],[3,3],[4,1],[4,2],[4,3],[4,4]];
          for (const [n, h] of combos) {
            const kinds = luKindsForConfig(n, h);
            const seats = {2:[0,2],3:[0,1,2],4:[0,1,2,3]}[n];
            const g = LudoEngine.createGame(kinds, {seats});
            out[`${n}p${h}h`] = {
              activeKinds: seats.map(s => g.players[s].kind),
              inactiveKinds: [0,1,2,3].filter(i => !seats.includes(i))
                                       .map(i => g.players[i].kind),
              flags: g.players.map(P => P.active)};
          }
          out.clampLow = luKindsForConfig(3, 0);
          out.clampHigh = luKindsForConfig(2, 5);
          out.clampNaN = luKindsForConfig(4, 'junk');
          return out;
        })()""")
        expect = {
          "2p1h": ["human", "ai"],           "2p2h": ["human", "human"],
          "3p1h": ["human", "ai", "ai"],     "3p2h": ["human", "human", "ai"],
          "3p3h": ["human", "human", "human"],
          "4p1h": ["human", "ai", "ai", "ai"],
          "4p2h": ["human", "human", "ai", "ai"],
          "4p3h": ["human", "human", "human", "ai"],
          "4p4h": ["human", "human", "human", "human"],
        }
        for k, v in expect.items():
            check(f"config/{k}", r[k]["activeKinds"] == v, json.dumps(r[k]["activeKinds"]))
        all_ok = all(all(kk != "human" for kk in r[k]["inactiveKinds"]) for k in expect)
        check("config/inactiveNeverHuman", all_ok,
              json.dumps({k: r[k]["inactiveKinds"] for k in expect}))
        check("config/clampLow", r["clampLow"][0] == "human" and r["clampLow"][1] == "ai", json.dumps(r["clampLow"]))
        check("config/clampHigh", r["clampHigh"][0] == "human" and r["clampHigh"][2] == "human", json.dumps(r["clampHigh"]))
        check("config/clampNaN", r["clampNaN"][0] == "human" and r["clampNaN"][1] == "ai", json.dumps(r["clampNaN"]))

        # ---------- 2: UI selector states per player count ----------
        set_cfg(2, 1)
        s = page.evaluate("""(() => ({
          n: window.LUG.seatCount, h: window.LUG.humanSeats,
          on: [...document.querySelectorAll('#luHumanPick .lu-sp-btn')]
               .filter(b => b.classList.contains('on')).map(b => b.dataset.humans),
          hidden: [...document.querySelectorAll('#luHumanPick .lu-sp-btn')]
               .filter(b => b.hidden).map(b => b.dataset.humans)}))()""")
        check("ui/2Poffers1H2H", s["on"] == ["1"] and sorted(s["hidden"]) == ["3", "4"], json.dumps(s))
        set_cfg(3, 2)
        s = page.evaluate("""(() => ({
          on: [...document.querySelectorAll('#luHumanPick .lu-sp-btn')]
               .filter(b => b.classList.contains('on')).map(b => b.dataset.humans),
          hidden: [...document.querySelectorAll('#luHumanPick .lu-sp-btn')]
               .filter(b => b.hidden).map(b => b.dataset.humans),
          kinds: window.LUG.game.players.map(P => P.kind)}))()""")
        check("ui/3Poffers1H2H3H", s["on"] == ["2"] and s["hidden"] == ["4"], json.dumps(s))
        check("ui/3P2Hkinds", s["kinds"] == ["human", "human", "ai", "ai"], json.dumps(s["kinds"]))
        set_cfg(4, 3)
        s = page.evaluate("""(() => ({
          on: [...document.querySelectorAll('#luHumanPick .lu-sp-btn')]
               .filter(b => b.classList.contains('on')).map(b => b.dataset.humans),
          hidden: [...document.querySelectorAll('#luHumanPick .lu-sp-btn')]
               .filter(b => b.hidden).map(b => b.dataset.humans),
          kinds: window.LUG.game.players.map(P => P.kind)}))()""")
        check("ui/4PoffersAll", s["on"] == ["3"] and s["hidden"] == [], json.dumps(s))
        check("ui/4P3Hkinds", s["kinds"] == ["human", "human", "human", "ai"], json.dumps(s["kinds"]))

        # blurb names each seat's controller
        blurb = page.evaluate("document.getElementById('luBlurb').textContent")
        # Phase 5.5: display names — first human is "You", others
        # "Player N"; engine colour names remain on the board only.
        check("ui/blurbShowsControllers",
              "You (human)" in blurb and "Blue (AI)" in blurb, blurb)

        # ---------- 3: all-human hotseat needs manual rolls ----------
        set_cfg(4, 4)
        page.wait_for_timeout(400)
        auto = 0
        t0 = time.time()
        while time.time() - t0 < 3:
            a = page.evaluate("window.LUG.game.rolls")
            page.wait_for_timeout(200)
            b = page.evaluate("window.LUG.game.rolls")
            if b > a: auto += 1
        check("turn/allHumanNoAutoRoll", auto == 0, f"auto={auto}")

        # human→human handoff via non-six
        stage_nonsix()
        pick_legal()
        dl = time.time() + 4
        while time.time() < dl:
            s = page.evaluate("({t: window.LUG.game.turn, ph: window.LUG.phase})")
            if s["t"] == 1 and s["ph"] == "idle": break
            page.wait_for_timeout(40)
        s = page.evaluate("""(() => ({t: window.LUG.game.turn,
             k: window.LUG.game.players[window.LUG.game.turn].kind,
             btn: document.getElementById('luRollBtn').disabled}))()""")
        check("turn/humanToHuman", s["t"] == 1 and s["k"] == "human" and not s["btn"], json.dumps(s))

        # ---------- 4: human→AI transition + input ignored during AI ----------
        set_cfg(2, 1)
        page.wait_for_timeout(400)
        stage_nonsix()
        pick_legal()
        dl = time.time() + 6
        while time.time() < dl:
            s = page.evaluate("({t: window.LUG.game.turn, k: window.LUG.game.players[window.LUG.game.turn].kind})")
            if s["t"] == 2 and s["k"] == "ai": break
            page.wait_for_timeout(40)
        check("turn/humanToAI", s["t"] == 2 and s["k"] == "ai", json.dumps(s))
        # Hammer the roll button (force-enabled) while it is YELLOW'S AI
        # turn. The click handler must reject every one: a roll can only
        # start from the AI's own scheduled timer. Assert no roll was ever
        # attributed to a human click while an AI seat held the turn.
        # Deterministic check: dispatch 10 roll-button clicks SYNCHRONOUSLY
        # in one JS tick while Yellow (AI) holds the turn at phase 'idle'.
        # The handler's kind guard must reject every one of them — no roll
        # may start from a human click during an AI turn.
        page.wait_for_timeout(100)
        res = page.evaluate("""(() => {
          const g = window.LUG.game;
          if (!g || g.players[g.turn].kind !== 'ai') return {skip: true};
          // freeze the clock so the AI's own timer cannot fire mid-test
          const savedClock = window.LUG.clock;
          let rollsBefore = g.rolls;
          let phaseChanges = 0;
          const origSpin = window.luDiceSpin;
          window.luDiceSpin = () => { phaseChanges++; };   // roll-start probe
          for (let k = 0; k < 10; k++) {
            const b = document.getElementById('luRollBtn');
            b.disabled = false;
            b.dispatchEvent(new MouseEvent('click', {bubbles: true}));
          }
          window.luDiceSpin = origSpin;
          return {skip: false, owner: g.players[g.turn].kind,
                  rollsBefore, rollsAfter: g.rolls,
                  rollStarts: phaseChanges,
                  phase: window.LUG.phase};
        })()""")
        if res.get("skip"):
            # AI turn not at idle right now — drive red once more and retry
            stage_nonsix()
            pick_legal()
            dl = time.time() + 6
            while time.time() < dl:
                s2 = page.evaluate("""({t: window.LUG.game.turn, ph: window.LUG.phase,
                     k: window.LUG.game.players[window.LUG.game.turn].kind})""")
                if s2["t"] == 2 and s2["k"] == "ai" and s2["ph"] == "idle": break
                page.wait_for_timeout(30)
            res = page.evaluate("""(() => {
              const g = window.LUG.game;
              if (!g || g.players[g.turn].kind !== 'ai') return {skip: true};
              let phaseChanges = 0;
              const origSpin = window.luDiceSpin;
              window.luDiceSpin = () => { phaseChanges++; };
              for (let k = 0; k < 10; k++) {
                const b = document.getElementById('luRollBtn');
                b.disabled = false;
                b.dispatchEvent(new MouseEvent('click', {bubbles: true}));
              }
              window.luDiceSpin = origSpin;
              return {skip: false, owner: g.players[g.turn].kind,
                      rollsAfter: g.rolls, rollStarts: phaseChanges,
                      phase: window.LUG.phase};
            })()""")
        check("turn/inputIgnoredDuringAI",
              res.get("skip") is False and res.get("rollStarts") == 0
              and res.get("owner") == "ai",
              json.dumps(res))

        # ---------- 5: consecutive AI turns (4P 1H) ----------
        set_cfg(4, 1)
        page.wait_for_timeout(400)
        stage_nonsix()
        pick_legal()
        seq = []
        t0 = time.time()
        while time.time() - t0 < 15:
            s = page.evaluate("""(() => ({t: window.LUG.game.turn,
                 k: window.LUG.game.players[window.LUG.game.turn].kind}))()""")
            if not seq or seq[-1][0] != s["t"]:
                seq.append((s["t"], s["k"]))
            if len(seq) >= 4: break
            page.wait_for_timeout(100)
        kinds_seq = [k for _, k in seq]
        check("turn/consecutiveAI", kinds_seq == ["human", "ai", "ai", "ai"], json.dumps(seq))

        # ---------- 6: no duplicate AI rolls (instrumented) ----------
        page.click("#luRestartBtn"); start_game()
        page.wait_for_timeout(300)
        page.evaluate("""(() => {
          window.__aiRollCount = 0;
          const orig = window.luAiRoll;
          window.luAiRoll = function() { window.__aiRollCount++; return orig.apply(this, arguments); };
        })()""")
        page.wait_for_timeout(4000)     # several AI turns at fast pacing
        calls = page.evaluate("window.__aiRollCount")
        rolls = page.evaluate("window.LUG.game.rolls")
        check("timers/noDuplicateAIRolls", calls <= rolls + 1, f"calls={calls} rolls={rolls}")

        # ---------- 7: restart preserves config ----------
        set_cfg(3, 2)
        page.wait_for_timeout(400)
        page.click("#luRestartBtn"); start_game()
        page.wait_for_timeout(500)
        s = page.evaluate("""(() => ({
          n: window.LUG.seatCount, h: window.LUG.humanSeats,
          kinds: window.LUG.game.players.map(P => P.kind),
          flags: window.LUG.game.players.map(P => P.active)}))()""")
        check("restart/keeps3P2H",
              s["n"] == 3 and s["h"] == 2
              and s["kinds"] == ["human", "human", "ai", "ai"]
              and s["flags"] == [True, True, True, False], json.dumps(s))

        # ---------- 8: back-nav & re-entry ----------
        page.evaluate("location.hash = '#funtime'")
        page.wait_for_timeout(400)
        page.evaluate("location.hash = '#funtime/lu'")
        page.wait_for_selector("#luBoard")
        page.wait_for_timeout(600)
        s = page.evaluate("""(() => ({
          n: window.LUG.seatCount, h: window.LUG.humanSeats,
          kinds: window.LUG.game.players.map(P => P.kind)}))()""")
        check("nav/reentryKeeps3P2H",
              s["n"] == 3 and s["h"] == 2 and s["kinds"][1] == "human", json.dumps(s))

        # ---------- 9: rapid input + timer cleanup ----------
        open_lobby()
        for n, h in [(2, 2), (4, 1), (3, 3), (2, 1), (4, 4)]:
            page.click(f"#luSeatPick [data-seats='{n}']")
            page.wait_for_timeout(150)
            page.click(f"#luHumanPick [data-humans='{h}']")
            page.wait_for_timeout(150)
        start_game()
        # hammer the roll button mid-configuration
        for _ in range(5):
            page.evaluate("""(() => {
              const b = document.getElementById('luRollBtn');
              b.disabled = false;
              b.dispatchEvent(new MouseEvent('click', {bubbles: true}));
            })()""")
        page.wait_for_timeout(1000)
        st = page.evaluate("""(() => ({
      diceRaf: window.LUG.diceRaf, anim: window.LUG.anim === null,
      nudge: window.LUG.nudge === null,
      settleOpen: window.LUG.settleUntil > performance.now(),
      phase: window.LUG.phase,
      n: window.LUG.seatCount, h: window.LUG.humanSeats,
      kinds: window.LUG.game.players.map(P => P.kind)}))()""")
        r7 = page.evaluate("""(() => {
          let calls = 0;
          const orig = window.luResize;
          window.luResize = function() { calls++; return orig.apply(this, arguments); };
          window.dispatchEvent(new Event('resize'));
          window.luResize = orig;
          return calls;
        })()""")
        check("life/rapidInputCleanup",
              st["diceRaf"] == 0 and st["anim"] and st["nudge"] and not st["settleOpen"],
              json.dumps(st))
        check("life/singleResizeListener", r7 == 1, f"calls={r7}")
        check("life/finalConfigIs4P4H",
              st["n"] == 4 and st["h"] == 4
              and st["kinds"] == ["human", "human", "human", "human"],
              json.dumps({"n": st["n"], "h": st["h"], "kinds": st["kinds"]}))
        check("browser/noConsoleErrors", not errors, errors[:3])

        browser.close()
    print(f"\n==== {PASS}/{PASS + FAIL} passed ====")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
