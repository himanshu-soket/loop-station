#!/usr/bin/env python3
"""Phase 3 Part C checks: dice animation.

  1. During a roll the die face CHANGES (tumble), not frozen.
  2. The final painted face always equals the engine's rolled value.
  3. Rapid double-clicks during rolling cannot produce two rolls.
  4. Roll button is disabled while rolling/moving.
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
    # real pacing for this part (not LUD.fast) so the tumble is observable
    page.evaluate("""(() => {
      window.__rollsSeen = [];
      const orig = window.LUD.onRoll.bind(window.LUD);
      window.LUD.onRoll = v => { window.__rollsSeen.push(v); orig(v); };
    })()""")

    def die_face():
        return page.evaluate("""(() => {
          const die = document.querySelector('#luRollBtn .lu-die');
          const vis = [...die.querySelectorAll('i')].map(d => d.style.visibility !== 'hidden');
          // map visible pip pattern → face value
          const P = {1:[4],2:[0,8],3:[0,4,8],4:[0,2,6,8],5:[0,2,4,6,8],6:[0,2,3,5,6,8]};
          for (const [v, idx] of Object.entries(P)) {
            if (idx.every(i => vis[i]) && vis.filter(Boolean).length === idx.length) return +v;
          }
          return -1;
        })()""")

    # ---------- 1+2: tumble then land on engine value ----------
    page.click("#luRollBtn")
    faces = []
    for _ in range(6):                       # sample mid-roll faces
        faces.append(die_face())
        page.wait_for_timeout(60)
    rolled_during = page.evaluate("window.__rollsSeen.length")
    page.wait_for_timeout(900)               # let the roll resolve
    final_face = die_face()
    seen = page.evaluate("window.__rollsSeen")
    check("dice/tumblesDuringRoll",
          len(set(faces)) > 1 and rolled_during == 0,
          f"faces={faces} resolvedDuringRoll={rolled_during}")
    check("dice/finalFaceMatchesEngine",
          len(seen) == 1 and final_face == seen[0],
          f"final={final_face} engine={seen}")

    # ---------- 3: no duplicate rolls from rapid clicks ----------
    # Count only HUMAN-initiated rolls (AI players legitimately roll on
    # their own turns after the turn passes).
    deadline = time.time() + 20
    while time.time() < deadline:
        st = page.evaluate("({phase: window.LUG.phase, kind: window.LUG.game.players[window.LUG.game.turn].kind})")
        if st["phase"] == "idle" and st["kind"] == "human":
            break
        page.wait_for_timeout(200)
    page.evaluate("""(() => {
      window.__rollsSeen.length = 0;
      window.__humanRolls = 0;
      const orig = window.LUD.onRoll.bind(window.LUD);
      window.LUD.onRoll = v => {
        if (window.LUG.game.players[window.LUG.game.turn].kind === 'human') window.__humanRolls++;
        window.__rollsSeen.push(v); orig(v);
      };
    })()""")
    btn_disabled_mid = None
    for _ in range(5):                        # hammer the button
        try: page.click("#luRollBtn", timeout=300)
        except Exception: pass
        if btn_disabled_mid is None:
            btn_disabled_mid = page.evaluate("document.getElementById('luRollBtn').disabled")
        page.wait_for_timeout(70)
    page.wait_for_timeout(1200)
    human_rolls = page.evaluate("window.__humanRolls")
    check("dice/noDuplicateRolls", human_rolls == 1, f"humanRolls={human_rolls}")
    check("dice/buttonDisabledWhileRolling", btn_disabled_mid is True,
          f"disabled={btn_disabled_mid}")

    # ---------- console errors ----------
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))

    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
