/* ============================================================
   FUN TIME · GAME 9 — LUDO (CLASSIC 4-PLAYER)
   Pure rules engine. DOM-free by design: this file owns nothing
   but the board geometry, the tokens and the rules. The UI layer
   (ludo-game.js) renders this state and never invents rules.

   Exposes one global: `LudoEngine` (classic script, matches the
   chess.js / solitaire-engine.js convention of the Fun Time arcade).

   ONE RULESET, applied identically to humans and AI:
     • 4 players × 4 tokens. Red, Green, Yellow, Blue.
     • A token leaves its yard ONLY on a 6 (classic rule).
     • 52-square shared track; each player's start is 13 apart.
     • A token walks 51 main squares (relative steps 0..50), then
       its own 5-square home lane (steps 51..55), then finishes
       (step 56). Overshooting 56 is ILLEGAL — exact count needed.
     • Landing on a lone opponent on a NON-safe square captures it:
       the victim returns to its yard. Own tokens may always stack.
     • Safe squares (each start + the square 8 past each start —
       8 stars in total) protect every token standing on them.
     • Rolling a 6 grants an extra roll — capped at THREE sixes in
       a row; the third six still moves, but the turn then passes.
     • A successful capture grants exactly ONE bonus roll to the
       capturing player (on top of any six rule).
     • Bringing a token to its exact final destination (FINISH) grants
       exactly ONE bonus roll to that player — same lifecycle as the
       capture bonus, and it never stacks with itself.
     • A roll with NO legal move never grants an extra turn — not
       even a six: the dice pass straight to the next player.
     • First player to bring all four tokens home wins.

   Coordinates: TRACK[g] = [row, col] on the classic 15×15 grid,
   generated from segments below (verified: 52 cells, starts at
   g = 0/13/26/39 sit beside their matching yards).
   ============================================================ */
(function (global) {
  'use strict';

  /* ---- board geometry ------------------------------------------------ */

  /** The 52-cell ring, clockwise, g=0 at Red's start (6,1). */
  const TRACK = (() => {
    const t = [];
    for (let c = 1; c <= 5; c++) t.push([6, c]);      // g 0-4  red arm, top row →
    for (let r = 5; r >= 0; r--) t.push([r, 6]);      // g 5-10 up, top arm left col
    t.push([0, 7]);                                    // g 11   top middle
    for (let r = 0; r <= 5; r++) t.push([r, 8]);      // g 12-17 down, top arm right col
    for (let c = 9; c <= 13; c++) t.push([6, c]);     // g 18-22 green arm, top row →
    t.push([6, 14]);                                   // g 23
    t.push([7, 14]);                                   // g 24   right middle
    for (let c = 14; c >= 9; c--) t.push([8, c]);     // g 25-30 yellow arm, bottom row ←
    for (let r = 9; r <= 14; r++) t.push([r, 8]);     // g 31-36 down, bottom arm right col
    t.push([14, 7]);                                   // g 37   bottom middle
    for (let r = 14; r >= 9; r--) t.push([r, 6]);     // g 38-43 blue arm, left col ↑
    for (let c = 5; c >= 0; c--) t.push([8, c]);      // g 44-49 red arm, bottom row ←
    t.push([7, 0]);                                    // g 50   left middle
    t.push([6, 0]);                                    // g 51
    return t;
  })();

  if (TRACK.length !== 52) throw new Error('ludo track must have 52 cells');

  /** Player order around the board: Red TL, Green TR, Yellow BR, Blue BL. */
  const PLAYERS = 4;
  const TOKENS = 4;
  const COLORS = ['red', 'green', 'yellow', 'blue'];
  const NAMES = ['Red', 'Green', 'Yellow', 'Blue'];

  /** Global track index of each player's start square. */
  const START_G = [0, 13, 26, 39];

  /** Star (safe) squares: every start plus the square 8 past it. */
  const SAFE_G = new Set([0, 8, 13, 21, 26, 34, 39, 47]);

  /** Home lanes (relative steps 51..55), innermost first. */
  const LANES = [
    [[7, 1], [7, 2], [7, 3], [7, 4], [7, 5]],   // red:    east toward centre
    [[1, 7], [2, 7], [3, 7], [4, 7], [5, 7]],   // green:  south
    [[7, 13], [7, 12], [7, 11], [7, 10], [7, 9]],// yellow: west
    [[13, 7], [12, 7], [11, 7], [10, 7], [9, 7]],// blue:   north
  ];

  /** Yard slot centres (grid units) for tokens waiting at home.
      Uniform relative offsets {1,4} in every quadrant: the 2×2 cluster
      is centred on the quadrant's centre (pixel (r0+3)·C) and all four
      yards are exact rotations of each other. */
  const YARD_SLOTS = [
    [[1, 1], [1, 4], [4, 1], [4, 4]],
    [[1, 10], [1, 13], [4, 10], [4, 13]],
    [[10, 10], [10, 13], [13, 10], [13, 13]],
    [[10, 1], [10, 4], [13, 1], [13, 4]],
  ];

  const LAST_MAIN = 50;   // highest relative step still on the shared track
  const FINISH = 56;      // relative step of the centre (token done)

  /* ---- pure helpers --------------------------------------------------- */

  /** Global track index for `steps` along `player`'s route (0..50). */
  function globalOf(player, steps) {
    return (START_G[player] + steps) % 52;
  }

  /** Grid [row,col] for any live position of `player`'s token. */
  function cellOf(player, steps) {
    if (steps < 0 || steps > FINISH) return null;
    if (steps === FINISH) return [7, 7];                    // centre pool
    if (steps > LAST_MAIN) return LANES[player][steps - 51]; // own lane
    return TRACK[globalOf(player, steps)];
  }

  /** Is this live position protected by a star? (lanes/centre are safe) */
  function isSafeStep(player, steps) {
    if (steps > LAST_MAIN) return true;
    return SAFE_G.has(globalOf(player, steps));
  }

  /* ---- state ----------------------------------------------------------- */

  /**
   * Fresh game.
   *
   * `kinds` is an array of 4 strings ('human' | 'ai'); defaults to one
   * human (Red) versus three AI. All tokens start in their yards (pos -1).
   * `turn` is the player to act; Red opens.
   *
   * Phase 4: `opts.seats` optionally restricts which of the four physical
   * board seats take part (e.g. [0,2] for a diagonal two-player game).
   * Inactive seats keep their colour geometry but never act: they are
   * skipped by advanceTurn(), excluded from checkWinner(), and the UI
   * renders them dimmed. Seat 0 is always active (the human seat).
   * Deterministic seat policy (documented):
   *   4 players → [0, 1, 2, 3]  Red, Green, Yellow, Blue (unchanged)
   *   3 players → [0, 1, 2]     Red, Green, Yellow
   *   2 players → [0, 2]        Red, Yellow (diagonal corners — standard
   *                             two-seat Ludo: opposite starts, maximum
   *                             track distance between the racers)
   */
  const DEFAULT_SEATS = [0, 1, 2, 3];
  function createGame(kinds, opts) {
    const k = Array.isArray(kinds) && kinds.length === PLAYERS
      ? kinds.slice() : ['human', 'ai', 'ai', 'ai'];
    /* Validate the requested seats: unique, in range, must include 0. */
    let seats = DEFAULT_SEATS;
    if (opts && Array.isArray(opts.seats)) {
      const want = opts.seats.filter(s => Number.isInteger(s)
        && s >= 0 && s < PLAYERS);
      const uniq = [...new Set(want)];
      if (uniq.length >= 2 && uniq.includes(0)) seats = uniq.sort((a, b) => a - b);
    }
    return {
      players: k.map((kind, i) => ({
        kind,
        active: seats.includes(i),   // false → seat never acts or wins
        tokens: Array.from({ length: TOKENS }, () => ({ pos: -1 })),
        home: 0,            // tokens finished
      })),
      seats,                // active seat indices, ascending
      turn: 0,
      chain: 0,             // consecutive sixes rolled this turn
      bonus: false,         // capture bonus roll pending (one per roll)
      finishBonus: false,   // finish bonus roll pending (one per roll)
      noMove: false,        // current roll had zero legal moves
      winner: null,         // player index once decided
      rolls: 0,             // dice thrown this game (diagnostics)
    };
  }

  /** Is `p` an active seat in this game? (default true for legacy states) */
  function isActive(state, p) {
    return !!(state.players[p] && state.players[p].active !== false);
  }

  /** Active seats only — the canonical rotation order. */
  function activeSeats(state) {
    const out = [];
    for (let p = 0; p < PLAYERS; p++) if (isActive(state, p)) out.push(p);
    return out;
  }

  /** Fair d6. `rand` injectable for deterministic tests. */
  function rollDice(rand) {
    const r = typeof rand === 'function' ? rand() : Math.random();
    return 1 + Math.floor(r * 6) % 6;
  }

  /**
   * Every legal move for `player` after rolling `dice`.
   * Returns [{ token, from, to }]; empty array = no legal move.
   *
   *   • yard token: only a 6 releases it, onto its start (to 0)
   *   • finished token (pos 56): can never move again
   *   • live token: target must not overshoot FINISH (56)
   */
  function legalMoves(state, player, dice) {
    if (state.winner !== null) return [];
    const out = [];
    const P = state.players[player];
    for (let i = 0; i < TOKENS; i++) {
      const pos = P.tokens[i].pos;
      if (pos === FINISH) continue;
      if (pos === -1) {
        if (dice === 6) out.push({ token: i, from: -1, to: 0 });
        continue;
      }
      const to = pos + dice;
      if (to <= FINISH) out.push({ token: i, from: pos, to });
    }
    return out;
  }

  /** Opponent tokens standing on the same GLOBAL square (captures). */
  function victimsAt(state, player, steps) {
    if (steps > LAST_MAIN) return [];           // lanes are private
    const g = globalOf(player, steps);
    if (SAFE_G.has(g)) return [];               // stars protect everyone
    const victims = [];
    for (let p = 0; p < PLAYERS; p++) {
      if (p === player) continue;
      const P = state.players[p];
      for (let i = 0; i < TOKENS; i++) {
        const pos = P.tokens[i].pos;
        if (pos >= 0 && pos <= LAST_MAIN && globalOf(p, pos) === g) {
          victims.push({ player: p, token: i, pos });
        }
      }
    }
    return victims;
  }

  /**
   * Execute a LEGAL move (caller checked legalMoves). Mutates state.
   * Returns the events the UI animates:
   *   { entered, finished, captured:[{player,token}] }
   */
  function applyMove(state, player, token, dice) {
    const P = state.players[player];
    const t = P.tokens[token];
    const ev = { entered: false, finished: false, captured: [] };

    if (t.pos === -1) {
      t.pos = 0;                                  // yard → start square
      ev.entered = true;
    } else {
      t.pos += dice;
    }

    if (t.pos === FINISH) { ev.finished = true; P.home++; }

    for (const v of victimsAt(state, player, t.pos)) {
      state.players[v.player].tokens[v.token].pos = -1;
      ev.captured.push(v);
    }
    /* Capture bonus: a successful capture grants exactly ONE extra roll
       to the capturing player. Set here (the sole rules authority) so
       luEndRoll/grantsExtraRoll can honour it; noteRoll clears it when
       the bonus roll is taken, and noteNoMove overrides it if that
       bonus roll turns out to be unplayable. */
    if (ev.captured.length) state.bonus = true;
    /* Finish bonus: bringing a token to its exact final destination
       (FINISH) grants ONE extra roll to the same player. Same lifecycle
       as the capture bonus: set only on a real completion (applyMove is
       reachable solely through legalMoves, which already rejects any
       overshoot), cleared by noteRoll when the bonus roll is taken, and
       overridden by noteNoMove if the bonus roll is unplayable. A single
       boolean — not a counter — means one finishing move can never stack
       multiple extra turns, and consecutive finishes each re-arm it
       independently because each of those rolls flows through its own
       noteRoll → applyMove cycle. */
    if (ev.finished) state.finishBonus = true;
    return ev;
  }

  /** Does this roll grant another roll for the same player?
   *  • a roll with NO legal move never does — not even a six: an
   *    unplayable six passes the dice straight on (see noteNoMove);
   *  • a capture grants exactly ONE bonus roll (state.bonus), whatever
   *    the dice showed;
   *  • otherwise the classic rule: a six grants another roll, capped
   *    at three sixes in a row. */
  function grantsExtraRoll(state, dice) {
    if (state.noMove) return false;
    if (state.bonus || state.finishBonus) return true;
    return dice === 6 && state.chain < 3;
  }

  /** Register a six in the chain (call after each roll is resolved).
   *  A new roll consumes any pending capture/finish bonus and clears the
   *  no-legal-move mark — all three describe the PREVIOUS roll only. */
  function noteRoll(state, dice) {
    state.rolls++;
    state.chain = dice === 6 ? state.chain + 1 : 0;
    state.bonus = false;
    state.finishBonus = false;
    state.noMove = false;
  }

  /** Mark the CURRENT roll as unplayable (zero legal moves). Such a
   *  roll never grants an extra turn — a six nobody can act on passes
   *  the dice to the next eligible player. Cleared by noteRoll. */
  function noteNoMove(state) {
    state.noMove = true;
    state.bonus = false;
    state.finishBonus = false;
  }

  /** Pass the dice to the next player (resets the six-chain). */
  function advanceTurn(state) {
    state.chain = 0;
    /* Rotate to the NEXT ACTIVE seat. Inactive seats (Phase 4 player-count
       configuration) never receive turns. Falls back to the classic modulo
       rotation when every seat is active — identical behaviour to before. */
    const act = activeSeats(state);
    if (act.length === 0) { state.turn = (state.turn + 1) % PLAYERS; return; }
    const at = act.indexOf(state.turn);
    state.turn = at === -1 ? act[0]
      : act[(at + 1) % act.length];
  }

  /** Active player who brought all tokens home, or null.
   *  Inactive seats can never win (their tokens stay parked in the yard). */
  function checkWinner(state) {
    for (let p = 0; p < PLAYERS; p++) {
      if (!isActive(state, p)) continue;
      if (state.players[p].home === TOKENS) { state.winner = p; return p; }
    }
    return null;
  }

  /**
   * AI strategy — score every legal move, take the best.
   * Same rules engine as humans; this only ever PICKS from the list
   * legalMoves() already validated, so the AI can never cheat.
   *
   *   +1000  finish a token (fewest left to win)
   *   + 500  capture, scaled by how far the victim had travelled
   *   + 300  release a token from the yard (mobility wins games)
   *   + 140  land on a star square, scaled by progress saved
   *   +  …   raw progress: keep the leader running
   *   ±  jitter so equal options don't deadlock deterministically
   */
  function selectAIMove(state, player, moves, rand) {
    if (!moves || !moves.length) return null;
    const r = typeof rand === 'function' ? rand : Math.random;
    let best = null, bestScore = -Infinity;
    for (const m of moves) {
      let s = 0;
      if (m.to === FINISH) s += 1000;
      const victims = victimsAt(state, player, m.to);
      if (victims.length) {
        s += 500 + victims.reduce((a, v) => a + v.pos, 0) * 2;
      }
      if (m.from === -1) s += 300;
      if (m.to <= LAST_MAIN && isSafeStep(player, m.to)) s += 140 + m.to;
      s += m.to;
      s += r() * 8;                                // tie-break jitter
      if (s > bestScore) { bestScore = s; best = m; }
    }
    return best;
  }

/** Difficulty-aware AI move selection (Phase 4.3).
 *
 * `difficulty`:
 *   • 'easy'      — beginner strategy (implemented here)
 *   • 'medium'    — strategic weighted scorer (Phase 4.4)
 *   • 'hard'      — reserved for Phase 4.5 (NOT implemented; same fallback)
 *   • omitted / anything else — legacy scoring AI (unchanged default)
 *
 * Easy strategy — a beginner Ludo player:
 *   1. only ever chooses from the engine-provided LEGAL moves;
 *   2. prefers releasing a token from the yard when the dice allows it
 *      (a release move exists only on a six, per engine rules);
 *   3. otherwise picks uniformly at random among the remaining legal
 *      moves (injectable `rand`, so tests are deterministic);
 *   4. no capture probability math, no defensive/offensive planning.
 */
  function selectAIMoveEasy(moves, rand) {
    if (!moves || !moves.length) return null;
    const r = typeof rand === 'function' ? rand : Math.random;
    const releases = moves.filter(m => m.from === -1);   // yard → track
    if (releases.length) return releases[0];             // rule 2
    const i = Math.floor(r() * moves.length) % moves.length;   // rule 3
    return moves[i];
  }

/** Phase 4.4 — MEDIUM AI.
 *
 * A transparent weighted scoring model over engine-legal moves only.
 * Every component is a real engine fact (positions, stars, victims);
 * no invented probabilities. Weights live in one object so tests can
 * read and reason about them (MEDIUM_WEIGHTS).
 *
 * Priorities (descending typical magnitude):
 *   finish > capture > release/safe-landing > lane entry > progress,
 *   with exposure/danger penalties and a diversity nudge subtracted,
 *   plus controlled jitter so near-equal moves stay non-greedy.
 */
  const MEDIUM_WEIGHTS = {
    finish:      800,   // bring a token home outright
    capture:     220,   // per victim …
    captureStep: 2,     //   … + this per step the victim had travelled
    release:     200,   // yard → track when a six is in hand
    safeLanding: 120,   // land on own star square
    homeStretch: 40,    // enter the private lane (to > LAST_MAIN)
    progress:    3,     // per step advanced toward home (dice distance)
    depth:       1,     // absolute track position — favour the runner
    exposure:   -90,    // LEAVE a safe square for an unsafe landing
    dangerAt:   -60,    // land where an enemy sits 1..6 behind (hit next roll)
    diversity:  -35,    // re-move the token moved last turn (no strategic benefit)
    jitter:      12,    // controlled randomness among near-equal scores
  };

  /* Per-state memo of each player's last moved token (diversity signal).
   * Kept OUTSIDE the game state so engine rules fields are untouched. */
  const mediumLastMoved = new WeakMap();

  /** How many enemy tokens could hit `steps` (for `player`) with their
   *  next roll? Only REAL positions count: an enemy threatens a square
   *  if it sits 1..6 steps BEHIND it on the shared ring. Stars shield. */
  function mediumThreatsAt(state, player, steps) {
    if (steps > LAST_MAIN) return 0;              // lanes are private
    if (isSafeStep(player, steps)) return 0;      // stars protect
    const g = globalOf(player, steps);
    let threats = 0;
    for (let p = 0; p < PLAYERS; p++) {
      if (p === player) continue;
      if (!isActive(state, p)) continue;
      const P = state.players[p];
      for (let i = 0; i < TOKENS; i++) {
        const pos = P.tokens[i].pos;
        if (pos < 0 || pos > LAST_MAIN) continue;
        const back = (g - globalOf(p, pos) + 52) % 52;
        if (back >= 1 && back <= 6) threats++;
      }
    }
    return threats;
  }

  /** Score one legal move for Medium. Pure except for the jitter term. */
  function scoreMoveMedium(state, player, m, W) {
    let s = 0;
    if (m.to === FINISH) s += W.finish;                       // home!
    const victims = victimsAt(state, player, m.to);
    if (victims.length) {
      s += W.capture * victims.length
         + W.captureStep * victims.reduce((a, v) => a + v.pos, 0);
    }
    if (m.from === -1) s += W.release;                        // yard out
    if (m.to <= LAST_MAIN && isSafeStep(player, m.to)) s += W.safeLanding;
    if (m.to > LAST_MAIN && m.to !== FINISH) s += W.homeStretch;   // lane
    s += W.progress * (m.to - Math.max(m.from, 0));           // advance
    s += W.depth * m.to;                                      // keep leaders running
    /* Exposure: stepping OFF a protected square onto open track. */
    if (m.from >= 0 && m.from <= LAST_MAIN && isSafeStep(player, m.from)
        && !(m.to <= LAST_MAIN && isSafeStep(player, m.to))
        && m.to !== FINISH) {
      s += W.exposure;
    }
    /* Danger at the destination: real enemies within one roll behind. */
    s += W.dangerAt * mediumThreatsAt(state, player, m.to);
    /* Diversity: discourage re-moving the same token pointlessly. */
    const memo = mediumLastMoved.get(state);
    if (memo && memo.player === player && memo.token === m.token) {
      s += W.diversity;
    }
    return s;
  }

  /* `record` (default true): store the choice as this player's last move
   * on this state, feeding next turn's diversity term. Pure-selection
   * tests pass false so repeated calls stay idempotent. */
  function selectAIMoveMedium(state, player, moves, rand, record) {
    if (!moves || !moves.length) return null;
    const r = typeof rand === 'function' ? rand : Math.random;
    const W = MEDIUM_WEIGHTS;
    let best = null, bestScore = -Infinity;
    for (const m of moves) {
      const s = scoreMoveMedium(state, player, m, W) + r() * W.jitter;
      if (s > bestScore) { bestScore = s; best = m; }
    }
    if (record !== false) {
      const memo = mediumLastMoved.get(state) || {};
      memo.player = player; memo.token = best.token;
      mediumLastMoved.set(state, memo);
    }
    return best;
  }

/** Phase 4.5 — HARD AI.
 *
 * One-ply evaluator with a strictly bounded look-ahead over opponent
 * replies, built ONLY from engine-legal primitives. The engine has no
 * safe state-cloning primitive, so the look-ahead does not simulate
 * moves on game state at all: it reasons about REPLY THREATS — what an
 * enemy could legally capture next turn given their real positions —
 * which needs no mutation and no invented cloning.
 *
 * Design rules (all verified by scripts/verify_ludo_phase4e.py):
 *   • picks only from legalMoves() output — the AI can never cheat;
 *   • never mutates the live game state (pure scoring + WeakMap memo);
 *   • deterministic under a fixed seed (injectable `rand` for jitter);
 *   • returns null for an empty move list;
 *   • node budget caps total work per decision.
 */
  const HARD_WEIGHTS = {
    finish:      1200,  // bring a token home outright — highest goal
    capture:     420,   // per victim …
    captureStep: 3,     //   … + this per step the victim had travelled
    release:     260,   // yard → track when a six is in hand
    safeLanding: 180,   // land on own star square
    homeStretch: 60,    // enter the private lane (to > LAST_MAIN)
    progress:    4,     // per step advanced toward home (dice distance)
    depth:       2,     // absolute track position — favour the runner
    exposure:   -140,   // LEAVE a safe square for an unsafe landing
    dangerAt:   -110,   // land where an enemy sits 1..6 behind (hit next roll)
    replyThreat: -160,  // enemy could recapture/reply after our move
    diversity:  -45,    // re-move the token moved last turn
    jitter:      10,    // tie-break randomness among near-equal scores
  };

  /* Node budget: caps evaluated (move × threat-scan) units per decision
     so Hard stays O(moves × enemies) worst case and never spins. */
  const HARD_NODE_BUDGET = 400;

  /* Per-state memo of each player's last moved token (diversity signal),
     kept OUTSIDE the game state exactly like mediumLastMoved. */
  const hardLastMoved = new WeakMap();

  /** Would moving `player`'s token from `from` to `to` leave any enemy
   *  with a legal capture next turn? Counts such enemies (0 = quiet).
   *  Pure: reads positions, mutates nothing. */
  function hardReplyThreatsAfter(state, player, m) {
    if (m.to > LAST_MAIN) return 0;             // private lanes are safe
    const gTo = globalOf(player, m.to);
    let threats = 0;
    for (let p = 0; p < PLAYERS; p++) {
      if (p === player) continue;
      if (!isActive(state, p)) continue;
      const P = state.players[p];
      for (let i = 0; i < TOKENS; i++) {
        const pos = P.tokens[i].pos;
        if (pos < 0 || pos > LAST_MAIN) continue;
        /* Could this enemy token reach gTo next turn? On the shared
           ring it must be 1..6 steps BEHIND gTo … */
        const back = (gTo - globalOf(p, pos) + 52) % 52;
        if (back >= 1 && back <= 6) {
          /* … and gTo must not be shielded for us. Our own star cover
             applies to OUR tokens; enemies cannot land on our start
             squares' protection, but stars are shared protection in
             this engine (SAFE_G), so honour it. */
          if (!isSafeStep(player, m.to)) threats++;
        }
      }
    }
    return threats;
  }

  /** Score one legal move for Hard. Pure except for the jitter term. */
  function scoreMoveHard(state, player, m, W) {
    let s = 0;
    if (m.to === FINISH) s += W.finish;                       // home!
    const victims = victimsAt(state, player, m.to);
    if (victims.length) {
      s += W.capture * victims.length
         + W.captureStep * victims.reduce((a, v) => a + v.pos, 0);
    }
    if (m.from === -1) s += W.release;                        // yard out
    if (m.to <= LAST_MAIN && isSafeStep(player, m.to)) s += W.safeLanding;
    if (m.to > LAST_MAIN && m.to !== FINISH) s += W.homeStretch;   // lane
    s += W.progress * (m.to - Math.max(m.from, 0));           // advance
    s += W.depth * m.to;                                      // leaders run
    /* Exposure: stepping OFF a protected square onto open track. */
    if (m.from >= 0 && m.from <= LAST_MAIN && isSafeStep(player, m.from)
        && !(m.to <= LAST_MAIN && isSafeStep(player, m.to))
        && m.to !== FINISH) {
      s += W.exposure;
    }
    /* Danger at the destination: real enemies within one roll behind. */
    s += W.dangerAt * mediumThreatsAt(state, player, m.to);
    /* Look-ahead: would an enemy have a legal capture as a reply? */
    s += W.replyThreat * hardReplyThreatsAfter(state, player, m);
    /* Diversity: discourage re-moving the same token pointlessly. */
    const memo = hardLastMoved.get(state);
    if (memo && memo.player === player && memo.token === m.token) {
      s += W.diversity;
    }
    return s;
  }

  /* `record` (default true): store the choice as this player's last move
     on this state, feeding next turn's diversity term. Pure-selection
     tests pass false so repeated calls stay idempotent. */
  function selectAIMoveHard(state, player, moves, rand, record) {
    if (!moves || !moves.length) return null;
    const r = typeof rand === 'function' ? rand : Math.random;
    const W = HARD_WEIGHTS;
    let best = null, bestScore = -Infinity, nodes = 0;
    for (const m of moves) {
      if (nodes++ >= HARD_NODE_BUDGET) break;   // strict budget cap
      const s = scoreMoveHard(state, player, m, W) + r() * W.jitter;
      if (s > bestScore) { bestScore = s; best = m; }
    }
    if (best && record !== false) {
      const memo = hardLastMoved.get(state) || {};
      memo.player = player; memo.token = best.token;
      hardLastMoved.set(state, memo);
    }
    return best;
  }

  function selectAIMoveForDifficulty(state, player, moves, difficulty, rand) {
    if (difficulty === 'easy') return selectAIMoveEasy(moves, rand);
    if (difficulty === 'medium') return selectAIMoveMedium(state, player, moves, rand);
    if (difficulty === 'hard') return selectAIMoveHard(state, player, moves, rand);
    return selectAIMove(state, player, moves, rand);
  }

  /** Deterministic seeded PRNG (mulberry32) — lets tests fix the AI's
   *  random choices without touching Math.random. */
  function makeRng(seed) {
    let a = (Number(seed) >>> 0) || 1;
    return function () {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  /* ---- exports --------------------------------------------------------- */

  const Engine = {
    PLAYERS, TOKENS, COLORS, NAMES, DEFAULT_SEATS,
    isActive, activeSeats,
    TRACK, START_G, SAFE_G, LANES, YARD_SLOTS,
    LAST_MAIN, FINISH,
    createGame, rollDice, legalMoves, applyMove,
    victimsAt, globalOf, cellOf, isSafeStep,
    grantsExtraRoll, noteRoll, noteNoMove, advanceTurn, checkWinner,
    selectAIMove,
    selectAIMoveEasy, selectAIMoveMedium, selectAIMoveHard,
    selectAIMoveForDifficulty, makeRng,
    MEDIUM_WEIGHTS, scoreMoveMedium,
    HARD_WEIGHTS, scoreMoveHard, hardReplyThreatsAfter, HARD_NODE_BUDGET,
  };

  global.LudoEngine = Engine;
  if (typeof module !== 'undefined' && module.exports) module.exports = Engine;
})(typeof window !== 'undefined' ? window : this);
