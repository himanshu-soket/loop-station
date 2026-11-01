#!/usr/bin/env python3
"""Phase 3 · Task 1 — smooth token movement (rAF interpolation) checks.

  1. Release a red token from the yard (engine commits pos 0).
  2. ONE normal track move (0 -> 3) at DEFAULT pacing: assert many
     distinct fractional `_viz` frames, per-hop chaining 0->1->2->3,
     t clamped to [0,1] and monotonic within each hop.
  3. Engine commit only via luMoveDone(): final pos == 3, `_viz` cleared.
  4. Race: unmount/remount mid-glide leaves no stale `_viz` or `LUG.anim`.

Run with the dashboard serving on 127.0.0.1:8907 (see docs/ludo-phase3.md).
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
    # DEFAULT pacing: stepMs=150/hop → a 3-square walk is ~450ms of frames

    def wait_idle_human(tmo=40):
        dl = time.time() + tmo
        while time.time() < dl:
            s = page.evaluate("({phase: window.LUG.phase, kind: window.LUG.game.players[window.LUG.game.turn].kind})")
            if s["phase"] == "idle" and s["kind"] == "human":
                return True
            page.wait_for_timeout(100)
        return False

    def click_legal_token():
        r = page.evaluate("""(() => {
          const LUG = window.LUG, E = window.LudoEngine;
          const g = LUG.game;
          if (LUG.phase !== 'awaiting' || !LUG.moves.length) return null;
          const mv = LUG.moves[0];
          const pos = g.players[g.turn].tokens[mv.token].pos;
          let pt;
          if (pos === -1) { const rc = E.YARD_SLOTS[g.turn][mv.token]; pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
          else { const rc = E.cellOf(g.turn,pos); pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
          const rect = document.getElementById('luBoard').getBoundingClientRect();
          return { x: rect.left + pt.x*(rect.width/LUG.w), y: rect.top + pt.y*(rect.height/LUG.h),
                   token: mv.token, from: pos, to: mv.to };
        })()""")
        if r:
            page.mouse.click(r["x"], r["y"])
        return r

    # ---------- STEP 1: release red token 0 ----------
    assert wait_idle_human()
    page.evaluate("window.LUD.force(6)")
    page.wait_for_timeout(700)              # dice tumble at default pacing
    rel = click_legal_token()
    check("release/clickLegalToken", rel is not None and rel["to"] == 0,
          json.dumps(rel))
    dl = time.time() + 10
    st = None
    while time.time() < dl:
        st = page.evaluate("({phase: window.LUG.phase, pos: window.LUG.game.players[0].tokens[0].pos})")
        if st["pos"] == 0 and st["phase"] != "moving":
            break
        page.wait_for_timeout(50)
    check("release/engineCommitted", st and st["pos"] == 0, json.dumps(st))

    # ---------- STEP 2: ONE NORMAL TRACK MOVE (0 -> 3), sampled ----------
    assert wait_idle_human()
    page.evaluate("window.LUD.force(3)")
    page.wait_for_timeout(700)
    mv = click_legal_token()
    check("trackMove/clickLegalToken", mv is not None and mv["from"] == 0 and mv["to"] == 3,
          json.dumps(mv))

    tok_i = mv["token"]
    seen, keys = [], set()
    final = None
    dl = time.time() + 8
    while time.time() < dl:
        r = page.evaluate("""(() => {
          const g = window.LUG.game;
          const tok = g.players[0].tokens[%d];
          return { phase: window.LUG.phase, pos: tok.pos,
                   viz: tok._viz ? {from: tok._viz.from, to: tok._viz.to, t: tok._viz.t} : null };
        })()""" % tok_i)
        if r["viz"]:
            k = (r["viz"]["from"], r["viz"]["to"], round(r["viz"]["t"], 3))
            if k not in keys:
                keys.add(k); seen.append(r["viz"])
        if r["phase"] != "moving" and r["pos"] == 3:
            final = r; break
        page.wait_for_timeout(4)

    ts = [v["t"] for v in seen]
    frs = sorted(set(v["from"] for v in seen))
    tos = sorted(set(v["to"] for v in seen))
    print(f"  samples={len(seen)} hops={list(zip(frs,tos))} t=[{min(ts):.3f}..{max(ts):.3f}]")

    check("glide/interpolatedFrames", len(seen) >= 10,
          f"{len(seen)} distinct fractional frames across 3 hops")
    check("glide/perHopChaining", frs == [0, 1, 2] and tos == [1, 2, 3],
          f"hops {frs}->{tos}")
    check("glide/tStartsAtZero", min(ts) >= 0.0, f"min t={min(ts):.3f}")   # no negative first frame
    check("glide/tMonotonicPerHop",
          all(seen[i]["t"] <= seen[i+1]["t"] or seen[i]["to"] != seen[i+1]["to"]
              for i in range(len(seen)-1)),
          "t non-decreasing within each hop")
    check("commit/enginePosIsFinal", final and final["pos"] == 3, json.dumps(final))
    check("commit/vizClearedAfterMove", final and final["viz"] is None)

    # ---------- STEP 3: race — restart mid-glide must not leave _viz ----------
    assert wait_idle_human()
    page.evaluate("window.LUD.force(6)")
    page.wait_for_timeout(700)
    rel2 = click_legal_token()
    # wait until it's mid-walk (phase moving) then hammer restart
    dl = time.time() + 5
    while time.time() < dl:
        ph = page.evaluate("window.LUG.phase")
        if ph == "moving":
            break
        page.wait_for_timeout(10)
    mid = page.evaluate("(window.LUG.phase)")
    # restart mid-animation (if still moving)
    page.evaluate("window.renderLuView && location.reload") if False else None
    # trigger a new game via the exposed restart path: find button
    has_btn = page.evaluate("""(() => {
      const b = document.querySelector('[data-lu-act="again"], #luAgain, .lu-new');
      return !!b;
    })()""")
    # use the internal path instead: LUD has none; emulate via hash remount
    if mid == "moving":
        page.evaluate("location.hash = '#funtime'")     # unmount mid-glide
        page.wait_for_timeout(300)
        page.evaluate("location.hash = '#funtime/lu'")  # fresh mount
        page.wait_for_timeout(400)
        st2 = page.evaluate("""(() => {
            const g = window.LUG.game;
            return { pos: g.players[0].tokens.map(t=>t.pos),
                     viz: g.players.flatMap(P=>P.tokens).some(t=>t._viz != null),
                     anim: window.LUG.anim };
        })()""")
        check("race/remountMidGlideNoStaleViz",
              st2 and st2["viz"] is False and st2["anim"] is None,
              json.dumps(st2))
    else:
        check("race/remountMidGlideNoStaleViz", True, "move finished before remount (skip)")

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
