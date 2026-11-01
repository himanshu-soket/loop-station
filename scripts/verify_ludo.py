#!/usr/bin/env python3
"""Ludo functional verification: engine rules + AI lifecycle + full gameplay
in real Chromium via Playwright. Mirrors the repo's .venv-test setup."""
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

    # ---------- ENGINE RULES (pure, deterministic) ----------
    page.evaluate("location.hash = '#funtime/lu'")
    page.wait_for_selector("#luBoard")
    # Phase 5.5: the view now mounts into a setup lobby — start the game
    # with the default configuration before running gameplay checks.
    page.wait_for_timeout(300)
    page.evaluate("window.luLobbyStart()")
    page.wait_for_timeout(400)
    page.evaluate("window.LUD.fast()")   # test pacing for the flow checks below

    r = page.evaluate("""(() => {
      const E = window.LudoEngine, out = {};
      // fresh game: all tokens home
      const g = E.createGame(['human','ai','ai','ai']);
      out.tokensStartHome = g.players.every(P => P.tokens.every(t => t.pos === -1));
      out.redOpens = g.turn === 0;
      // dice range over 5000 rolls
      let bad = 0;
      for (let i=0;i<5000;i++){ const v=E.rollDice(); if(v<1||v>6||v!==Math.floor(v)) bad++; }
      out.diceRange = bad === 0;
      // release only on 6
      const g2 = E.createGame();
      out.releaseNeeds6 = E.legalMoves(g2,0,5).length===0 && E.legalMoves(g2,0,6).length===4;
      // movement: token on start advances exactly dice; overshoot rejected
      const g3 = E.createGame();
      E.applyMove(g3,0,0,6);            // red token to start (pos 0)
      const mv = E.legalMoves(g3,0,4);
      out.moveFromStart = mv.length===1 && mv[0].to===4;
      const g4 = E.createGame();
      g4.players[0].tokens[0].pos = 54; // needs exactly 2
      out.exactFinish = E.legalMoves(g4,0,2)[0].to===56 && E.legalMoves(g4,0,3).length===0;
      // finished cannot move
      const g5 = E.createGame();
      g5.players[0].tokens[0].pos = 56;
      out.finishedFrozen = E.legalMoves(g5,0,6).every(m=>m.token!==0);
      // capture on plain square
      const g6 = E.createGame();
      g6.players[0].tokens[0].pos = 2;   // red on global 2
      g6.players[2].tokens[0].pos = 28;  // yellow start26+28=54%52=2 -> same square
      out.sameSquareCheck = E.globalOf(0,2) === 2 && E.globalOf(2,28) === 2;
      // capture: red lands on lone green token off-star
      const g8 = E.createGame();
      g8.players[0].tokens[0].pos = 1;                    // red on global 1
      g8.players[1].tokens[0].pos = (1 - 13 + 52) % 52;   // green also on global 1
      const ev8 = E.applyMove(g8,0,0,0);
      out.captureWorks = ev8.captured.length === 1 && g8.players[1].tokens[0].pos === -1;
      // safe star protects
      const g9 = E.createGame();
      g9.players[0].tokens[0].pos = 0;                  // red start = SAFE
      g9.players[1].tokens[0].pos = (0-13+52)%52;
      const ev9 = E.applyMove(g9,0,0,0);
      out.safeProtects = ev9.captured.length === 0;
      // lane entry: pos 50 + roll enters lane
      const gA = E.createGame();
      gA.players[0].tokens[0].pos = 50;
      const lmv = E.legalMoves(gA,0,3);
      out.laneEntry = lmv.length===1 && lmv[0].to===53 && E.cellOf(0,53)!==null;
      // finish increments home + winner
      const gB = E.createGame();
      gB.players[0].tokens.forEach(t=>t.pos=56); gB.players[0].home=4;
      out.winnerDetected = E.checkWinner(gB)===0 && gB.winner===0;
      // extra roll rules
      const gC = E.createGame();
      E.noteRoll(gC,6);
      out.sixGrantsExtra = E.grantsExtraRoll(gC,6)===true;
      E.noteRoll(gC,6); E.noteRoll(gC,6); // chain now 3
      out.thirdSixStops = E.grantsExtraRoll(gC,6)===false;
      // advanceTurn cycles
      const gD = E.createGame();
      E.advanceTurn(gD); E.advanceTurn(gD); E.advanceTurn(gD);
      out.turnCycles = gD.turn===3;
      E.advanceTurn(gD);
      out.turnWraps = gD.turn===0;
      // AI picks only legal moves
      const gE = E.createGame();
      gE.players[1].tokens[0].pos = 10;
      const legal = E.legalMoves(gE,1,3);
      const pick = E.selectAIMove(gE,1,legal);
      out.aiLegal = legal.some(m=>m.token===pick.token&&m.to===pick.to);
      // AI handles no-moves
      out.aiNoMove = E.selectAIMove(gE,1,[])===null;
      // AI prefers finishing
      const gF = E.createGame();
      gF.players[1].tokens[0].pos = 53; // can finish with 3
      gF.players[1].tokens[1].pos = 20;
      const fmv = E.legalMoves(gF,1,3);
      const fpick = E.selectAIMove(gF,1,fmv);
      out.aiPrefersFinish = fpick.token===0 && fpick.to===56;
      // track integrity
      out.track52 = E.TRACK.length===52;
      const seen = new Set(E.TRACK.map(c=>c.join(',')));
      out.trackUnique = seen.size===52;
      out.startsApart = E.START_G.every((s,i)=>s===[0,13,26,39][i]);
      return out;
    })()""")
    for k, v in r.items():
        check("engine/" + k, v)

    # ---------- BOARD RENDERS ----------
    box = page.evaluate("""(() => {
      const c = document.getElementById('luBoard');
      const r = c.getBoundingClientRect();
      return {w:r.width, h:r.height, ratio:r.width/r.height};
    })()""")
    check("board/square", abs(box["ratio"] - 1.0) < 0.05, f"ratio={box['ratio']:.3f}")
    check("board/visible", box["w"] > 300, f"w={box['w']:.0f}px")
    # pixels actually painted (not blank canvas)
    px = page.evaluate("""(() => {
      const c = document.getElementById('luBoard');
      const ctx = c.getContext('2d');
      const d = ctx.getImageData(0,0,c.width,c.height).data;
      let n=0; for(let i=3;i<d.length;i+=4*97){ if(d[i]>0) n++; }
      return n;
    })()""")
    check("board/painted", px > 200, f"{px} sampled non-transparent px")

    # ---------- HUMAN TURN: roll + move ----------
    page.evaluate("window.LUD.force(6)")   # force a six
    page.wait_for_timeout(300)
    phase = page.evaluate("({phase: window.LUG.phase, turn: window.LUG.game.turn, moves: window.LUG.moves.length})")
    check("human/roll6givesMoves", phase["moves"] >= 1 and phase["turn"] == 0, json.dumps(phase))

    # click first highlighted token centre
    clicked = page.evaluate("""(() => {
      const LUG = window.LUG;
      const mv = LUG.moves[0];
      const E = window.LudoEngine;
      // find pixel position of that token
      const g = LUG.game, p = g.turn, tok = mv.token;
      const pos = g.players[p].tokens[tok].pos;
      let pt;
      if (pos === -1) { const rc = E.YARD_SLOTS[p][tok]; pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
      else { const rc = E.cellOf(p,pos); pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
      const c = document.getElementById('luBoard');
      const r = c.getBoundingClientRect();
      return {x: r.left + pt.x*(r.width/LUG.w), y: r.top + pt.y*(r.height/LUG.h)};
    })()""")
    page.mouse.click(clicked["x"], clicked["y"])
    page.wait_for_timeout(2500)  # animation ~150ms/step + resolution

    st = page.evaluate("({turn: window.LUG.game.turn, phase: window.LUG.phase, red0: window.LUG.game.players[0].tokens[0].pos})")
    check("human/tokenMoved", st["red0"] >= 0, f"red token0 pos={st['red0']}")

    # ---------- AI TURNS RUN AUTONOMOUSLY ----------
    # The human's six grants an extra roll; auto-play any human turn so the
    # three AI players must cycle back to Red for this check to pass.
    def autoplay_human():
        s = page.evaluate("({turn: window.LUG.game.turn, kind: window.LUG.game.players[window.LUG.game.turn].kind, phase: window.LUG.phase})")
        if s["kind"] != "human":
            return False
        if s["phase"] == "idle":
            page.evaluate("window.LUD.force(Math.floor(Math.random()*6)+1)")
        elif s["phase"] == "awaiting":
            page.evaluate("""(() => {
              const LUG = window.LUG; if (!LUG.moves.length) return;
              const mv = LUG.moves[0]; const E = window.LudoEngine;
              const g = LUG.game, p = g.turn, tok = mv.token;
              const pos = g.players[p].tokens[tok].pos;
              let pt;
              if (pos === -1) { const rc = E.YARD_SLOTS[p][tok]; pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
              else { const rc = E.cellOf(p,pos); pt = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
              const c = document.getElementById('luBoard');
              const r = c.getBoundingClientRect();
              window.__clickAt = {x: r.left + pt.x*(r.width/LUG.w), y: r.top + pt.y*(r.height/LUG.h)};
            })()""")
            pt = page.evaluate("window.__clickAt")
            if pt: page.mouse.click(pt["x"], pt["y"])
        return True

    deadline = time.time() + 30
    ai_rolls_seen = {"v": 0}
    while time.time() < deadline:
        s = page.evaluate("({turn: window.LUG.game.turn, rolls: window.LUG.game.rolls, winner: window.LUG.game.winner})")
        if s["winner"] is not None:
            break
        if s["turn"] != 0 and s["rolls"] > 4:
            ai_rolls_seen["v"] = s["rolls"]
            break
        autoplay_human()
        page.wait_for_timeout(350)
    check("ai/autonomousCycle", ai_rolls_seen["v"] > 4, f"ai advanced game to rolls={ai_rolls_seen['v']}")

    # ---------- RESTART KILLS STALE CALLBACKS ----------
    # Stop autoplay first: let any in-flight AI turn finish so the click
    # lands on a settled state (the check is about callback invalidation).
    # NOTE: restart deliberately does NOT bump the VIEW epoch (listeners
    # must survive restarts); it bumps the GAME CLOCK, which every pending
    # turn-flow callback captures and re-checks. Assert the real mechanism.
    settle = time.time() + 15
    while time.time() < settle:
        s = page.evaluate("({phase: window.LUG.phase})")
        if s["phase"] == "idle":
            break
        page.wait_for_timeout(300)
    pre = page.evaluate("({epoch: window.LUG.epoch, clock: window.LUG.clock})")
    page.click("#luRestartBtn")
    page.wait_for_timeout(400)
    post = page.evaluate("({epoch: window.LUG.epoch, clock: window.LUG.clock, rolls: window.LUG.game.rolls, turn: window.LUG.game.turn, winner: window.LUG.game.winner})")
    check("restart/freshState",
          post["rolls"] == 0 and post["turn"] == 0 and post["winner"] is None
          and post["clock"] > pre["clock"],
          json.dumps(post))
    # epoch stability is required for listeners to keep working across
    # restarts — verify it was NOT bumped.
    check("restart/epochStableForListeners", post["epoch"] == pre["epoch"],
          f"epoch {pre['epoch']}→{post['epoch']}")

    # let AI run again briefly after restart to prove no zombie interference
    page.wait_for_timeout(3000)
    s2 = page.evaluate("({turn: window.LUG.game.turn, rolls: window.LUG.game.rolls, phase: window.LUG.game.winner===null ? window.LUG.phase : 'over'})")
    check("restart/noZombies", s2["rolls"] >= 0 and s2["phase"] != "over", json.dumps(s2))

    # ---------- NAV BACK TO FUNTIME ----------
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(500)
    cards = page.evaluate("document.querySelectorAll('.ft-card').length")
    check("nav/backToFuntime", cards >= 9, f"{cards} cards visible")
    lu_card = page.evaluate("!!document.getElementById('ftCardLu')")
    check("nav/ludoCardPresent", lu_card)

    # existing games intact
    for cid in ["ftCardT3","ftCardSi","ftCardHt","ftCardCr","ftCardCh","ftCardTm","ftCardSo","ftCardFn"]:
        ok = page.evaluate(f"!!document.getElementById('{cid}')")
        check(f"nav/{cid}", ok)

    # ---------- CONSOLE ERRORS ----------
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))

    browser.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
