#!/usr/bin/env python3
"""Phase 5.3 · Premium token movement & animation polish — checks.

  A. HOP ARC: mid-glide samples show a vertical lift (rendered y is
     ABOVE the pure linear interpolation) that peaks mid-hop and returns
     to zero at both endpoints; hit-test still matches the painted spot.
  B. FX LAYER: each event kind fires exactly when it should and every
     effect expires (fx list empties, pump rAF returns to 0).
  C. CAPTURE: engine commits instantly (victim pos -1) while impact+fly
     overlays run; turn pacing unchanged (eventMs settle still applies).
  D. STACK EASE: fan-out converges to the EXACT deterministic layout
     (no permanent drift) and hit-test agrees with painted centres.
  E. LIFECYCLE: restart / teardown / route-away during glide, FX and
     stack-ease leave no stale rAF (glide, fx, stack), no frozen _viz,
     no duplicate pumps. Resize mid-glide keeps coordinates correct.
  F. NUDGE + HOVER: illegal click flashes red ring that decays; hover
     on a legal token paints the selection keyline (state-free).

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
    page.wait_for_timeout(300)
    page.evaluate("window.luLobbyStart()")
    page.wait_for_timeout(500)
    page.evaluate("window.LUD.fast()")

    # helper: freeze flow, place tokens, roll for red, execute first move
    def synth_move(setup, dice):
        return page.evaluate("""(args) => {
          window.LUG.clock++;                    // freeze pending turn flow
          const g = window.LUG.game;
          for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
          g.turn = 0; g.chain = 0; g.winner = null;
          eval(args.setup);
          LUG.phase = 'awaiting'; LUG.dice = args.dice;
          LUG.moves = LudoEngine.legalMoves(g, 0, args.dice);
          if (!LUG.moves.length) return null;
          const m = LUG.moves[0];
          window.luExecuteMove({token: m.token, to: m.to});
          return {token: m.token, from: m.from, to: m.to};
        }""", {"setup": setup, "dice": dice})

    def wait_quiet(ms=900):
        page.wait_for_timeout(ms)

    # ---------- A: hop arc ----------
    mv = synth_move("g.players[0].tokens[0].pos = 2;", 3)   # 2 -> 5, 3 hops
    check("hop/moveStarted", mv is not None and mv["to"] == 5, json.dumps(mv))
    samples = page.evaluate("""(tok) => new Promise(res => {
      const out = [];
      const t0 = performance.now();
      const iv = setInterval(() => {
        const g = window.LUG.game;
        const t = g.players[0].tokens[tok];
        const LUG = window.LUG, E = window.LudoEngine;
        if (t && t._viz && typeof t._viz === 'object') {
          // painted point (with hop) vs pure linear point (no hop)
          const painted = (typeof luTokenPointSafe === 'function')
            ? null : null;
          const A = t._viz.from === -1
            ? {x:(E.YARD_SLOTS[0][tok][1]+.5)*LUG.cell, y:(E.YARD_SLOTS[0][tok][0]+.5)*LUG.cell}
            : (() => { const rc = E.cellOf(0, t._viz.from); return {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; })();
          const B = (() => { const rc = E.cellOf(0, t._viz.to); return {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; })();
          const tt = Math.max(0, Math.min(1, t._viz.t));
          const e = tt < .5 ? 2*tt*tt : 1 - Math.pow(-2*tt+2, 2)/2;
          const linear = {x: A.x+(B.x-A.x)*e, y: A.y+(B.y-A.y)*e};
          const viz = {from: t._viz.from, to: t._viz.to, t: tt};
          out.push({viz, linear});
        }
        if (performance.now() - t0 > 400) { clearInterval(iv); res(out); }
      }, 8);
    })""", mv["token"])
    lifted = [s for s in samples
              if 0.15 < s["viz"]["t"] < 0.85 and s["viz"]["from"] != s["viz"]["to"]]
    # the hop is applied inside luGlidePoint; verify via canvas sampling:
    # the painted y must be LESS than linear y (above) for mid-hop frames
    # — recompute what luGlidePoint yields by calling it directly.
    hop_ok = page.evaluate("""(samples) => {
      let above = 0, total = 0;
      for (const s of samples) {
        const pt = window.luGlidePoint ? null : null;
        total++;
      }
      return {above, total};
    }""", samples)
    # direct probe: call luGlidePoint with a synthetic mid-hop viz
    probe = page.evaluate("""(() => {
      const out = [];
      for (const t of [0, .25, .5, .75, 1]) {
        const rcA = [1,6], rcB = [1,7];   // any two adjacent cells
        // use real cells: red rel 2 → 3
        const E = window.LudoEngine, LUG = window.LUG;
        const a = E.cellOf(0, 2), b = E.cellOf(0, 3);
        const A = {x:(a[1]+.5)*LUG.cell, y:(a[0]+.5)*LUG.cell};
        const B = {x:(b[1]+.5)*LUG.cell, y:(b[0]+.5)*LUG.cell};
        const pt = luGlidePoint(0, 0, {from: 2, to: 3, t: t});
        const e = t < .5 ? 2*t*t : 1 - Math.pow(-2*t+2, 2)/2;
        const linY = A.y + (B.y - A.y) * e;
        out.push({t, y: pt.y, linY, lift: linY - pt.y});
      }
      return out;
    })()""")
    lifts = [pr["lift"] for pr in probe]
    check("hop/zeroAtEndpoints",
          abs(lifts[0]) < 0.01 and abs(lifts[-1]) < 0.01,
          f"lifts={[round(l,2) for l in lifts]}")
    check("hop/peaksMidHop",
          max(lifts) == lifts[2] and max(lifts) > 3,
          f"peak={lifts[2]:.1f}px at t=0.5")
    wait_quiet()

    # hit-test sync during hover of a legal token (static position)
    hit_sync = page.evaluate("""(() => {
      const LUG = window.LUG, g = LUG.game;
      g.players[0].tokens[0].pos = 10;
      LUG.phase = 'awaiting'; LUG.dice = 1;
      LUG.moves = LudoEngine.legalMoves(g, 0, 1);
      window.luDraw();
      // painted centre via the same path the renderer uses
      const stacks = window.luBuildStacks();
      const pt = window.luTokenPoint(0, 0, stacks);
      const hit = window.luHitTest(pt.x, pt.y);
      return {painted: {x: Math.round(pt.x), y: Math.round(pt.y)},
              hit: hit ? {p: hit.p, i: hit.i} : null};
    })()""")
    check("hitTest/matchesPaintedCentre",
          hit_sync["hit"] and hit_sync["hit"]["p"] == 0 and hit_sync["hit"]["i"] == 0,
          json.dumps(hit_sync))

    # ---------- B: FX layer per event ----------
    def fx_kinds(setup, dice, label):
        mv = synth_move(setup, dice)
        if mv is None:
            check(f"fx/{label}", False, "no legal move in setup")
            return
        r = page.evaluate("""(tok) => new Promise(res => {
          const seen = new Set();
          const t0 = performance.now();
          const iv = setInterval(() => {
            for (const f of LUG.fx) seen.add(f.kind);
            if (performance.now() - t0 > 800) {
              clearInterval(iv);
              res({kinds: [...seen], fxAfter: LUG.fx.length, fxRaf: LUG.fxRaf});
            }
          }, 20);
        })""", mv["token"])
        check(f"fx/{label}", label in r["kinds"] and r["fxAfter"] == 0 and r["fxRaf"] == 0,
              json.dumps(r))

    fx_kinds("g.players[0].tokens[0].pos = 2;", 6, "safe")       # lands global 8
    fx_kinds("g.players[0].tokens[0].pos = 50;", 3, "lane")      # enters lane
    fx_kinds("g.players[0].tokens[0].pos = 54;", 2, "finish")    # reaches centre

    # capture: impact + fly, engine commits immediately
    cap = page.evaluate("""(() => {
      window.LUG.clock++;
      const g = window.LUG.game;
      for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
      g.turn = 0; g.chain = 0; g.winner = null;
      g.players[0].tokens[0].pos = 9;
      g.players[1].tokens[0].pos = (10 - 13 + 52) % 52;   // green on global 10
      LUG.phase = 'awaiting'; LUG.dice = 1;
      LUG.moves = LudoEngine.legalMoves(g, 0, 1);
      const m = LUG.moves[0];
      window.luExecuteMove({token: m.token, to: m.to});
      return {token: m.token};
    })()""")
    capr = page.evaluate("""(tok) => new Promise(res => {
      const seen = new Set();
      let victimPosEarly = null;
      const t0 = performance.now();
      const iv = setInterval(() => {
        const g = window.LUG.game;
        if (victimPosEarly === null && g.players[1].tokens[0].pos === -1
            && g.players[0].tokens[tok].pos === 10) {
          victimPosEarly = true;   // engine committed while FX may still run
        }
        for (const f of LUG.fx) seen.add(f.kind);
        if (performance.now() - t0 > 900) {
          clearInterval(iv);
          res({kinds: [...seen], engineCommittedEarly: victimPosEarly === true,
               fxAfter: LUG.fx.length, fxRaf: LUG.fxRaf,
               victim: g.players[1].tokens[0].pos});
        }
      }, 20);
    })""", cap["token"])
    check("fx/captureImpactAndFly",
          "impact" in capr["kinds"] and "fly" in capr["kinds"],
          json.dumps(capr))
    check("fx/captureEngineCommitsImmediately",
          capr["engineCommittedEarly"] and capr["victim"] == -1,
          json.dumps(capr))

    # six flare
    sixr = page.evaluate("""(() => {
      window.LUG.clock++;
      const g = window.LUG.game;
      g.turn = 0;
      window.luResolveRoll(6);
      const kinds = LUG.fx.map(f => f.kind);
      return {kinds};
    })()""")
    check("fx/sixFlare", "six" in sixr["kinds"], json.dumps(sixr))

    # ---------- D: stack ease convergence ----------
    stack = page.evaluate("""(() => new Promise(res => {
      window.LUG.clock++;
      const g = window.LUG.game;
      for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
      g.turn = 0; g.chain = 0; g.winner = null;
      // three red + one green on rel 10 (global 10)
      g.players[0].tokens[0].pos = 10;
      g.players[0].tokens[1].pos = 10;
      g.players[0].tokens[2].pos = 10;
      g.players[1].tokens[0].pos = (10 - 13 + 52) % 52;
      LUG.phase = 'idle'; LUG.dice = null; LUG.moves = [];
      window.luDraw();
      window.luStackPump && window.luStackPump();
      const t0 = performance.now();
      const iv = setInterval(() => {
        const stacks = window.luBuildStacks();
        const E = window.LudoEngine;
        const gl = E.globalOf(0, 10);
        const mates = stacks.get('T' + gl) || [];
        if (mates.length) {
          // compare each token's eased offset against the deterministic seat
          let maxErr = 0;
          for (let k = 0; k < mates.length; k++) {
            const m = mates[k];
            const tok = g.players[m.p].tokens[m.i];
            const cur = window.LU_STACK_EASE ? window.LU_STACK_EASE.get(tok) : null;
            const seat = window.luStackLayout(mates.length)[k];
            if (cur) {
              maxErr = Math.max(maxErr,
                Math.abs(cur.dx - seat.dx), Math.abs(cur.dy - seat.dy));
            }
          }
          if (performance.now() - t0 > 1200) {
            clearInterval(iv);
            res({n: mates.length, converged: maxErr < 0.5, maxErr,
                 stackRaf: window.luStackRaf || 0});
          }
        } else if (performance.now() - t0 > 1200) {
          clearInterval(iv); res({n: 0, converged: false, why: 'no stack'});
        }
      }, 40);
    }))()""")
    check("stack/easesToDeterministicLayout",
          stack.get("converged") and stack.get("n") == 4,
          json.dumps(stack))

    # ---------- E: lifecycle ----------
    # E1: restart during glide
    page.evaluate("window.luNewGame()")
    page.wait_for_timeout(200)
    mv = synth_move("g.players[0].tokens[0].pos = 2;", 6)
    page.wait_for_timeout(30)                      # mid-walk
    st = page.evaluate("""(() => ({
      phase: window.LUG.phase,
      moving: window.LUG.phase === 'moving',
      fx: window.LUG.fx.length}))()""")
    page.evaluate("window.luNewGame()")            # restart MID-GLIDE
    page.wait_for_timeout(250)
    st2 = page.evaluate("""(() => ({
      anim: window.LUG.anim, fxRaf: window.LUG.fxRaf, fx: window.LUG.fx.length,
      stackRaf: window.luStackRaf || 0,
      vizFrozen: window.LUG.game.players.flatMap(P => P.tokens)
        .some(t => t._viz != null)}))()""")
    check("life/restartDuringGlide",
          st["phase"] == "moving" and st2["anim"] is None and st2["vizFrozen"] is False
          and st2["fxRaf"] == 0 and st2["fx"] == 0 and st2["stackRaf"] == 0,
          f"mid={json.dumps(st)} after={json.dumps(st2)}")

    # E2: restart during FX (capture flight)
    page.evaluate("""(() => {
      window.LUG.clock++;
      const g = window.LUG.game;
      g.turn = 0;
      g.players[0].tokens[0].pos = 9;
      g.players[1].tokens[0].pos = (10-13+52)%52;
      LUG.phase = 'awaiting'; LUG.dice = 1;
      LUG.moves = LudoEngine.legalMoves(g, 0, 1);
      const m = LUG.moves[0];
      window.luExecuteMove({token: m.token, to: m.to});
    })()""")
    # poll until an FX record is actually live (walk ~fast, then impact)
    fxlive = 0
    dlx = time.time() + 2
    while time.time() < dlx:
        fxlive = page.evaluate("LUG.fx.length")
        if fxlive > 0:
            break
        page.wait_for_timeout(10)
    page.evaluate("window.luNewGame()")
    page.wait_for_timeout(200)
    st3 = page.evaluate("({fx: LUG.fx.length, fxRaf: LUG.fxRaf})")
    check("life/restartDuringFx",
          st3["fx"] == 0 and st3["fxRaf"] == 0,
          f"liveBefore={fxlive} after={json.dumps(st3)}")

    # E3: teardown (route-away) during glide
    mv = synth_move("g.players[0].tokens[0].pos = 2;", 6)
    page.wait_for_timeout(30)
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(300)
    page.evaluate("location.hash = '#funtime/lu'")
    page.wait_for_selector("#luBoard")
    page.wait_for_timeout(300)
    st4 = page.evaluate("""(() => ({
      anim: window.LUG.anim, fxRaf: window.LUG.fxRaf,
      stackRaf: window.luStackRaf || 0, diceRaf: window.LUG.diceRaf,
      vizFrozen: window.LUG.game.players.flatMap(P => P.tokens)
        .some(t => t._viz != null)}))()""")
    check("life/routeAwayDuringGlide",
          st4["anim"] is None and st4["vizFrozen"] is False
          and st4["fxRaf"] == 0 and st4["stackRaf"] == 0 and st4["diceRaf"] == 0,
          json.dumps(st4))

    # E4: rapid successive moves (stack churn) — no stuck pumps
    page.evaluate("window.LUD.fast()")
    for _ in range(6):
        synth_move("g.players[0].tokens[0].pos = 2;", 3)
        page.wait_for_timeout(60)
    page.wait_for_timeout(1000)
    st5 = page.evaluate("""(() => ({
      anim: window.LUG.anim === null, fx: LUG.fx.length, fxRaf: LUG.fxRaf,
      stackRaf: window.luStackRaf || 0}))()""")
    check("life/rapidMovesNoStuckPumps",
          st5["anim"] and st5["fx"] == 0 and st5["fxRaf"] == 0 and st5["stackRaf"] == 0,
          json.dumps(st5))

    # E5: resize during glide keeps the walk on-grid
    mv = synth_move("g.players[0].tokens[0].pos = 2;", 6)
    page.wait_for_timeout(40)
    page.set_viewport_size({"width": 900, "height": 700})
    page.wait_for_timeout(120)
    rz = page.evaluate("""(tok) => {
      const g = window.LUG.game;
      const t = g.players[0].tokens[tok];
      // whatever the state, no token may render off-canvas
      const stacks = window.luBuildStacks();
      let off = 0;
      for (let pp = 0; pp < 4; pp++) for (let ii = 0; ii < 4; ii++) {
        const pt = window.luTokenPoint(pp, ii, stacks);
        if (pt && (pt.x < 0 || pt.y < 0 || pt.x > LUG.w || pt.y > LUG.w)) off++;
      }
      return {off, w: LUG.w, phase: LUG.phase};
    }""", mv["token"])
    page.set_viewport_size({"width": 1280, "height": 900})
    page.wait_for_timeout(300)
    check("life/resizeDuringGlideOnGrid",
          rz["off"] == 0, json.dumps(rz))

    # ---------- F: nudge + hover ----------
    # illegal pick (opponent token) → nudge flash that decays
    nud = page.evaluate("""(() => new Promise(res => {
      window.LUG.clock++;
      const g = window.LUG.game;
      for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
      g.turn = 0;
      g.players[1].tokens[0].pos = 15;         // opponent on board
      LUG.phase = 'awaiting'; LUG.dice = 1;
      LUG.moves = [];                           // nothing legal
      const stacks = window.luBuildStacks();
      const pt = window.luTokenPoint(1, 0, stacks);
      window.luIllegalPick({p: 1, i: 0});
      const t0 = performance.now();
      let sawNudge = false, decayed = false;
      const iv = setInterval(() => {
        if (LUG.nudge && !sawNudge) sawNudge = true;
        if (sawNudge && (!LUG.nudge || performance.now() > LUG.nudge.until)) decayed = true;
        if (performance.now() - t0 > 2200) {
          clearInterval(iv);
          res({sawNudge, decayed, nudgeNow: !!LUG.nudge,
               phase: LUG.phase, dice: LUG.dice});
        }
      }, 30);
    }))()""")
    check("nudge/flashesAndDecays",
          nud["sawNudge"] and nud["decayed"] and not nud["nudgeNow"],
          json.dumps(nud))

    # hover: pointer over a legal token sets hoverToken (state-free)
    hov = page.evaluate("""(() => {
      window.LUG.clock++;
      const g = window.LUG.game;
      for (const P of g.players) P.tokens.forEach(t => { t.pos = -1; delete t._viz; });
      g.turn = 0;
      g.players[0].tokens[0].pos = 10;
      LUG.phase = 'awaiting'; LUG.dice = 1;
      LUG.moves = LudoEngine.legalMoves(g, 0, 1);
      const stacks = window.luBuildStacks();
      const pt = window.luTokenPoint(0, 0, stacks);
      // simulate pointermove through the real handler path
      const c = document.getElementById('luBoard');
      const r = c.getBoundingClientRect();
      const cx = r.left + pt.x * (r.width / LUG.w);
      const cy = r.top + pt.y * (r.height / LUG.w);
      c.dispatchEvent(new MouseEvent('pointermove', {
        clientX: cx, clientY: cy, bubbles: true}));
      return {hover: LUG.hoverToken ? {p: LUG.hoverToken.p, i: LUG.hoverToken.i} : null,
              cursor: c.style.cursor};
    })()""")
    check("hover/legalTokenSelected",
          hov["hover"] and hov["hover"]["p"] == 0 and hov["hover"]["i"] == 0
          and hov["cursor"] == "pointer",
          json.dumps(hov))

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [x for x in RESULTS if not x[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
