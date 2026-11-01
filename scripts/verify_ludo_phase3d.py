#!/usr/bin/env python3
"""Phase 3 Part D checks: state protection + illegal-move feedback.

  1. LUD.force rejected while rolling / moving / awaiting / game over.
  2. LUD.force still works from idle (legit test hook preserved).
  3. Clicking an opponent token: message shown, NO state change.
  4. Clicking own unmovable token: message, no state change.
  5. Legal selection still works right after illegal clicks.
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
    page.wait_for_timeout(300)
    page.evaluate("window.luLobbyStart()")
    page.wait_for_timeout(400)
    page.evaluate("window.LUD.fast()")

    # ---------- helper: reach a human idle phase ----------
    def pick_first_legal():
        """Click the first legal token (a real player action) so an
        awaiting human turn resolves and the game can flow on. Without
        this, wait_idle_human below could never return once any section
        left the game in 'awaiting' with nobody picking."""
        r = page.evaluate("""(() => {
          const LUG = window.LUG;
          if (LUG.phase !== 'awaiting' || !LUG.moves || !LUG.moves.length) return null;
          const E = window.LudoEngine;
          const g = LUG.game, p = g.turn;
          if (g.players[p].kind !== 'human') return null;
          const mv = LUG.moves[0];
          const pos = g.players[p].tokens[mv.token].pos;
          let pt;
          if (pos === -1) { const rc = E.YARD_SLOTS[p][mv.token]; pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
          else { const rc = E.cellOf(p,pos); pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
          const c = document.getElementById('luBoard');
          const rect = c.getBoundingClientRect();
          return { x: rect.left + pt.x*(rect.width/LUG.w), y: rect.top + pt.y*(rect.height/LUG.h) };
        })()""")
        if r:
            page.mouse.click(r["x"], r["y"])
        return r is not None

    def wait_idle_human(tmo=20):
        dl = time.time() + tmo
        while time.time() < dl:
            st = page.evaluate("({phase: window.LUG.phase, kind: window.LUG.game.players[window.LUG.game.turn].kind})")
            if st["phase"] == "idle" and st["kind"] == "human":
                return True
            # a pending HUMAN awaiting turn blocks idle forever — resolve it
            # exactly as a player would (click a legal token)
            if st["phase"] == "awaiting" and st["kind"] == "human":
                pick_first_legal()
            page.wait_for_timeout(150)
        return False

    # ---------- 1a: force rejected while ROLLING ----------
    wait_idle_human()
    page.click("#luRollBtn")
    page.wait_for_timeout(30)                       # mid-tumble (fast pacing ~30ms)
    mid = page.evaluate("window.LUG.phase")
    r_rolling = page.evaluate("window.LUD.force(6)") if mid == "rolling" else None
    if mid != "rolling":                            # timing miss: retry once
        page.wait_for_timeout(500)
        wait_idle_human()
        page.click("#luRollBtn")
        page.wait_for_timeout(10)
        mid = page.evaluate("window.LUG.phase")
        r_rolling = page.evaluate("window.LUD.force(6)") if mid == "rolling" else None
    page.wait_for_timeout(400)

    # ---------- 1b: force rejected while AWAITING ----------
    # guarantee an awaiting state regardless of what the natural roll gave
    wait_idle_human()
    page.evaluate("window.LUD.force(6)")            # legit hook → awaiting
    rolls_before = page.evaluate("window.LUG.game.rolls")
    r_awaiting = page.evaluate("window.LUD.force(6)")
    rolls_after = page.evaluate("window.LUG.game.rolls")
    check("force/rejectedWhileRolling", mid == "rolling" and r_rolling is False,
          f"phase={mid} force→{r_rolling}")
    check("force/rejectedWhileAwaiting",
          r_awaiting is False and rolls_after == rolls_before,
          f"force→{r_awaiting} rolls {rolls_before}→{rolls_after}")

    # ---------- 1c: force rejected while MOVING ----------
    # Stage an awaiting state, click a legal token, and try to force
    # mid-animation. Retry until the click lands inside 'moving'.
    staged_move = False
    for _ in range(6):
        wait_idle_human()
        page.evaluate("window.LUD.force(6)")         # guaranteed awaiting+moves
        page.wait_for_timeout(60)
        stg = page.evaluate("""(() => {
          const LUG = window.LUG;
          if (LUG.phase !== 'awaiting' || !LUG.moves || !LUG.moves.length) return null;
          const mv = LUG.moves[0];
          const g = LUG.game, p = g.turn;
          const pt = window.__luTokenPoint(p, mv.token, window.__luBuildStacks());
          if (!pt) return null;
          const c = document.getElementById('luBoard');
          const r = c.getBoundingClientRect();
          return {x: r.left + pt.x*(r.width/LUG.w), y: r.top + pt.y*(r.height/LUG.h)};
        })()""")
        if not stg:
            page.wait_for_timeout(300)
            continue
        page.mouse.click(stg["x"], stg["y"])
        page.wait_for_timeout(8)                     # mid-glide (fast pacing)
        ph = page.evaluate("window.LUG.phase")
        if ph == "moving":
            r_moving = page.evaluate("window.LUD.force(6)")
            check("force/rejectedWhileMoving", r_moving is False,
                  f"phase={ph} force→{r_moving}")
            staged_move = True
            break
        page.wait_for_timeout(400)                   # missed window; retry
    if not staged_move:
        check("force/rejectedWhileMoving", False, "could not catch mid-moving window")

    # ---------- 2: legit force still works from idle ----------
    wait_idle_human()
    ok_force = page.evaluate("window.LUD.force(6)")
    page.wait_for_timeout(80)
    st = page.evaluate("({phase: window.LUG.phase, moves: window.LUG.moves.length, dice: window.LUG.dice, turn: window.LUG.game.turn})")
    check("force/worksFromIdle", ok_force is True and st["phase"] == "awaiting" and st["moves"] > 0,
          json.dumps(st))

    # ---------- staging for illegal-click sections ----------
    def stage_awaiting():
        """Guarantee a FRESH awaiting+moves state for the human.

        Marks the roll generation so we never accept a stale awaiting
        phase left over from an earlier section (its LUG.moves may have
        been computed against different token positions)."""
        for _ in range(10):
            gen = page.evaluate("window.LUG.game.rolls")
            wait_idle_human(5)
            forced = page.evaluate("window.LUD.force(6)")
            page.wait_for_timeout(60)
            st = page.evaluate(
                "({phase: window.LUG.phase, n: (window.LUG.moves||[]).length,"
                " rolls: window.LUG.game.rolls})")
            if (forced is True and st["phase"] == "awaiting"
                    and st["n"] > 0 and st["rolls"] > gen):
                return True
            page.wait_for_timeout(300)
        return False

    # ---------- 3: opponent token click → feedback, no state change ----------
    if not stage_awaiting():
        check("illegal/opponentTokenFeedback", False, "could not stage awaiting")
    r = page.evaluate("""(() => {
      const LUG = window.LUG, E = window.LudoEngine;
      const g = LUG.game;
      // put a green token on the track for red to wrongly click
      g.players[1].tokens[0].pos = 10;   // green rel 10 → global 23
      window.__luDraw();
      const rc = E.cellOf(1, 10);
      const c = document.getElementById('luBoard');
      const rect = c.getBoundingClientRect();
      return { x: rect.left + ((rc[1]+0.5)*LUG.cell)*(rect.width/LUG.w),
               y: rect.top  + ((rc[0]+0.5)*LUG.cell)*(rect.height/LUG.h),
               before: JSON.stringify({phase: LUG.phase, dice: LUG.dice,
                     moves: LUG.moves.length, turn: g.turn,
                     green: g.players[1].tokens[0].pos}) };
    })()""")
    page.mouse.click(r["x"], r["y"])
    page.wait_for_timeout(250)
    after = page.evaluate("""JSON.stringify({phase: window.LUG.phase, dice: window.LUG.dice,
       moves: window.LUG.moves.length, turn: window.LUG.game.turn,
       green: window.LUG.game.players[1].tokens[0].pos,
       msg: document.getElementById('luMsg').textContent})""")
    check("illegal/opponentTokenFeedback",
          "Green" in json.loads(after)["msg"] or "token" in json.loads(after)["msg"].lower(),
          json.loads(after)["msg"][:60])
    check("illegal/noStateChange", after.split(',"msg"')[0] + "}" == r["before"],
          f"before={r['before']} after={after.split(',\"msg\"')[0]}}}")

    # ---------- 4: own unmovable token → feedback ----------
    # Set the token FINISHED first, THEN roll, so the engine's legalMoves
    # genuinely excludes it (mirrors real play where pos was already 56).
    page.evaluate("""(() => {
      window.LUG.game.players[0].tokens[3].pos = 56;   // finished — frozen
    })()""")
    if not stage_awaiting():
        check("illegal/ownUnmovableFeedback", False, "could not stage awaiting")
        r2 = None
    else:
      r2 = page.evaluate("""(() => {
      const LUG = window.LUG, E = window.LudoEngine;
      const g = LUG.game;
      window.__luDraw();
      // click the finished token's slot in the centre pool
      const c = document.getElementById('luBoard');
      const rect = c.getBoundingClientRect();
      const pt = window.__luTokenPoint ? window.__luTokenPoint(0, 3) : null;
      if (!pt) return null;
      return { x: rect.left + pt.x*(rect.width/LUG.w),
               y: rect.top + pt.y*(rect.height/LUG.h) };
    })()""")
    if r2:
        page.mouse.click(r2["x"], r2["y"])
        page.wait_for_timeout(250)
        st2 = page.evaluate("""({msg: document.getElementById('luMsg').textContent,
             phase: window.LUG.phase, moves: window.LUG.moves.length})""")
        check("illegal/ownUnmovableFeedback",
              "home" in st2["msg"].lower() or "can't" in st2["msg"].lower() or "cannot" in st2["msg"].lower(),
              st2["msg"][:60])
        check("illegal/unmovableNoStateChange",
              st2["phase"] == "awaiting" and st2["moves"] > 0,
              json.dumps(st2))
    else:
        check("illegal/ownUnmovableFeedback", False, "staging failed")

    # ---------- 5: legal pick still works after illegal clicks ----------
    if not stage_awaiting():
        check("illegal/legalPickStillWorks", False, "could not stage awaiting")
        r3 = None
    # Use the stack-aware __luTokenPoint helper: when several tokens share
    # a cell they fan out, and the naive cell-centre math clicks empty space
    # (the pick-fix releases more tokens, so stacks now actually occur).
    r3 = page.evaluate("""(() => {
      const LUG = window.LUG;
      const g = LUG.game;
      if (LUG.phase !== 'awaiting' || !LUG.moves || !LUG.moves.length) return null;
      const mv = LUG.moves.find(m => m.token !== 3) || LUG.moves[0];
      const pt = window.__luTokenPoint(g.turn, mv.token, window.__luBuildStacks());
      if (!pt) return null;
      const c = document.getElementById('luBoard');
      const rect = c.getBoundingClientRect();
      return { x: rect.left + pt.x*(rect.width/LUG.w), y: rect.top + pt.y*(rect.height/LUG.h),
               token: mv.token, to: mv.to };
    })()""")
    if r3:
        page.mouse.click(r3["x"], r3["y"])
    page.wait_for_timeout(2000)
    st3 = page.evaluate("""({phase: window.LUG.phase,
        tok: window.LUG.game.players[0].tokens[%d].pos})""" % (r3["token"] if r3 else 0))
    check("illegal/legalPickStillWorks",
          st3["tok"] >= 0 and st3["phase"] != "error",
          json.dumps(st3))

    # ---------- console errors ----------
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))

    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
