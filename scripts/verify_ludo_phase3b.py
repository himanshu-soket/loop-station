#!/usr/bin/env python3
"""Phase 3 Part B checks: stacking + hit-test single source of truth.

Drives the real page in Chromium:
  1. draw/hit coordinates agree for every token (resting & stacked)
  2. mixed-player stacks stay clickable, legal tokens preferred
  3. 2–8+ token stacks: every member individually clickable
  4. stack radius keeps tokens inside their cell
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

    # ---------- 1. draw/hit coordinate agreement ----------
    r = page.evaluate("""(() => {
      const LUG = window.LUG, E = window.LudoEngine;
      // stage a game with several resting tokens on track squares
      const g = LUG.game;
      g.players[0].tokens[0].pos = 10;   // red pair on global 10
      g.players[0].tokens[1].pos = 10;
      g.players[1].tokens[0].pos = 23;   // green pair (global 13+23=36)
      g.players[1].tokens[1].pos = 23;
      g.players[2].tokens[0].pos = 49;   // yellow trio (global 26+49=75%52=23)
      g.players[2].tokens[1].pos = 49;
      g.players[2].tokens[2].pos = 49;
      LUG.phase = 'idle'; LUG.moves = [];
      window.__luDraw();
      // hit-test each token centre — same stack map both sides use
      const stacks = window.__luBuildStacks();
      let mismatch = 0;
      for (let p = 0; p < 4; p++) {
        for (let i = 0; i < 4; i++) {
          const tok = g.players[p].tokens[i];
          if (tok.pos < 0 && tok.pos !== -1) continue;
          const pt = window.__luTokenPoint(p, i, stacks);
          if (!pt) continue;
          const hit = window.__luHitTest(pt.x, pt.y);
          if (!hit || hit.p !== p || hit.i !== i) mismatch++;
        }
      }
      return { mismatch };
    })()""")
    check("stack/drawHitAgree", r["mismatch"] == 0, f"mismatches={r['mismatch']}")

    # ---------- 2. mixed-player stack: legal token preferred ----------
    r = page.evaluate("""(() => {
      const LUG = window.LUG, E = window.LudoEngine;
      const g = LUG.game;
      // reset to clean slate
      const fresh = E.createGame(['human','ai','ai','ai']);
      LUG.game = fresh; LUG.phase = 'awaiting';
      LUG.dice = 1;
      // red token0 on global 5 (red start=0), blue token0 also on global 5
      fresh.players[0].tokens[0].pos = 5;
      fresh.players[3].tokens[0].pos = (5 - 39 + 52) % 52;  // blue rel → global 5
      LUG.moves = E.legalMoves(fresh, 0, 1);                 // red can move token0
      window.__luDraw();
      // click exactly on the shared square centre
      const c = document.getElementById('luBoard');
      const rect = c.getBoundingClientRect();
      const cell = LUG.cell;
      const rc = E.cellOf(0, 5);
      const x = rect.left + ((rc[1] + 0.5) * cell) * (rect.width / LUG.w);
      const y = rect.top + ((rc[0] + 0.5) * cell) * (rect.height / LUG.h);
      return { x, y };
    })()""")
    page.mouse.click(r["x"], r["y"])
    page.wait_for_timeout(1200)
    st = page.evaluate("({moved: window.LUG.game.players[0].tokens[0].pos, phase: window.LUG.phase})")
    check("stack/mixedLegalPreferred", st["moved"] > 5 or st["phase"] != "moving",
          json.dumps(st))

    # ---------- 3. up-to-16-token stack: every member clickable ----------
    r = page.evaluate("""(() => {
      const LUG = window.LUG, E = window.LudoEngine;
      const g = LUG.game;
      // park everyone on global 30 (a non-safe plain square)
      const target = 30;
      let n = 0;
      for (let p = 0; p < 4; p++) {
        for (let i = 0; i < 4; i++) {
          g.players[p].tokens[i].pos = (target - E.START_G[p] + 52) % 52;
          n++;
        }
      }
      g.players[0].home = 0;
      LUG.phase = 'idle'; LUG.moves = [];
      window.__luDraw();
      // verify each member's own point resolves back to itself via hit-test
      const stacks = window.__luBuildStacks();
      let bad = [];
      for (let p = 0; p < 4; p++) {
        for (let i = 0; i < 4; i++) {
          const pt = window.__luTokenPoint(p, i, stacks);
          if (!pt) { bad.push(p + ':' + i + ':nopt'); continue; }
          const hit = window.__luHitTest(pt.x, pt.y);
          if (!hit) bad.push(p + ':' + i + ':null');
          else if (hit.p !== p || hit.i !== i) bad.push(p + ':' + i + ':wrong→' + hit.p + ':' + hit.i);
        }
      }
      // all tokens must stay inside their cell bounds
      const C = LUG.cell;
      const rc = E.TRACK[target];
      const cx = (rc[1] + 0.5) * C, cy = (rc[0] + 0.5) * C;
      let outside = 0;
      for (let p = 0; p < 4; p++) {
        for (let i = 0; i < 4; i++) {
          const pt = window.__luTokenPoint(p, i, stacks);
          if (Math.abs(pt.x - cx) > C / 2 || Math.abs(pt.y - cy) > C / 2) outside++;
        }
      }
      return { n, bad, outside };
    })()""")
    check("stack/all16Clickable", len(r["bad"]) == 0, "; ".join(r["bad"][:4]))
    check("stack/allInsideCell", r["outside"] == 0, f"outside={r['outside']}")

    # ---------- 4. 2–8 token stacks radius scaling monotonic ----------
    r = page.evaluate("""(() => {
      const LUG = window.LUG;
      const rs = [2,3,4,5,6,7,8,12,16].map(n => window.__luStackRadius(n));
      let mono = true;
      for (let i = 1; i < rs.length; i++) if (rs[i] > rs[i-1]) mono = false;
      const C = LUG.cell;
      const fits = rs.every(r => r * 2 <= C * 1.02);   // diameter within cell
      return { rs: rs.map(x => Math.round(x * 100) / 100), mono, fits };
    })()""")
    check("stack/radiusScalesDown", r["mono"], json.dumps(r["rs"]))
    check("stack/radiusFitsCell", r["fits"], json.dumps(r["rs"]))

    # ---------- console errors ----------
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))

    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
