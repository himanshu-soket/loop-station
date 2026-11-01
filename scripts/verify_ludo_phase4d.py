#!/usr/bin/env python3
"""Phase 4.4 — Medium AI difficulty.

Covers: capture preference when beneficial; safe-square preference; home
progress (depth) preference; yard release on six; exposure avoidance;
multiple legal moves; deterministic behavior with fixed seeds; controlled
randomness for near-equal scores; every selected move is legal; empty move
list returns null; Classic unchanged; Easy unchanged; human turns manual;
multiple AI seats; restart preserves Medium; back-navigation re-entry; no
duplicate AI timers or rolls; Hard enabled (Phase 4.5).

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

        def set_cfg(n, h, diff=None):
            # Phase 5.5: selectors live in the setup lobby; apply with a
            # Start so the configuration reaches a live game (JS .click()
            # on the elements — coordinate clicks race layout shifts).
            if not page.evaluate("window.LUG.lobby"):
                page.evaluate("window.luLobbyShow()")
                page.wait_for_timeout(300)
            sel_seat = json.dumps(f"#luSeatPick [data-seats='{n}']")
            page.evaluate(f"document.querySelector({sel_seat}).click()")
            page.wait_for_timeout(250)
            sel_hum = json.dumps(f"#luHumanPick [data-humans='{h}']")
            page.evaluate(f"document.querySelector({sel_hum}).click()")
            page.wait_for_timeout(250)
            if diff is not None:
                dd = "" if diff == "" else diff
                sel_d = json.dumps(f"#luDiffPick [data-diff='{dd}']")
                page.evaluate(f"document.querySelector({sel_d}).click()")
                page.wait_for_timeout(350)
            page.evaluate("window.luLobbyStart()")
            page.wait_for_timeout(400)

        # ---------- 1: engine-level Medium strategy ----------
        r = page.evaluate("""(() => {
          const E = LudoEngine, W = E.MEDIUM_WEIGHTS, out = {};
          // -- capture preferred over ordinary movement --
          let g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 10;
          g.players[1].tokens[1].pos = 20;
          g.players[0].tokens[0].pos = 38;      // victim at green's 25-landing
          let mv = E.legalMoves(g, 1, 5);       // 20→25 captures red
          let pick = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(1));
          out.capture = {victims: E.victimsAt(g,1,25).length,
                         pick: pick.from + '->' + pick.to};
          // -- safe-square landing preferred (no capture available) --
          g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 12;      // +1 → 13 star
          g.players[1].tokens[1].pos = 45;      // +1 → 46 plain
          mv = E.legalMoves(g, 1, 1);
          pick = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(1));
          out.safe = {aSafe: E.isSafeStep(1,13), bSafe: E.isSafeStep(1,46),
                      pick: pick.from + '->' + pick.to};
          // -- home progress: deeper runner advances (plain moves only) --
          // (scores read from a memo-free twin so the diversity term —
          //  which keys off the state's last-move memo — cannot pollute
          //  the raw comparison)
          g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 30;      // →32 plain
          g.players[1].tokens[1].pos = 44;      // →46 plain
          mv = E.legalMoves(g, 1, 2);
          const twin = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          twin.players[1].tokens[0].pos = 30;
          twin.players[1].tokens[1].pos = 44;
          const tmv = E.legalMoves(twin, 1, 2);
          pick = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(1));
          out.progress = {s32: E.scoreMoveMedium(twin,1,tmv[0],W),
                          s46: E.scoreMoveMedium(twin,1,tmv[1],W),
                          pick: pick.from + '->' + pick.to};
          // -- yard release on six --
          g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 30;
          mv = E.legalMoves(g, 1, 6);
          pick = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(1));
          out.release = {pick: pick.from + '->' + pick.to};
          // -- exposure avoidance: star→danger vs path to star --
          g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 13;      // star; +4 → 17 threatened
          g.players[1].tokens[1].pos = 22;      // +4 → 26 star
          g.players[0].tokens[0].pos = 29;      // threatens green's 17
          mv = E.legalMoves(g, 1, 4);
          pick = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(1));
          out.exposure = {pick: pick.from + '->' + pick.to};
          // -- multiple legal moves: all scored, best wins --
          g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 5;
          g.players[1].tokens[1].pos = 15;
          g.players[1].tokens[2].pos = 25;
          g.players[1].tokens[3].pos = 35;
          mv = E.legalMoves(g, 1, 3);
          out.multiCount = mv.length;
          pick = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(1));
          out.multiLegal = mv.some(m => m.token === pick.token
                                   && m.from === pick.from && m.to === pick.to);
          return out;
        })()""")
        check("engine/capturePreferred", r["capture"]["pick"] == "20->25",
              json.dumps(r["capture"]))
        check("engine/safeSquarePreferred", r["safe"]["pick"] == "12->13",
              json.dumps(r["safe"]))
        check("engine/homeProgressPreferred",
              r["progress"]["pick"] == "44->46" and r["progress"]["s46"] > r["progress"]["s32"],
              json.dumps(r["progress"]))
        check("engine/yardReleaseOnSix", r["release"]["pick"] == "-1->0",
              json.dumps(r["release"]))
        check("engine/exposureAvoided", r["exposure"]["pick"] == "22->26",
              json.dumps(r["exposure"]))
        check("engine/multipleLegalMovesHandled",
              r["multiCount"] >= 3 and r["multiLegal"],
              f"moves={r['multiCount']} legal={r['multiLegal']}")

        # determinism / randomness / legality / router
        r2 = page.evaluate("""(() => {
          const E = LudoEngine, out = {};
          // deterministic: same seed, record=false → identical picks
          let bad = [];
          for (let s = 1; s <= 25; s++) {
            const gg = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
            gg.players[1].tokens[0].pos = 10 + (s % 20);
            gg.players[1].tokens[1].pos = 30 + (s % 15);
            gg.players[1].tokens[2].pos = 45;
            const mm = E.legalMoves(gg, 1, 3);
            const a = JSON.stringify(E.selectAIMoveMedium(gg,1,mm,E.makeRng(s),false));
            const c = JSON.stringify(E.selectAIMoveMedium(gg,1,mm,E.makeRng(s),false));
            if (a !== c) bad.push(s);
          }
          out.detBad = bad;
          // controlled randomness: near-equal scores split across seeds
          let g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 30;   // →33 score 42
          g.players[1].tokens[1].pos = 33;   // →36 score 45  (gap 3 < jitter 12)
          let mv = E.legalMoves(g, 1, 3);
          const counts = {};
          for (let s = 1; s <= 60; s++) {
            const seed = (s * 2654435761) >>> 0;
            const q = E.selectAIMoveForDifficulty(g, 1, mv, 'medium', E.makeRng(seed));
            counts[q.from] = (counts[q.from] || 0) + 1;
          }
          out.nearEqual = counts;
          // every pick legal over randomized boards
          let allLegal = true, n = 0;
          for (let s = 1; s <= 120; s++) {
            const gg = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
            for (let t = 0; t < 4; t++)
              gg.players[1].tokens[t].pos = [-1, 0, 7, 19, 28, 41, 50][(s + t*3) % 7];
            gg.players[0].tokens[0].pos = (s * 11) % 52;
            const dice = 1 + (s % 6);
            const mm = E.legalMoves(gg, 1, dice);
            const q = E.selectAIMoveForDifficulty(gg, 1, mm, 'medium', E.makeRng(s));
            n++;
            if (q && !mm.some(x => x.token === q.token && x.from === q.from && x.to === q.to))
              allLegal = false;
          }
          out.allLegal = {ok: allLegal, n};
          // empty list → null
          out.emptyNull = E.selectAIMoveForDifficulty(g, 1, [], 'medium') === null;
          // Classic & Easy unchanged through the router
          g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          g.players[1].tokens[0].pos = 30;
          g.players[1].tokens[1].pos = 33;
          mv = E.legalMoves(g, 1, 3);
          const legacy = E.selectAIMoveForDifficulty(g, 1, mv, null, E.makeRng(3));
          const easy = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(3));
          out.router = {legacy: legacy !== null, easy: easy !== null};
          // Easy's spread still random (not the Medium scorer's pick logic)
          const ec = {};
          for (let s = 1; s <= 40; s++) {
            const seed = (s * 2654435761) >>> 0;
            const q = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(seed));
            ec[q.from] = (ec[q.from] || 0) + 1;
          }
          out.easySpread = Object.keys(ec).length;
          // diversity: recorded memo steers away from repeating
          const first = E.selectAIMoveMedium(g,1,mv,()=>0.99,true);
          const second = E.selectAIMoveMedium(g,1,mv,()=>0.99,true);
          out.diversity = {first: first.token, second: second.token,
                           avoided: first.token !== second.token};
          return out;
        })()""")
        check("engine/deterministicFixedSeed", not r2["detBad"], json.dumps(r2["detBad"][:5]))
        nk = list(r2["nearEqual"].keys())
        check("engine/controlledRandomnessNearEqual",
              len(nk) == 2 and min(r2["nearEqual"].values()) >= 10,
              json.dumps(r2["nearEqual"]))
        check("engine/everyPickLegal", r2["allLegal"]["ok"], json.dumps(r2["allLegal"]))
        check("engine/emptyListNull", r2["emptyNull"])
        check("engine/classicUnchanged", r2["router"]["legacy"])
        check("engine/easyUnchanged",
              r2["router"]["easy"] and r2["easySpread"] >= 2,
              f"spread={r2['easySpread']}")
        check("engine/strategicDiversity", r2["diversity"]["avoided"],
              json.dumps(r2["diversity"]))

        # weights are exposed and documented shape
        w = page.evaluate("LudoEngine.MEDIUM_WEIGHTS")
        expected = {"finish": 800, "capture": 220, "captureStep": 2, "release": 200,
                    "safeLanding": 120, "homeStretch": 40, "progress": 3, "depth": 1,
                    "exposure": -90, "dangerAt": -60, "diversity": -35, "jitter": 12}
        check("engine/weightsExposed", w == expected,
              "diff=" + json.dumps({k: v for k, v in expected.items() if w.get(k) != v}))

        # ---------- 2: UI states ----------
        ui = page.evaluate("""(() => ({
          btns: [...document.querySelectorAll('#luDiffPick .lu-sp-btn')]
            .map(b => ({d: b.dataset.diff, dis: b.disabled,
                        on: b.classList.contains('on'),
                        soon: b.classList.contains('lu-soon')})),
          hardTitle: document.querySelector('#luDiffPick [data-diff=\\'hard\\']').title}))()""")
        d = {b["d"]: b for b in ui["btns"]}
        check("ui/mediumEnabled", not d["medium"]["dis"] and not d["medium"]["soon"])
        # Phase 4.5 landed: Hard is now a real, enabled difficulty. The
        # old "coming soon" placeholder contract is superseded.
        check("ui/hardNowEnabled", not d["hard"]["dis"] and not d["hard"]["soon"]
              and "Coming soon" not in ui["hardTitle"], ui["hardTitle"])
        check("ui/classicStillDefault", d[""]["on"])

        # ---------- 3: select Medium; applies via fresh game ----------
        set_cfg(4, 1, "medium")
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
          kinds: window.LUG.game.players.map(P => P.kind)}))()""")
        check("apply/mediumSelected", s["d"] == "medium"
              and s["kinds"] == ["human", "ai", "ai", "ai"], json.dumps(s))
        blurb = page.evaluate("document.getElementById('luBlurb').textContent")
        check("apply/blurbShowsLevel", "AI level: Medium" in blurb, blurb)

        # ---------- 4: live game — Medium AI plays legally ----------
        def drive_red_and_wait(want, timeout=12, distinct_seats=False, stage_ai=()):
            page.evaluate("""(() => {
              window.__pickLog = [];
              const orig = LudoEngine.selectAIMoveForDifficulty;
              LudoEngine.selectAIMoveForDifficulty = function(g, p, moves, diff, rand) {
                const q = orig.call(this, g, p, moves, diff, rand);
                window.__pickLog.push({player: p, diff,
                  legal: moves.map(m => m.token + ':' + m.from + '-' + m.to),
                  picked: q ? (q.token + ':' + q.from + '-' + q.to) : null});
                return q;
              };
            })()""")
            # Wait for a clean human-idle state before staging: anything
            # left in flight by earlier steps (animation, settle) must
            # finish first, else force() is rightly rejected.
            dl = time.time() + 5
            while time.time() < dl:
                st = page.evaluate("({ph: window.LUG.phase, t: window.LUG.game.turn})")
                if st["ph"] == "idle" and st["t"] == 0:
                    break
                page.wait_for_timeout(50)
            page.evaluate("""(() => {
              window.LUG.clock++; window.luCancelDiceSpin(); window.luCancelAnim();
              const g = window.LUG.game;
              g.players[0].tokens[0].pos = 10;
              for (const s of %s) g.players[s].tokens[0].pos = 8 + s * 7;
              window.LUD.force(3);
            })()""" % json.dumps(list(stage_ai)))
            dl = time.time() + 5          # generous: roll resolve can lag
            while time.time() < dl and page.evaluate("window.LUG.phase") != "awaiting":
                page.wait_for_timeout(30)
            # commit red's move through the REAL pick path (the same
            # function the canvas click handler calls; it re-validates
            # against the live engine) — no mouse-coordinate flake
            picked = page.evaluate("""(() => {
              if (window.LUG.phase !== 'awaiting') return 'not-awaiting';
              if (!window.LUG.moves || !window.LUG.moves.length) return 'no-moves';
              const t = window.LUG.game.turn;
              const tok = window.LUG.moves[0].token;
              window.luPick({p: t, i: tok});
              return 'ok';
            })()""")
            if picked != "ok": return None
            deadline = time.time() + timeout
            while time.time() < deadline:
                page.wait_for_timeout(200)
                log = page.evaluate("window.__pickLog || []")
                if distinct_seats:
                    if len(set(e["player"] for e in log)) >= want:
                        return log
                elif len(log) >= want:
                    return log
            return page.evaluate("window.__pickLog || []")

        log = None
        for _ in range(3):                       # timing-flake retries
            log = drive_red_and_wait(2, timeout=8)
            if log and len(log) >= 2:
                break
        check("live/mediumPicksFlow", log is not None and len(log) >= 2,
              f"picks={0 if not log else len(log)}")
        if log:
            illegal = [e for e in log if e["picked"] and e["picked"] not in e["legal"]]
            wrong = [e for e in log if e["diff"] != "medium"]
            check("live/mediumPicksOnlyLegal", not illegal, f"illegal={len(illegal)}")
            check("live/difficultyFlowsThrough", not wrong, f"non-medium={len(wrong)}")
        else:
            check("live/mediumPicksOnlyLegal", False, "no picks captured")
            check("live/difficultyFlowsThrough", False, "no picks captured")

        # ---------- 5: human turn stays manual under Medium ----------
        page.click("#luRestartBtn"); start_game()
        page.wait_for_timeout(300)
        auto = 0
        t0 = time.time()
        while time.time() - t0 < 2.5:
            a = page.evaluate("window.LUG.game.rolls")
            page.wait_for_timeout(200)
            b = page.evaluate("window.LUG.game.rolls")
            if b > a: auto += 1
        check("live/humanTurnManual", auto == 0, f"auto={auto}")

        # ---------- 6: multiple AI seats ----------
        set_cfg(4, 1, "medium")
        page.wait_for_timeout(400)
        log = drive_red_and_wait(2, timeout=20, distinct_seats=True, stage_ai=(1, 2, 3))
        seats_seen = sorted(set(e["player"] for e in (log or [])))
        check("live/multipleAISeatsUseMedium",
              len(seats_seen) >= 2 and all(x in (1, 2, 3) for x in seats_seen),
              f"seats={seats_seen} n={len(log or [])}")

        # ---------- 7: restart preserves Medium ----------
        page.click("#luRestartBtn"); start_game()
        page.wait_for_timeout(500)
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
             n: window.LUG.seatCount, h: window.LUG.humanSeats,
             kinds: window.LUG.game.players.map(P => P.kind),
             rngFresh: window.LUG.aiRng !== null}))()""")
        check("restart/keepsMedium",
              s["d"] == "medium" and s["n"] == 4 and s["h"] == 1
              and s["kinds"] == ["human", "ai", "ai", "ai"] and s["rngFresh"],
              json.dumps(s))

        # ---------- 8: back-nav & re-entry ----------
        page.evaluate("location.hash = '#funtime'")
        page.wait_for_timeout(400)
        page.evaluate("location.hash = '#funtime/lu'")
        page.wait_for_selector("#luBoard")
        page.wait_for_timeout(600)
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
             n: window.LUG.seatCount, h: window.LUG.humanSeats}))()""")
        check("nav/reentryKeepsMedium", s["d"] == "medium" and s["n"] == 4 and s["h"] == 1,
              json.dumps(s))
        on = page.evaluate("""[...document.querySelectorAll('#luDiffPick .lu-sp-btn')]
             .filter(b => b.classList.contains('on')).map(b => b.dataset.diff)""")
        check("nav/selectorStateRestored", on == ["medium"], json.dumps(on))

        # ---------- 9: switching to Easy and back ----------
        open_lobby(); page.click("#luDiffPick [data-diff='easy']")
        start_game()
        page.wait_for_timeout(400)
        e1 = page.evaluate("window.LUG.aiDifficulty")
        open_lobby(); page.click("#luDiffPick [data-diff='medium']")
        start_game()
        page.wait_for_timeout(400)
        e2 = page.evaluate("window.LUG.aiDifficulty")
        check("switch/easyAndBack", e1 == "easy" and e2 == "medium", f"{e1}→{e2}")

        # ---------- 10: human-only game unaffected ----------
        set_cfg(4, 4, "medium")
        page.wait_for_timeout(400)
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
          anyAI: window.LUG.game.players.some(P => P.active && P.kind === 'ai')}))()""")
        check("humanOnly/unaffected", s["d"] == "medium" and not s["anyAI"],
              json.dumps(s))
        auto = 0
        t0 = time.time()
        while time.time() - t0 < 2:
            a = page.evaluate("window.LUG.game.rolls")
            page.wait_for_timeout(200)
            b = page.evaluate("window.LUG.game.rolls")
            if b > a: auto += 1
        check("humanOnly/noAutoRolls", auto == 0, f"auto={auto}")

        # ---------- 11: no duplicate AI timers/rolls ----------
        set_cfg(4, 1, "medium")
        page.wait_for_timeout(300)
        page.evaluate("""(() => {
          window.__aiRollCount = 0;
          const orig = window.luAiRoll;
          window.luAiRoll = function() { window.__aiRollCount++; return orig.apply(this, arguments); };
        })()""")
        page.wait_for_timeout(4000)
        calls = page.evaluate("window.__aiRollCount")
        rolls = page.evaluate("window.LUG.game.rolls")
        check("timers/noDuplicateAIRolls", calls <= rolls + 1, f"calls={calls} rolls={rolls}")
        check("browser/noConsoleErrors", not errors, errors[:3])

        browser.close()
    print(f"\n==== {PASS}/{PASS + FAIL} passed ====")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
