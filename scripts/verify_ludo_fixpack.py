#!/usr/bin/env python3
"""Phase 5.6 fix-pack — three Ludo regressions, engine + UI verified.

  FIX 1 · INVALID SIX (engine rule)
     A. Three tokens finished, last token needs 2 steps, dice shows 6:
        no legal move exists → NO extra turn is awarded; the dice pass
        to the next eligible player.
     B. Same board, last token needs exactly 6: the finishing move IS
        legal (and finishing on it still resolves the game correctly).
     C. The classic six chain is untouched: a playable six still grants
        an extra roll, three-in-a-row still caps.

  FIX 2 · CAPTURE BONUS (turn resolution)
     D. Engine: a capture made on a NON-six roll grants exactly ONE
        extra roll to the capturer (previously the turn passed on).
     E. Engine: the bonus applies exactly once — the bonus roll that
        follows (any value, no capture) passes the turn on normally.
     F. UI, human seat: capture on a 3 → same player rolls again.
     G. UI, AI seat: capture by AI → same AI rolls again.
     H. Regression: a plain non-capturing non-six roll still passes.

  FIX 3 · ACTIVE-PLAYER DICE PLACEMENT (UI)
     I.  Exactly ONE .lu-die renders across all player panels — inside
         the CURRENT player's panel only.
     J.  Inactive panels carry NO empty dice placeholder (the old
         .lu-p-die-empty is gone everywhere).
     K.  Panel dimensions stay stable when the turn changes (width and
         height of every panel identical before/after the hand-off).
     L.  Dice ownership moves with every turn change (seat i → i+1),
         including across inactive seats in a 3-player game.

Run with the dashboard serving on 127.0.0.1:8907 (--no-auth is fine).
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

    def mount(seats=None, humans=1):
        page.goto(BASE + "/console")
        page.wait_for_selector("#viewRoot")
        page.evaluate("location.hash = '#funtime/lu'")
        page.wait_for_selector("#luBoard")
        page.wait_for_timeout(400)
        if seats is not None:
            # configure a custom game via the lobby state, then start
            page.evaluate(f"""(() => {{
              window.LUG.seatCount = {seats};
              window.LUG.humanSeats = {humans};
              window.luLobSync();
            }})()""")
        page.evaluate("window.luLobbyStart()")
        page.wait_for_timeout(400)
        page.evaluate("window.LUD.fast()")
        page.wait_for_timeout(150)

    # ==================================================================
    # FIX 1 + FIX 2 — pure engine rules (deterministic, no UI timing)
    # ==================================================================
    mount()

    r = page.evaluate("""(() => {
      const E = window.LudoEngine, out = {};

      // ---- FIX 1A: invalid six must NOT grant an extra turn --------
      // Red: 3 tokens FINISHED, last token on rel 54 (needs exactly 2).
      const g = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
      g.players[0].tokens[0].pos = 56;
      g.players[0].tokens[1].pos = 56;
      g.players[0].tokens[2].pos = 56;
      g.players[0].tokens[3].pos = 54;   // needs 2 → a 6 overshoots
      const mv6 = E.legalMoves(g, 0, 6);
      out.invalidSix_noLegalMove = mv6.length === 0;
      E.noteRoll(g, 6);                  // the roll happened
      E.noteNoMove(g);                   // …and was unplayable
      out.invalidSix_noExtra = E.grantsExtraRoll(g, 6) === false;
      E.advanceTurn(g);
      out.invalidSix_nextPlayer = g.turn === 1;

      // ---- FIX 1B: needing EXACTLY 6 is a legal finish -------------
      // NOTE: pos is set directly, so `home` must be seeded to match
      // (applyMove only increments home when IT performs the finish).
      const g2 = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
      g2.players[0].tokens[0].pos = 56; g2.players[0].home = 1;
      g2.players[0].tokens[1].pos = 56; g2.players[0].home = 2;
      g2.players[0].tokens[2].pos = 56; g2.players[0].home = 3;
      g2.players[0].tokens[3].pos = 50;  // needs exactly 6
      const mvF = E.legalMoves(g2, 0, 6);
      out.exactSix_isLegal = mvF.length === 1 && mvF[0].token === 3 && mvF[0].to === 56;
      const ev = E.applyMove(g2, 0, 3, 6);
      out.exactSix_finishes = ev.finished === true && g2.players[0].home === 4;
      out.exactSix_wins = E.checkWinner(g2) === 0;

      // ---- FIX 1C: the classic six chain is untouched --------------
      const g3 = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
      E.noteRoll(g3, 6);
      out.six_chainStillGrants = E.grantsExtraRoll(g3, 6) === true;
      E.noteRoll(g3, 6); E.noteRoll(g3, 6);   // chain now 3
      out.six_chainCapsAtThree = E.grantsExtraRoll(g3, 6) === false;

      // ---- FIX 2D: capture on a NON-six grants exactly one bonus ----
      // Geometry (verified): red rel 0 +5 lands global 5 — NOT a star.
      // Green shares global 5 at green rel 44.
      const g4 = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
      g4.players[0].tokens[0].pos = 0;
      g4.players[1].tokens[0].pos = 44;   // same global square as red's landing
      const victims = E.victimsAt(g4, 0, 5);
      out.capture_victimPresent = victims.length === 1 && victims[0].player === 1;
      /* REAL UI ORDER: noteRoll at roll time, applyMove at move time. */
      E.noteRoll(g4, 5);                       // the five is rolled…
      const ev4 = E.applyMove(g4, 0, 0, 5);    // …then the capture happens
      out.capture_happened = ev4.captured.length === 1;
      out.capture_bonusSet = g4.bonus === true;
      out.capture_grantsExtra = E.grantsExtraRoll(g4, 5) === true;

      // ---- FIX 2E: the bonus applies EXACTLY ONCE ------------------
      // Take the bonus roll (a plain 4, no capture): turn must pass on.
      E.noteRoll(g4, 4);                       // consumes the bonus
      const movesB = E.legalMoves(g4, 0, 4);
      if (movesB.length) E.applyMove(g4, 0, movesB[0].token, 4);
      out.bonus_onlyOnce = E.grantsExtraRoll(g4, 4) === false;
      E.advanceTurn(g4);
      out.bonus_thenPasses = g4.turn === 1;

      // ---- FIX 2H: plain non-capture non-six still passes ----------
      const g5 = E.createGame(['human','ai','ai','ai'], {seats:[0,1,2,3]});
      g5.players[0].tokens[0].pos = 10;
      E.noteRoll(g5, 3);                   // roll first (real order)
      const ev5 = E.applyMove(g5, 0, 0, 3);    // plain walk, no capture
      out.plainRoll_passes = E.grantsExtraRoll(g5, 3) === false;

      return out;
    })()""")
    for k, v in r.items():
        check("engine/" + k, v, json.dumps({k: v}) if not v else "")

    # ==================================================================
    # FIX 2F — UI: HUMAN capture grants an extra roll
    # ==================================================================
    mount()
    ui_cap = page.evaluate("""(() => {
      const E = window.LudoEngine, LUG = window.LUG;
      const g = LUG.game;
      // deterministic board: red (human) captures green on a 3
      // geometry: red rel 0 +3 → global 3 (not a star); green rel 42
      // shares global 3.
      g.chain = 0; g.bonus = false; g.noMove = false;
      g.players.forEach(P => P.tokens.forEach(t => t.pos = -1));
      g.players[0].tokens[0].pos = 0;
      g.players[1].tokens[0].pos = 42;
      g.turn = 0;
      LUG.phase = 'idle'; LUG.dice = null; LUG.moves = [];
      return { turn: g.turn, kind: g.players[0].kind };
    })()""")
    page.evaluate("window.LUD.force(3)")       # human rolls the 3
    page.wait_for_timeout(300)
    # pick the capturing token (red token 0, from yard? no: from rel 2)
    st = page.evaluate("({phase: window.LUG.phase, moves: window.LUG.moves.length, turn: window.LUG.game.turn})")
    check("ui/captureHuman_movesOffered", st["moves"] >= 1 and st["turn"] == 0, json.dumps(st))
    page.evaluate("""(() => {
      const LUG = window.LUG;
      const mv = LUG.moves.find(m => m.token === 0) || LUG.moves[0];
      window.__pick = mv;
    })()""")
    # click the token via the exposed geometry helper
    pt = page.evaluate("""(() => {
      const LUG = window.LUG, E = window.LudoEngine;
      const mv = window.__pick;
      const g = LUG.game, p = g.turn, tok = mv.token;
      const pos = g.players[p].tokens[tok].pos;
      let q;
      if (pos === -1) { const rc = E.YARD_SLOTS[p][tok]; q = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
      else { const rc = E.cellOf(p,pos); q = {x:(rc[1]+.5)*LUG.cell, y:(rc[0]+.5)*LUG.cell}; }
      const c = document.getElementById('luBoard'), r = c.getBoundingClientRect();
      return { x: r.left + q.x*(r.width/LUG.w), y: r.top + q.y*(r.height/LUG.h) };
    })()""")
    page.mouse.click(pt["x"], pt["y"])
    page.wait_for_timeout(900)                 # glide + settle (fast mode)
    after = page.evaluate("""(() => ({
      turn: window.LUG.game.turn,
      phase: window.LUG.phase,
      bonus: window.LUG.game.bonus,
      victimHome: window.LUG.game.players[1].tokens[0].pos === -1,
      msg: document.getElementById('luMsg').textContent }))()""")
    check("ui/captureHuman_victimSentHome", after["victimHome"], json.dumps(after))
    check("ui/captureHuman_extraRollGranted",
          after["turn"] == 0 and after["bonus"] == True,
          json.dumps(after))

    # ==================================================================
    # FIX 2G — UI: AI capture also grants the extra roll
    # ==================================================================
    mount()
    page.evaluate("""(() => {
      const LUG = window.LUG, g = LUG.game;
      g.chain = 0; g.bonus = false; g.noMove = false;
      g.players.forEach(P => P.tokens.forEach(t => t.pos = -1));
      // GREEN (AI, seat 1) captures RED (human) on a 4:
      // green rel 0 +4 → global 17 (not a star); red shares global 17
      // at red rel 17.
      g.players[1].tokens[0].pos = 0;
      g.players[0].tokens[0].pos = 17;
      g.turn = 1;
      LUG.phase = 'idle'; LUG.dice = null; LUG.moves = [];
      // deterministic AI roll: stub rollDice to always throw the 4
      window.__aiRollValue = 4;
      const orig = window.LudoEngine.rollDice.bind(window.LudoEngine);
      window.LudoEngine.rollDice = () => window.__aiRollValue;
      window.__restoreRoll = () => { window.LudoEngine.rollDice = orig; };
      // instrument applyMove: snapshot state AT THE CAPTURE COMMIT —
      // the bonus roll that follows may resolve (and pass the turn on)
      // before Python can poll, so the moment itself is what we assert.
      window.__capSeen = null;
      const origApply = window.LudoEngine.applyMove.bind(window.LudoEngine);
      window.LudoEngine.applyMove = (st, pl, tk, dc) => {
        const ev = origApply(st, pl, tk, dc);
        if (ev.captured.length) window.__capSeen = { turn: st.turn, bonus: st.bonus };
        return ev;
      };
      // START the AI's turn through the real controller so its timers fire
      window.luStartTurn();
    })()""")
    # poll for the capture marker (think → roll → glide → commit)
    cap_seen = None
    deadline = time.time() + 8
    while time.time() < deadline:
        page.wait_for_timeout(120)
        cap_seen = page.evaluate("window.__capSeen")
        if cap_seen: break
    ai_after = page.evaluate("""(() => {
      if (window.__restoreRoll) window.__restoreRoll();
      const LUG = window.LUG, g = LUG.game;
      return { turn: g.turn, bonus: g.bonus,
               victimHome: g.players[0].tokens[0].pos === -1,
               greenPos: g.players[1].tokens[0].pos,
               phase: LUG.phase,
               msg: document.getElementById('luMsg').textContent };
    })()""")
    check("ui/captureAI_victimSentHome", ai_after["victimHome"], json.dumps(ai_after))
    check("ui/captureAI_extraRollGranted",
          cap_seen is not None and cap_seen["turn"] == 1 and cap_seen["bonus"] == True,
          f"captureMoment={cap_seen} after={json.dumps(ai_after)}")

    # ==================================================================
    # FIX 1 (UI level) — invalid six passes the turn, no extra roll
    # ==================================================================
    mount()
    page.evaluate("""(() => {
      const LUG = window.LUG, g = LUG.game;
      g.chain = 0; g.bonus = false; g.noMove = false;
      // red: 3 finished + last token needing 2 → a six is UNPLAYABLE.
      // Give every OTHER seat one live track token so their turns are
      // always playable whatever they roll (the hand-off must land on
      // an ELIGIBLE next player, not stall or bounce back to red).
      g.players[0].tokens[0].pos = 56;
      g.players[0].tokens[1].pos = 56;
      g.players[0].tokens[2].pos = 56;
      g.players[0].tokens[3].pos = 54;   // needs exactly 2
      g.players[1].tokens[0].pos = 10;
      g.players[2].tokens[0].pos = 10;
      g.players[3].tokens[0].pos = 10;
      g.players[0].home = 3;
      g.turn = 0;
      LUG.phase = 'idle'; LUG.dice = null; LUG.moves = [];
    })()""")
    forced = page.evaluate("window.LUD.force(6)")
    check("ui/invalidSix_forceAccepted", forced is True, f"force={forced}")
    # wait for the no-move settle window to elapse and the hand-off to land
    deadline = time.time() + 5
    inv = None
    while time.time() < deadline:
        page.wait_for_timeout(150)
        inv = page.evaluate("""(() => ({
          turn: window.LUG.game.turn,
          noMove: window.LUG.game.noMove,
          phase: window.LUG.phase,
          dice: window.LUG.dice }))()""")
        if inv["turn"] != 0:
            break
    check("ui/invalidSix_turnPassesToNext",
          inv and inv["turn"] == 1, json.dumps(inv))
    check("ui/invalidSix_noExtraRoll",
          inv and inv["turn"] != 0, json.dumps(inv))

    # ==================================================================
    # FIX 3 — active-player-only dice placement
    # ==================================================================
    mount()                                     # default 4 seats, seat 0 human
    dice_state = page.evaluate("""(() => {
      const panels = [...document.querySelectorAll('#luPanels .lu-panel')];
      return {
        count: panels.length,
        diceOwners: panels.filter(el => el.querySelector('.lu-die'))
                          .map(el => Number(el.dataset.seat)),
        emptyPlaceholders: document.querySelectorAll(
          '#luPanels .lu-p-die-empty').length,
        slots: document.querySelectorAll('#luPanels .lu-p-die-slot').length,
        turn: window.LUG.game.turn,
        sizes: panels.map(el => { const r = el.getBoundingClientRect();
          return { w: Math.round(r.width*10)/10, h: Math.round(r.height*10)/10 }; }),
      };
    })()""")
    check("fix3/oneDieOnly_currentPlayer",
          dice_state["diceOwners"] == [dice_state["turn"]]
          and len(dice_state["diceOwners"]) == 1,
          json.dumps(dice_state["diceOwners"]))
    check("fix3/noEmptyPlaceholders", dice_state["emptyPlaceholders"] == 0,
          f"{dice_state['emptyPlaceholders']} empty placeholders")
    check("fix3/inactivePanelsHaveInvisibleSlot",
          dice_state["slots"] == dice_state["count"] - 1,
          f"slots={dice_state['slots']} panels={dice_state['count']}")
    sizes_before = dice_state["sizes"]

    # turn change: hand the dice to seat 1 (AI) and re-measure
    page.evaluate("""(() => {
      const LUG = window.LUG, g = LUG.game;
      window.LudoEngine.advanceTurn(g);
      LUG.phase = 'idle'; LUG.dice = 6; LUG.moves = [];
      window.luRenderHud(); window.luRenderControls();
    })()""")
    page.wait_for_timeout(200)
    after = page.evaluate("""(() => {
      const panels = [...document.querySelectorAll('#luPanels .lu-panel')];
      return {
        diceOwners: panels.filter(el => el.querySelector('.lu-die'))
                          .map(el => Number(el.dataset.seat)),
        turn: window.LUG.game.turn,
        sizes: panels.map(el => { const r = el.getBoundingClientRect();
          return { w: Math.round(r.width*10)/10, h: Math.round(r.height*10)/10 }; }),
      };
    })()""")
    check("fix3/diceMovesWithTurn",
          after["diceOwners"] == [after["turn"]] and after["turn"] == 1,
          json.dumps({"owners": after["diceOwners"], "turn": after["turn"]}))
    check("fix3/panelDimensionsStable",
          after["sizes"] == sizes_before,
          f"before={sizes_before} after={after['sizes']}")

    # L: ownership moves across EVERY seat in a 3-player game
    # (seat 3 inactive — the die must skip it entirely)
    mount(seats=3, humans=1)
    owners = []
    for expect in (0, 1, 2, 0):
        got = page.evaluate("""(() => {
          const LUG = window.LUG, g = LUG.game;
          g.turn = """ + str(expect) + """;
          LUG.phase = 'idle'; LUG.dice = 3; LUG.moves = [];
          window.luRenderHud();
          const panels = [...document.querySelectorAll('#luPanels .lu-panel')];
          return {
            owners: panels.filter(el => el.querySelector('.lu-die'))
                          .map(el => Number(el.dataset.seat)),
            n: panels.length };
        })()""")
        owners.append((expect, got["owners"], got["n"]))
    ok_l = all(o == [e] and n == 3 for e, o, n in owners)
    check("fix3/threePlayerGame_ownershipFollowsTurn", ok_l, json.dumps(owners))

    # ==================================================================
    # console errors across the whole run
    # ==================================================================
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))

    browser.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
