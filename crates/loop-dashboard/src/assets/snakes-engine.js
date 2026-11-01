/* ============================================================
   FUN TIME · GAME 10 — SNAKES & LADDERS
   Pure rules engine. DOM-free by design (mirrors ludo-engine.js /
   solitaire-engine.js): this file owns the board geometry, the
   deterministic snake/ladder configuration and the rules. The UI layer
   (snakes-game.js) renders this state and never invents rules.

   Exposes one global: `SnakesEngine` (classic script, matching the
   Fun Time arcade convention).

   ONE RULESET, applied identically to humans and AI:
     • Traditional 10×10 board, 100 numbered squares in zigzag order
       ("boustrophedon"): row 0 (bottom) runs 1→10 left-to-right,
       row 1 runs 11→20 right-to-left, and so on up to square 100 at
       the top-left corner.
     • Every player starts OUTSIDE the board at position 0 and enters
       on their first successful roll (no "roll a six to start" gate).
     • A turn = one fair d6 roll + movement of exactly that many squares.
     • Landing EXACTLY on a ladder bottom climbs instantly to its top.
     • Landing EXACTLY on a snake head slides instantly down to its tail.
     • Square 100 must be reached with an EXACT roll. Overshooting 100
       does not move the token — the turn is simply wasted.
     • No extra roll for any face (no sixes bonus): one roll, one move,
       then the dice pass on. Keeps the ruleset consistent throughout.
     • Only one player can win; the game ends the moment someone lands
       on 100 exactly.
   ============================================================ */
(function (global) {
  'use strict';

  /* ---- board geometry ------------------------------------------------ */

  const SIZE = 10;          // 10×10 grid
  const LAST = 100;         // winning square

  /**
   * [row, col] (0-based, row 0 = bottom) of every square 1..100.
   * Zigzag ordering: odd rows (0-based) run right-to-left.
   * Index 0 is unused so CELL[n] is square n.
   */
  const CELL = (() => {
    const cells = [null];
    for (let n = 1; n <= LAST; n++) {
      const idx = n - 1;
      const row = Math.floor(idx / SIZE);
      let col = idx % SIZE;
      if (row % 2 === 1) col = SIZE - 1 - col;   // odd rows reverse
      cells.push([row, col]);
    }
    return cells;
  })();

  if (CELL.length !== LAST + 1) throw new Error('board must have 100 cells');
  if (CELL[1][0] !== 0 || CELL[1][1] !== 0) throw new Error('square 1 must be bottom-left');
  if (CELL[10][0] !== 0 || CELL[10][1] !== 9) throw new Error('square 10 must be bottom-right');
  if (CELL[11][0] !== 1 || CELL[11][1] !== 9) throw new Error('square 11 must be row 1 right');
  if (CELL[100][0] !== 9 || CELL[100][1] !== 0) throw new Error('square 100 must be top-left');

  /* ---- deterministic snakes & ladders -------------------------------- */

  /** Ladders as [bottom, top]; climbing is mandatory. */
  const LADDERS = [
    [4, 14],
    [9, 31],
    [21, 42],
    [28, 84],
    [36, 44],
    [51, 67],
    [71, 91],
  ];

  /** Snakes as [head, tail]; sliding down is mandatory. */
  const SNAKES = [
    [47, 26],
    [49, 11],
    [56, 53],
    [62, 19],
    [87, 24],
    [93, 73],
    [98, 78],
  ];

  /** square -> destination square (both directions resolved eagerly). */
  const JUMPS = (() => {
    const m = new Map();
    for (const [b, t] of LADDERS) m.set(b, t);
    for (const [h, t] of SNAKES) m.set(h, t);
    return m;
  })();

  /** Fast membership sets for the renderer / tests. */
  const LADDER_BOTTOMS = new Set(LADDERS.map(x => x[0]));
  const LADDER_TOPS = new Set(LADDERS.map(x => x[1]));
  const SNAKE_HEADS = new Set(SNAKES.map(x => x[0]));
  const SNAKE_TAILS = new Set(SNAKES.map(x => x[1]));

  /* ---- players -------------------------------------------------------- */

  const MAX_PLAYERS = 4;
  const COLORS = ['red', 'green', 'yellow', 'blue'];
  const NAMES = ['Red', 'Green', 'Yellow', 'Blue'];

  /* ---- pure helpers --------------------------------------------------- */

  /** Fair d6. `rand` injectable for deterministic tests. */
  function rollDice(rand) {
    const r = typeof rand === 'function' ? rand() : Math.random();
    return 1 + Math.floor(r * 6) % 6;
  }

  /**
   * Destination after landing on `sq`, following at most one jump.
   * (The shipped board never chains jumps, but a defensive loop keeps
   * the engine honest if the config ever changes.)
   */
  function resolve(sq) {
    let cur = sq, hops = 0;
    while (JUMPS.has(cur) && hops < 8) { cur = JUMPS.get(cur); hops++; }
    return { square: cur, hops };
  }

  /**
   * Everything that happens when `player` rolls `dice`, computed WITHOUT
   * mutating state — the UI uses this to drive its animations.
   *
   * Returns null when the roll is illegal for the current phase.
   * Shape:
   *   {
   *     legal: true,
   *     overshoot: false,        // true when pos+dice > 100 (no move)
   *     from: 12,                // starting square (0 = off board)
   *     to: 16,                  // raw landing square (== from when overshoot)
   *     final: 16,               // square after snakes/ladders
   *     kind: 'move'|'ladder'|'snake'|'win'|'stuck',
   *     climb: [16, 42] | null,  // ladder [from,to] when climbing
   *     slide: [87, 24] | null,  // snake [from,to] when sliding
   *     won: false,
   *   }
   */
  function previewMove(state, player, dice) {
    if (!state || state.winner !== null) return null;
    if (state.turn !== player) return null;
    if (dice < 1 || dice > 6 || Math.floor(dice) !== dice) return null;

    const P = state.players[player];
    const from = P.pos;
    const to = from + dice;

    /* Overshooting 100 wastes the turn: the token does not move. */
    if (to > LAST) {
      return {
        legal: true, overshoot: true, from, to: from, final: from,
        kind: 'stuck', climb: null, slide: null, won: false,
      };
    }

    const r = resolve(to);
    const climbed = r.square > to;
    const slid = r.square < to;

    return {
      legal: true,
      overshoot: false,
      from, to,
      final: r.square,
      kind: r.square === LAST ? 'win'
        : climbed ? 'ladder'
        : slid ? 'snake'
        : 'move',
      climb: climbed ? [to, r.square] : null,
      slide: slid ? [to, r.square] : null,
      won: r.square === LAST,
    };
  }

  /* ---- state ----------------------------------------------------------- */

  /**
   * Fresh game. `kinds` is an array of 'human' | 'ai' (2..4 entries);
   * defaults to one human versus one AI.
   */
  function createGame(kinds) {
    const k = Array.isArray(kinds) && kinds.length >= 2 && kinds.length <= MAX_PLAYERS
      ? kinds.slice()
      : ['human', 'ai'];
    return {
      players: k.map(kind => ({ kind, pos: 0 })),   // 0 = outside the board
      turn: 0,
      winner: null,
      rolls: 0,            // dice thrown this game (diagnostics)
      history: [],         // per-roll record for the UI's event log
    };
  }

  /**
   * Apply a roll for the CURRENT player (caller checks state.turn).
   * Mutates state; returns the same shape as previewMove so the UI can
   * animate either path identically. Returns null for an illegal roll.
   */
  function applyRoll(state, dice) {
    const mv = previewMove(state, state.turn, dice);
    if (!mv) return null;

    const p = state.turn;
    state.rolls++;
    state.players[p].pos = mv.final;

    state.history.push({
      player: p, dice, from: mv.from, to: mv.to,
      final: mv.final, kind: mv.kind,
    });

    if (mv.won) state.winner = p;
    else advanceTurn(state);
    return mv;
  }

  /** Pass the dice to the next player still in the game. */
  function advanceTurn(state) {
    state.turn = (state.turn + 1) % state.players.length;
  }

  /**
   * AI "strategy". Snakes & Ladders has zero decisions — the only thing
   * an opponent can control is rolling. This exists so the UI has one
   * place to ask "whose turn is automated?", and so tests can assert the
   * AI never claims an illegal outcome.
   */
  function aiShouldRoll(state) {
    return state && state.winner === null
      && state.players[state.turn].kind === 'ai';
  }

  /* ---- exports --------------------------------------------------------- */

  const Engine = {
    SIZE, LAST, CELL,
    LADDERS, SNAKES, JUMPS,
    LADDER_BOTTOMS, LADDER_TOPS, SNAKE_HEADS, SNAKE_TAILS,
    MAX_PLAYERS, COLORS, NAMES,
    createGame, rollDice, previewMove, applyRoll,
    advanceTurn, resolve, aiShouldRoll,
  };

  global.SnakesEngine = Engine;
  if (typeof module !== 'undefined' && module.exports) module.exports = Engine;
})(typeof window !== 'undefined' ? window : this);
