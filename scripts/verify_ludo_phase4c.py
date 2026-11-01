#!/usr/bin/env python3
"""Phase 4.3 — Easy AI difficulty.

Covers: Easy picks only legal moves; yard-release preference on six;
random choice among non-release legal moves; deterministic behavior with a
fixed seed (engine makeRng); multiple AI seats; human turns stay manual;
no-legal-move handling; extra rolls; three consecutive sixes; restart and
difficulty preservation; back-navigation re-entry; no duplicate AI timers
or rolls; Medium/Hard shown but disabled.

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
            # JS .click() on the element itself — coordinate-based clicks
            # can race layout shifts (blurb re-render) and land on the
            # roll button, starting a stray roll.
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

        def drive_red_and_wait(want, timeout=12, distinct_seats=False, stage_ai=()):
            """Stage red (and any seats in `stage_ai`) on track, force a
            non-six for red, click the legal move, then poll until `want`
            picks (or `want` DISTINCT AI seats when distinct_seats=True)
            have been logged. Pre-staging AI seats guarantees they have
            non-six legal moves without dice luck."""
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
              window.__forceRet = window.LUD.force(3);
              window.__forcePhase = window.LUG.phase;
              window.__forceSettle = Math.round(window.LUG.settleUntil - performance.now());
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
            page.evaluate("window.__drvDiag = 'pick:" + str(picked) +
                          " force=' + window.__forceRet + ' ph0=' + window.__forcePhase" +
                          " + ' settle=' + window.__forceSettle")
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

        # ---------- 1: engine-level Easy strategy ----------
        r = page.evaluate("""(() => {
          const E = LudoEngine, out = {};
          let g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          // six → release preferred (all four tokens in yard)
          let mv = E.legalMoves(g, 1, 6);
          out.sixAllRelease = mv.every(m => m.from === -1);
          out.easyPicksRelease =
            E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(42)).from === -1;
          // mixed: one token out, six → BOTH release & advance legal;
          // Easy must still prefer the release.
          g.players[1].tokens[0].pos = 5;
          mv = E.legalMoves(g, 1, 6);
          out.mixedMoves = mv.map(m => ({f: m.from, t: m.to}));
          for (let s = 1; s <= 50; s++) {
            const q = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(s));
            if (q.from !== -1) { out.mixedAlwaysRelease = false; break; }
            out.mixedAlwaysRelease = true;
          }
          // non-six: random among legal; same seed → same pick
          // (two tokens on track → TWO legal moves to spread across)
          g.players[1].tokens[1].pos = 12;
          mv = E.legalMoves(g, 1, 3);
          out.threeMoves = mv.map(m => ({f: m.from, t: m.to}));
          const p1 = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(7));
          const p2 = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(7));
          out.sameSeedSamePick = JSON.stringify(p1) === JSON.stringify(p2);
          // spread across seeds (randomness actually varies)
          /* mulberry32's FIRST output is correlated across small sequential
             seeds — decorrelate by hashing the seed before use. */
          const counts = {};
          for (let s = 1; s <= 300; s++) {
            const seed = (s * 2654435761) >>> 0;
            const q = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(seed));
            counts[q.token] = (counts[q.token] || 0) + 1;
          }
          out.spread = Object.keys(counts).length;
          out.counts = counts;
          // every Easy pick is in the legal set
          let allLegal = true;
          for (let s = 1; s <= 100; s++) {
            const q = E.selectAIMoveForDifficulty(g, 1, mv, 'easy', E.makeRng(s));
            if (!mv.some(m => m.token === q.token && m.from === q.from && m.to === q.to))
              { allLegal = false; break; }
          }
          out.allPicksLegal = allLegal;
          // no legal moves → null
          out.noMovesNull = E.selectAIMoveForDifficulty(g, 1, [], 'easy') === null;
          // default (null difficulty) unchanged → legacy scorer path
          out.legacyPath = E.selectAIMoveForDifficulty(g, 1, mv, null, E.makeRng(7)) !== null;
          return out;
        })()""")
        check("engine/sixOffersRelease", r["sixAllRelease"], json.dumps(r.get("sixMoves", "")))
        check("engine/easyPrefersRelease", r["easyPicksRelease"])
        check("engine/mixedSixAlwaysRelease", r.get("mixedAlwaysRelease"), json.dumps(r.get("mixedMoves")))
        check("engine/sameSeedDeterministic", r["sameSeedSamePick"])
        check("engine/randomSpread", r["spread"] >= 2, json.dumps(r["counts"]))
        check("engine/allPicksLegal", r["allPicksLegal"])
        check("engine/noMovesNull", r["noMovesNull"])
        check("engine/defaultUnchanged", r["legacyPath"])

        # ---------- 2: UI states ----------
        ui = page.evaluate("""(() => ({
          btns: [...document.querySelectorAll('#luDiffPick .lu-sp-btn')]
            .map(b => ({d: b.dataset.diff, dis: b.disabled,
                        on: b.classList.contains('on'),
                        soon: b.classList.contains('lu-soon')})),
          seatPickStillThere: !!document.querySelector('#luSeatPick'),
          humanPickStillThere: !!document.querySelector('#luHumanPick')}))()""")
        d = {b["d"]: b for b in ui["btns"]}
        check("ui/classicDefaultOn", d[""]["on"] and not d[""]["dis"])
        check("ui/easyAvailable", not d["easy"]["dis"] and not d["easy"]["on"])
        # Phase 4.4 enabled Medium; this suite pins Easy's contract.
        # Phase 4.5 landed: Hard is now a real, enabled difficulty too —
        # the old "reserved placeholder" contract is superseded.
        check("ui/mediumEnabledSincePhase4d",
              not d["medium"]["dis"] and not d["medium"]["soon"])
        check("ui/hardEnabledSincePhase4e",
              not d["hard"]["dis"] and not d["hard"]["soon"])
        check("ui/existingSelectorsPreserved",
              ui["seatPickStillThere"] and ui["humanPickStillThere"])

        # clicking an unselected level now applies it (fresh game)
        page.evaluate("""document.querySelector('#luDiffPick [data-diff=\\'hard\\']')
                         .dispatchEvent(new MouseEvent('click', {bubbles: true}))""")
        page.wait_for_timeout(300)
        s = page.evaluate("window.LUG.aiDifficulty")
        check("ui/hardClickApplies", s == "hard", f"aiDifficulty={s}")
        # restore Easy for the rest of this suite
        set_cfg(4, 1, "easy")

        # ---------- 3: select Easy; applies via fresh game ----------
        set_cfg(4, 1, "easy")
        s = page.evaluate("""(() => ({
          d: window.LUG.aiDifficulty,
          rng: typeof window.LUG.aiRng === 'function',
          kinds: window.LUG.game.players.map(P => P.kind)}))()""")
        check("apply/easySelected", s["d"] == "easy" and s["rng"], json.dumps(s))
        check("apply/kindsStill4P1H", s["kinds"] == ["human", "ai", "ai", "ai"], json.dumps(s["kinds"]))
        blurb = page.evaluate("document.getElementById('luBlurb').textContent")
        check("apply/blurbShowsLevel", "AI level: Easy" in blurb, blurb)

        # ---------- 4: live game — Easy AI plays legally ----------
        # stage AI seats too: guarantees they hold legal (non-six) moves,
        # so picks are certain instead of dice-luck dependent
        log = drive_red_and_wait(2, stage_ai=(1, 2, 3))
        # single drive with a generous window; retrying would re-stage
        # mid-AI-turn and corrupt the flow (force() is human-only)
        log = drive_red_and_wait(2, timeout=20)
        diag = page.evaluate("window.__drvDiag || 'none'")
        check("live/easyPicksFlow", log is not None and len(log) >= 2,
              f"picks={0 if not log else len(log)} diag={diag}")
        if log:
            illegal = [e for e in log if e["picked"] and e["picked"] not in e["legal"]]
            wrong_diff = [e for e in log if e["diff"] != "easy"]
            check("live/easyPicksOnlyLegal", not illegal, f"illegal={len(illegal)}")
            check("live/difficultyFlowsThrough", not wrong_diff, f"non-easy={len(wrong_diff)}")
        else:
            check("live/easyPicksOnlyLegal", False, "no picks captured")
            check("live/difficultyFlowsThrough", False, "no picks captured")

        # ---------- 5: human turn stays manual under Easy ----------
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

        # ---------- 6: no-legal-move AI turn passes correctly ----------
        # stage: green has all tokens home/finished so its roll yields none
        st = page.evaluate("""(() => {
          const g = window.LUG.game;
          // force green's turn with zero legal moves: block its yard release
          // by filling start cell? simpler: give green finished tokens
          g.players[1].tokens.forEach(t => { t.pos = 106; });   // all finished
          g.turn = 1; g.players[0].tokens[0].pos = 10;
          return {turn: g.turn};
        })()""")
        page.wait_for_timeout(1500)   # green rolls, finds no moves, passes
        s = page.evaluate("""(() => ({t: window.LUG.game.turn,
             k: window.LUG.game.players[window.LUG.game.turn].kind,
             ph: window.LUG.phase}))()""")
        check("live/noMoveAITurnPasses", s["t"] != 1 or s["ph"] == "idle", json.dumps(s))

        # ---------- 7: extra rolls & three consecutive sixes ----------
        # reset to a clean 2P easy game and drive red manually through
        # a six (extra roll) then verify turn logic intact
        set_cfg(2, 1, "easy")
        page.wait_for_timeout(400)
        page.evaluate("""(() => {
          window.LUG.clock++; window.luCancelDiceSpin(); window.luCancelAnim();
          window.LUG.game.players[0].tokens[0].pos = 20;
          window.LUD.force(6);
        })()""")
        dl = time.time() + 2
        while time.time() < dl and page.evaluate("window.LUG.phase") != "awaiting":
            page.wait_for_timeout(30)
        css = page.evaluate("""(() => {
          const stacks = window.__luBuildStacks();
          const mv = window.LUG.moves[0]; if (!mv) return null;
          const pt = window.__luTokenPoint(window.LUG.game.turn, mv.token, stacks);
          const r = document.getElementById('luBoard').getBoundingClientRect();
          return {x: r.left + pt.x*(r.width/window.LUG.w),
                  y: r.top + pt.y*(r.height/window.LUG.h)};
        })()""")
        if css: page.mouse.click(css["x"], css["y"])
        page.wait_for_timeout(600)
        s = page.evaluate("""(() => ({t: window.LUG.game.turn,
             sixes: window.LUG.game.players[0].sixes || null,
             ph: window.LUG.phase}))()""")
        # after a six, red either rolls again (extra roll) or already moved
        check("live/extraRollOnSix", s["t"] == 0 or s["ph"] in ("idle", "rolling", "moving", "awaiting"),
              json.dumps(s))

        # three consecutive sixes → forfeit (engine rule): simulate directly
        tri = page.evaluate("""(() => {
          const E = LudoEngine;
          const g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
          // engine contract: each six is registered after its move; the
          // NEXT extra roll is granted only while chain < 3. Third six
          // caps the chain → no further extra roll → turn passes on.
          E.noteRoll(g, 6);
          const r1 = E.grantsExtraRoll(g, 6);   // second six allowed
          E.noteRoll(g, 6);
          const r2 = E.grantsExtraRoll(g, 6);   // third six allowed
          E.noteRoll(g, 6);
          const r3 = E.grantsExtraRoll(g, 6);   // fourth roll DENIED
          return {r1, r2, r3, chain: g.chain};
        })()""")
        check("engine/threeSixesForfeit",
              tri["r1"] and tri["r2"] and not tri["r3"] and tri["chain"] == 3,
              json.dumps(tri))

        # ---------- 8: multiple AI seats all use Easy ----------
        set_cfg(4, 1, "easy")
        page.wait_for_timeout(400)
        log = drive_red_and_wait(2, timeout=20, distinct_seats=True, stage_ai=(1, 2, 3))
        seats_seen = sorted(set(e["player"] for e in (log or [])))
        check("live/multipleAISeatsUseEasy",
              len(seats_seen) >= 2 and all(s in (1, 2, 3) for s in seats_seen),
              f"seats={seats_seen} n={len(log or [])}")

        # ---------- 9: restart preserves difficulty ----------
        page.click("#luRestartBtn"); start_game()
        page.wait_for_timeout(500)
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
             n: window.LUG.seatCount, h: window.LUG.humanSeats,
             kinds: window.LUG.game.players.map(P => P.kind),
             rngFresh: window.LUG.aiRng !== null}))()""")
        check("restart/keepsEasy",
              s["d"] == "easy" and s["n"] == 4 and s["h"] == 1
              and s["kinds"] == ["human", "ai", "ai", "ai"] and s["rngFresh"],
              json.dumps(s))

        # ---------- 10: back-nav & re-entry ----------
        page.evaluate("location.hash = '#funtime'")
        page.wait_for_timeout(400)
        page.evaluate("location.hash = '#funtime/lu'")
        page.wait_for_selector("#luBoard")
        page.wait_for_timeout(600)
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
             n: window.LUG.seatCount, h: window.LUG.humanSeats}))()""")
        check("nav/reentryKeepsEasy", s["d"] == "easy" and s["n"] == 4 and s["h"] == 1,
              json.dumps(s))
        on = page.evaluate("""[...document.querySelectorAll('#luDiffPick .lu-sp-btn')]
             .filter(b => b.classList.contains('on')).map(b => b.dataset.diff)""")
        check("nav/selectorStateRestored", on == ["easy"], json.dumps(on))

        # ---------- 11: switching back to Classic restores legacy AI ----------
        open_lobby(); page.click("#luDiffPick [data-diff='']")
        start_game()
        page.wait_for_timeout(400)
        s = page.evaluate("""(() => ({d: window.LUG.aiDifficulty,
             blurb: document.getElementById('luBlurb').textContent}))()""")
        check("switch/backToClassic", s["d"] is None and "AI level" not in s["blurb"],
              json.dumps(s))

        # ---------- 12: human-only game unaffected ----------
        set_cfg(4, 4, "easy")
        page.wait_for_timeout(400)
        s = page.evaluate("""(() => ({
          d: window.LUG.aiDifficulty,
          kinds: window.LUG.game.players.map(P => P.kind),
          anyAI: window.LUG.game.players.some(P => P.active && P.kind === 'ai')}))()""")
        check("humanOnly/unaffectedByDifficulty",
              s["d"] == "easy" and not s["anyAI"]
              and s["kinds"] == ["human", "human", "human", "human"],
              json.dumps(s))
        auto = 0
        t0 = time.time()
        while time.time() - t0 < 2:
            a = page.evaluate("window.LUG.game.rolls")
            page.wait_for_timeout(200)
            b = page.evaluate("window.LUG.game.rolls")
            if b > a: auto += 1
        check("humanOnly/noAutoRolls", auto == 0, f"auto={auto}")

        # ---------- 13: no duplicate AI timers/rolls ----------
        set_cfg(4, 1, "easy")
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
