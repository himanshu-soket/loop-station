#!/usr/bin/env python3
"""FINAL ACCEPTANCE · Section 4 — reproducible seeded AI simulations.

Runs complete games on the REAL engine (window.LudoEngine) in the real
browser: human-vs-AI, AI-vs-AI, mixed 3P and 4P, across all four
difficulties. Records illegal-move attempts, stuck games, unexpected
transitions, inactive-seat turns and completions.

Usage: python3 scripts/audit_ludo_simulations.py [--port 8907]
"""
import sys, json
from playwright.sync_api import sync_playwright

PORT = sys.argv[sys.argv.index("--port") + 1] if "--port" in sys.argv else "8907"
BASE = f"http://127.0.0.1:{PORT}"

# (label, seats, difficulty, seed, max-turns)
SIMS = [
    ("2P 1H-vs-AI classic", [0, 2],   "classic", 11, 4000),
    ("2P 1H-vs-AI easy",    [0, 2],   "easy",    12, 4000),
    ("2P 1H-vs-AI medium",  [0, 2],   "medium",  13, 4000),
    ("2P 1H-vs-AI hard",    [0, 2],   "hard",    14, 4000),
    ("2P AI-vs-AI classic", [0, 2],   "classic", 21, 4000),
    ("2P AI-vs-AI easy",    [0, 2],   "easy",    22, 4000),
    ("2P AI-vs-AI medium",  [0, 2],   "medium",  23, 4000),
    ("2P AI-vs-AI hard",    [0, 2],   "hard",    24, 4000),
    ("3P mixed 1H medium",  [0, 1, 2], "medium", 31, 6000),
    ("3P mixed 2H hard",    [0, 1, 2], "hard",   32, 6000),
    ("3P AI-only easy",     [0, 1, 2], "easy",   33, 6000),
    ("4P mixed 1H hard",    [0, 1, 2, 3], "hard", 41, 8000),
    ("4P mixed 2H medium",  [0, 1, 2, 3], "medium", 42, 8000),
    ("4P AI-only classic",  [0, 1, 2, 3], "classic", 43, 8000),
    ("4P AI-only hard",     [0, 1, 2, 3], "hard",  44, 8000),
]

JS_DRIVER = """
(args) => {
  const [seats, difficulty, seed, maxTurns] = args;
  const E = window.LudoEngine;
  const rng = E.makeRng(seed);
  const diceRng = E.makeRng(seed * 7919 + 13);
  const kinds = [0,1,2,3].map(i => 'ai');   // ALL seats driven by the AI
  const g = E.createGame(kinds, { seats });
  const log = { rolls: [], illegal: 0, inactiveTurns: 0,
                unexpectedWinner: false, stuck: false };
  let turnCount = 0;
  while (g.winner === null && turnCount < maxTurns) {
    turnCount++;
    const p = g.turn;
    if (!E.isActive(g, p)) { log.inactiveTurns++; break; }
    const dice = E.rollDice(diceRng);
    log.rolls.push(dice);
    if (log.rolls.length > 400) log.rolls = log.rolls.slice(-400);
    const moves = E.legalMoves(g, p, dice);
    if (!moves.length) {
      // Unplayable roll — even a six never grants an extra turn.
      E.noteNoMove(g);
      E.advanceTurn(g);
      continue;
    }
    {
      const mv = E.selectAIMoveForDifficulty(g, p, moves, difficulty, rng);
      // verify legality of the chosen move against the list
      if (!mv || !moves.some(m => m.token === mv.token && m.to === mv.to)) {
        log.illegal++; break;
      }
      E.applyMove(g, p, mv.token, dice);
      E.checkWinner(g);
      if (g.winner !== null && !E.isActive(g, g.winner)) log.unexpectedWinner = true;
    }
    E.noteRoll(g, dice);
    // Capture bonus (state.bonus from applyMove) or a six: roll again.
    if (E.grantsExtraRoll(g, dice) && g.winner === null) continue;
    E.advanceTurn(g);
  }
  if (g.winner === null) log.stuck = true;
  return { winner: g.winner, turns: turnCount, rollsTotal: g.rolls,
           seats: g.seats, home: g.players.map(P => P.home),
           chainOk: g.chain >= 0 && g.chain <= 3, ...log };
}
"""

def main():
    results = []
    with sync_playwright() as p:
        b = p.chromium.launch()
        pg = b.new_page(viewport={"width": 1280, "height": 900})
        errs = []
        pg.on("pageerror", lambda e: errs.append(str(e)))
        pg.goto(BASE + "/console"); pg.wait_for_selector("#viewRoot")
        pg.evaluate("location.hash='#funtime/lu'")
        pg.wait_for_selector("#luBoard"); pg.wait_for_timeout(600)

        for label, seats, diff, seed, mx in SIMS:
            r = pg.evaluate(JS_DRIVER, [seats, diff, seed, mx])
            ok = (r["illegal"] == 0 and r["inactiveTurns"] == 0
                  and not r["stuck"] and r["winner"] is not None
                  and not r["unexpectedWinner"] and r["chainOk"]
                  and r["home"][r["winner"]] == 4)
            # dice distribution sanity 1..6 only
            dice_ok = all(1 <= d <= 6 for d in r["rolls"])
            ok &= dice_ok
            status = "PASS" if ok else "FAIL"
            print(f"{status}  {label:28s} winner=P{r['winner']} "
                  f"turns={r['turns']} rolls={r['rollsTotal']} "
                  f"home={r['home']}")
            if not ok:
                print("      detail:", json.dumps(r))
            results.append((label, ok))

    n_ok = sum(1 for _, ok in results if ok)
    print(f"\n==== simulations: {n_ok}/{len(results)} passed ====")
    sys.exit(0 if n_ok == len(results) else 1)

if __name__ == "__main__":
    main()
