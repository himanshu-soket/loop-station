#!/usr/bin/env python3
"""Phase 4.5 — Hard AI difficulty (engine core + UI wiring).

Engine: legal move selection; winning-move priority; capture priority;
safe-square preference; threat avoidance; deterministic fixed-seed
behavior; empty move list → null; no live-state mutation; node budget.

UI (4.5-B): Hard button enabled and selectable; router connection;
difficulty label/blurb update; Hard preserved across restart; multiple
AI seats; Classic/Easy/Medium unchanged.

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
        # Phase 5.5: dismiss the setup lobby with defaults before checks.
        page.wait_for_timeout(300)
        page.evaluate("window.luLobbyStart()")
        page.wait_for_timeout(400)

        # ---------- engine-level Hard strategy ----------
        r = page.evaluate("""(() => {
          const E = LudoEngine, W = E.HARD_WEIGHTS, out = {};
          const mk = () => E.createGame({kinds:['ai','ai','ai','ai']});

          // -- 1: picks a LEGAL move when any exist --
          {
            const g = mk();
            const ms = E.legalMoves(g, 0, 6);
            const pick = E.selectAIMoveHard(g, 0, ms, E.makeRng(42), false);
            out.picksLegal = ms.some(m => m.token === pick.token && m.to === pick.to);
            out.hasMoves = ms.length > 0;
          }

          // -- 2: winning move priority (to === FINISH beats all) --
          {
            const g = mk();
            // Red token at 55 with dice 1 → finishes (FINISH=56).
            g.players[0].tokens[0].pos = 55;
            g.players[0].tokens[1].pos = 10;
            const ms = E.legalMoves(g, 0, 1);
            const pick = E.selectAIMoveHard(g, 0, ms, E.makeRng(7), false);
            out.winMove = ms.some(m => m.to === E.FINISH);
            out.winsFirst = pick.to === E.FINISH;
          }

          // -- 3: capture priority over plain progress --
          {
            const g = mk();
            // Green token sits where Red lands with dice 3 from pos 20.
            g.players[0].tokens[0].pos = 20;
            g.players[1].tokens[0].pos = E.globalOf(0, 23) - E.START_G[1] >= 0
              ? (E.globalOf(0, 23) - E.START_G[1] + 52) % 52 : 0;
            // place directly: green's pos whose global equals red's to-square
            const gTo = E.globalOf(0, 23);
            g.players[1].tokens[0].pos = (gTo - E.START_G[1] + 52) % 52;
            g.players[0].tokens[1].pos = 40;   // alternative: plain runner
            const ms = E.legalMoves(g, 0, 3);
            const pick = E.selectAIMoveHard(g, 0, ms, E.makeRng(9), false);
            out.captureVictims = E.victimsAt(g, 0, 23).length;
            out.captures = pick.to === 23 && pick.token === 0;
          }

          // -- 4: safe-square preference between two equal advances --
          {
            const g = mk();
            g.players[0].tokens[0].pos = 4;    // to 8 → global 8 SAFE star
            g.players[0].tokens[1].pos = 30;   // to 34 → global 34 is ALSO a
                                               // star, so use 29→33 instead:
            g.players[0].tokens[1].pos = 29;   // to 33 → global 33 unsafe
            const ms = E.legalMoves(g, 0, 4);
            const pick = E.selectAIMoveHard(g, 0, ms, E.makeRng(5), false);
            out.safePicked = pick.token === 0 && pick.to === 8;
            out.to8Safe = E.isSafeStep(0, 8);
            out.to33Unsafe = !E.isSafeStep(0, 33);
          }

          // -- 5: threat avoidance — prefers the move not in enemy range --
          {
            const g = mk();
            // Yellow 6 behind Red's landing square 12 (unsafe, no star).
            const gTo = E.globalOf(0, 12);
            g.players[0].tokens[0].pos = 8;    // advance 4 → 12 (exposed)
            g.players[0].tokens[1].pos = 44;   // advance 4 → 48 (SAFE star)
            g.players[2].tokens[0].pos = (gTo - 6 - E.START_G[2] + 104) % 52;
            const ms = E.legalMoves(g, 0, 4);
            const pick = E.selectAIMoveHard(g, 0, ms, E.makeRng(11), false);
            out.avoidsThreat = pick.to === 48;
            out.threatReal = E.victimsAt(g, 2, 0).length === 0; // sanity only
          }

          // -- 6: deterministic under fixed seed --
          {
            const g = mk();
            g.players[0].tokens[0].pos = 10;
            g.players[0].tokens[1].pos = 20;
            const ms = E.legalMoves(g, 0, 3);
            const a = E.selectAIMoveHard(g, 0, ms, E.makeRng(123), false);
            const b = E.selectAIMoveHard(g, 0, ms, E.makeRng(123), false);
            out.deterministic = a.token === b.token && a.to === b.to;
          }

          // -- 7: empty move list returns null --
          out.emptyNull = E.selectAIMoveHard(mk(), 0, [], E.makeRng(1)) === null;

          // -- 8: no live-state mutation --
          {
            const g = mk();
            g.players[0].tokens[0].pos = 10;
            g.players[0].tokens[1].pos = 20;
            const before = JSON.stringify(g);
            const ms = E.legalMoves(g, 0, 3);
            E.selectAIMoveHard(g, 0, ms, E.makeRng(77), false);   // record=false
            E.selectAIMoveHard(g, 0, ms, E.makeRng(78));          // record=true
            out.noMutation = JSON.stringify(g) === before;
          }

          // -- 9: node budget is positive, finite, and enforced --
          {
            out.budget = E.HARD_NODE_BUDGET;
            out.budgetSane = Number.isInteger(E.HARD_NODE_BUDGET)
                             && E.HARD_NODE_BUDGET > 0
                             && E.HARD_NODE_BUDGET <= 1000;
            // Build a synthetic oversized move list: the selector must
            // still return a move (budget caps work, never correctness).
            const fake = Array.from({length: 500}, (_, i) =>
              ({token: i % 4, from: i % 50, to: (i % 50) + 3}));
            const g = mk();
            const pick = E.selectAIMoveHard(g, 0, fake, E.makeRng(3), false);
            out.budgetReturnsMove = !!pick && fake.slice(0, E.HARD_NODE_BUDGET)
              .some(m => m.token === pick.token && m.to === pick.to);
          }

          // -- 10: router wires 'hard' to the Hard selector --
          {
            const g = mk();
            g.players[0].tokens[0].pos = 10;
            const ms = E.legalMoves(g, 0, 3);
            const viaRouter = E.selectAIMoveForDifficulty(g, 0, ms, 'hard', E.makeRng(21));
            const direct = E.selectAIMoveHard(g, 0, ms, E.makeRng(21), false);
            out.routerMatches = viaRouter && viaRouter.token === direct.token
              && viaRouter.to === direct.to;
          }
          return out;
        })()""")

        check("hard: picks a legal move", r.get("picksLegal") and r.get("hasMoves"))
        check("hard: winning move available in fixture", r.get("winMove"))
        check("hard: finishing move chosen first", r.get("winsFirst"))
        check("hard: capture fixture has victims", r.get("captureVictims", 0) == 1,
              f"victims={r.get('captureVictims')}")
        check("hard: capture preferred over progress", r.get("captures"))
        check("hard: fixture star facts", r.get("to8Safe") and r.get("to33Unsafe"),
              f"to8Safe={r.get('to8Safe')} to33Unsafe={r.get('to33Unsafe')}")
        check("hard: safe square preferred", r.get("safePicked"))
        check("hard: threat avoided (chose star over exposed square)", r.get("avoidsThreat"))
        check("hard: deterministic fixed-seed", r.get("deterministic"))
        check("hard: empty list → null", r.get("emptyNull"))
        check("hard: no live-state mutation", r.get("noMutation"))
        check("hard: node budget sane", r.get("budgetSane"), f"budget={r.get('budget')}")
        check("hard: budget caps work, still returns move", r.get("budgetReturnsMove"))
        check("hard: router routes to Hard selector", r.get("routerMatches"))

        # ---------- UI wiring (Phase 4.5-B) ----------
        page.evaluate("window.LUD.fast()")
        page.wait_for_timeout(200)

        def pick_diff(d):
            dd = "" if d == "" else d
            sel = json.dumps(f"#luDiffPick [data-diff='{dd}']")
            page.evaluate(f"document.querySelector({sel}).click()")
            page.wait_for_timeout(350)

        def diff_state():
            return page.evaluate("""(() => ({
              hardDisabled: document.querySelector("#luDiffPick [data-diff='hard']").disabled,
              hardSoon: document.querySelector("#luDiffPick [data-diff='hard']").classList.contains('lu-soon'),
              selected: [...document.querySelectorAll('#luDiffPick .lu-sp-btn')]
                .filter(b => b.classList.contains('on')).map(b => b.dataset.diff),
              aria: [...document.querySelectorAll('#luDiffPick .lu-sp-btn')]
                .map(b => b.getAttribute('aria-checked')),
              blurb: document.getElementById('luBlurb').textContent,
              cfg: window.LUG.aiDifficulty,
            }))()""")

        # -- 11: Hard button enabled, no 'coming soon' styling --
        st = diff_state()
        check("ui: Hard button enabled", not st["hardDisabled"] and not st["hardSoon"],
              f"disabled={st['hardDisabled']} lu-soon={st['hardSoon']}")

        # -- 12: selecting Hard applies it and updates the label/blurb --
        pick_diff("hard")
        st = diff_state()
        check("ui: Hard selectable", st["selected"] == ["hard"], f"selected={st['selected']}")
        check("ui: Hard aria-checked", st["aria"][3] == "true", f"aria={st['aria']}")
        check("ui: blurb shows Hard level", "AI level: Hard" in st["blurb"], st["blurb"])
        check("ui: LUG.aiDifficulty === 'hard'", st["cfg"] == "hard")

        # -- 13: Hard preserved across restart --
        page.evaluate("window.LUG.game && true")
        page.evaluate("""(() => {
          // force a restart via the overlay 'again' path if present, else
          // call the internal new-game directly through a config change
          const again = document.querySelector("[data-act='again']");
          if (again) again.click(); else window.LUG.clock++, null;
        })()""")
        page.wait_for_timeout(400)
        # deterministic restart: re-select seats=4 (no-op) then verify state
        page.evaluate("document.querySelector(\"#luSeatPick [data-seats='4']\").click()")
        page.wait_for_timeout(400)
        st = diff_state()
        check("ui: Hard survives restart", st["cfg"] == "hard" and st["selected"] == ["hard"],
              f"cfg={st['cfg']} selected={st['selected']}")

        # -- 14: Hard works with multiple AI seats (drive the human roll
        #        through the real force() hook, then let AI seats respond) --
        page.evaluate("document.querySelector(\"#luHumanPick [data-humans='1']\").click()")
        page.wait_for_timeout(300)
        t0 = time.time()
        progressed = False
        while time.time() - t0 < 10:
            s = page.evaluate("""(() => ({
              ph: window.LUG.phase,
              turn: window.LUG.game ? window.LUG.game.turn : -1,
              anyPos: window.LUG.game ? window.LUG.game.players.some(P => P.tokens.some(t => t.pos >= 0)) : false,
              winner: window.LUG.game ? window.LUG.game.winner : null}))()""")
            if s["anyPos"]:
                progressed = True
                break
            if s["ph"] == "idle" and s["turn"] == 0:
                page.evaluate("window.LUD.force(6)")   # release a token
            elif s["ph"] == "awaiting" and s["turn"] == 0:
                # commit the human's pick through the REAL pick path
                page.evaluate("""(() => {
                  if (!window.LUG.moves || !window.LUG.moves.length) return;
                  const t = window.LUG.game.turn;
                  window.luPick({p: t, i: window.LUG.moves[0].token});
                })()""")
            page.wait_for_timeout(150)
        check("ui: Hard game progresses with AI seats", progressed)

        # -- 15: Classic / Easy / Medium unchanged --
        for d, label in [("", "Classic"), ("easy", "Easy"), ("medium", "Medium")]:
            pick_diff(d)
            st = diff_state()
            ok = st["selected"] == ([d] if d else [""]) and st["cfg"] == (d if d else None)
            lbl_ok = (f"AI level: {label}" in st["blurb"]) if d else ("AI level:" not in st["blurb"])
            check(f"ui: {label} still selectable + labelled", ok and lbl_ok,
                  f"selected={st['selected']} cfg={st['cfg']}")

        # back to hard for the final screenshot
        pick_diff("hard")
        page.wait_for_timeout(300)

        page.screenshot(path="/tmp/ludo_4e.png")
        browser.close()

    print(f"\n{'='*46}\nPhase 4.5 Hard AI: {PASS} passed, {FAIL} failed")
    if errors:
        print("Console/page errors:")
        for e in errors[:10]:
            print("  " + e)
    sys.exit(1 if FAIL or errors else 0)


if __name__ == "__main__":
    main()
