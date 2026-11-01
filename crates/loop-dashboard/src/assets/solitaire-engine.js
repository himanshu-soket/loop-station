/* ============================================================
   FUN TIME · GAME 7 — SOLITAIRE (CLASSIC KLONDIKE, DRAW 1)
   Pure rules engine. DOM-free by design: this file owns nothing
   but the deck, the piles and the rules. The UI layer
   (solitaire-game.js) renders this state and never invents rules.

   Exposes one global: `SolitaireEngine` (classic script, matches the
   chess.js / timberman-game.js convention of the Fun Time arcade).
   ============================================================ */
(function (global) {
  'use strict';

  const SUITS = ['S', 'H', 'D', 'C'];            // spades hearts diamonds clubs
  const RANKS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]; // A..K
  const RED = new Set(['H', 'D']);

  /* ---- pure helpers -------------------------------------------------- */

  function isRed(suit) { return RED.has(suit); }

  /** A card is a stable record: `suit`/`rank`/`id` never change, while
   *  `faceUp` is the one mutable flag the rules flip. The id ("AS", "10H",
   *  "KD"…) is what the UI binds its DOM nodes to. */
  function makeCard(suit, rank) {
    return {
      id: (rank === 10 ? '10' : RANK_CHARS[rank]) + suit,
      suit, rank,
      red: isRed(suit),
      faceUp: false,
    };
  }
  const RANK_CHARS = { 1: 'A', 2: '2', 3: '3', 4: '4', 5: '5', 6: '6', 7: '7',
    8: '8', 9: '9', 10: '10', 11: 'J', 12: 'Q', 13: 'K' };

  /** Fresh ordered 52-card deck. */
  function createDeck() {
    const d = [];
    for (const s of SUITS) for (const r of RANKS) d.push(makeCard(s, r));
    return d;
  }

  /** Fisher-Yates on a copy; the input array is never mutated. */
  function shuffle(deck, rand) {
    const r = rand || Math.random;
    const a = deck.slice();
    for (let i = a.length - 1; i > 0; i--) {
      const j = Math.floor(r() * (i + 1));
      const t = a[i]; a[i] = a[j]; a[j] = t;
    }
    return a;
  }

  /* ---- scoring table (standard Klondike draw-one) -------------------- */
  const SCORE = {
    tableauToFoundation: 10,
    wasteToTableau: 5,
    wasteToFoundation: 10,
    foundationToTableau: -15,
    reveal: 5,
    recycleWaste: -20,     // applied only when configured (default OFF)
  };

  /* ---- the game ------------------------------------------------------ */

  function createGame(opts) {
    const options = Object.assign({ drawCount: 1, recyclePenalty: false }, opts || {});
    const g = {
      options,
      stock: [],            // face-down, top = last element
      waste: [],            // face-up,   top = last element
      foundations: [[], [], [], []],
      tableau: [[], [], [], [], [], [], []],
      score: 0,
      moves: 0,
      recycles: 0,
      status: 'ready',      // ready | playing | won
      history: [],
      _seq: 0,
    };
    deal(g, options.rand);
    return g;
  }

  /** Deal from a fresh shuffled deck: 28 cards to the tableau (only the
   *  last card of each column face up), the remaining 24 to the stock. */
  function deal(g, rand) {
    const deck = shuffle(createDeck(), rand);
    g.stock = []; g.waste = [];
    g.foundations = [[], [], [], []];
    g.tableau = [[], [], [], [], [], [], []];
    let k = 0;
    for (let col = 0; col < 7; col++) {
      for (let row = 0; row <= col; row++) {
        const c = deck[k++];
        c.faceUp = row === col;
        g.tableau[col].push(c);
      }
    }
    g.stock = deck.slice(k);          // 24 cards, all face down
    g.score = 0; g.moves = 0; g.recycles = 0;
    g.status = 'ready';
    g.history = [];
  }

  /* ---- predicates ---------------------------------------------------- */

  /** Can `card` (face up) start a foundation pile? Only an Ace. */
  function canStartFoundation(card) {
    return !!card && card.faceUp && card.rank === 1;
  }

  /** Can `card` be placed on foundation `f`? */
  function canMoveToFoundation(foundation, card) {
    if (!card || !card.faceUp) return false;
    if (foundation.length === 0) return canStartFoundation(card);
    const top = foundation[foundation.length - 1];
    return top.suit === card.suit && card.rank === top.rank + 1;
  }

  /** Can `card` (or a King-led sequence starting at it) land on tableau
   *  column `col`? Empty columns take Kings only. */
  function canMoveToTableau(col, card) {
    if (!card || !card.faceUp) return false;
    if (col.length === 0) return card.rank === 13;
    const top = col[col.length - 1];
    return top.faceUp && top.red !== card.red && top.rank === card.rank + 1;
  }

  /** Waste is never a drop target in Klondike. */
  function canMoveToWaste() { return false; }

  /** Is the face-up run starting at index `i` in `col` a valid movable
   *  sequence (descending, alternating colours)? */
  function isValidRun(col, i) {
    if (i < 0 || i >= col.length) return false;
    for (let k = i; k < col.length; k++) {
      const c = col[k];
      if (!c.faceUp) return false;
      if (k > i) {
        const prev = col[k - 1];
        if (prev.red === c.red || prev.rank !== c.rank + 1) return false;
      }
    }
    return true;
  }

  /** Deepest legal grab index in a column (the longest movable run). */
  function movableIndex(col) {
    for (let i = 0; i < col.length; i++) {
      if (isValidRun(col, i)) return i;
    }
    return -1;
  }

  /* ---- moves --------------------------------------------------------- */

  /** Deep-enough snapshot for undo: piles, flags, counters, status.
   *  Card objects are shared (frozen ids) but `faceUp` is restored by
   *  copying the flag per card, so a flip survives an undo. */
  function snapshot(g) {
    const faceFlags = new Map();
    const walk = c => { faceFlags.set(c.id, c.faceUp); };
    g.stock.forEach(walk); g.waste.forEach(walk);
    g.foundations.forEach(f => f.forEach(walk));
    g.tableau.forEach(t => t.forEach(walk));
    return {
      stock: g.stock.map(c => c.id),
      waste: g.waste.map(c => c.id),
      foundations: g.foundations.map(f => f.map(c => c.id)),
      tableau: g.tableau.map(t => t.map(c => c.id)),
      faceFlags: [...faceFlags.entries()],
      score: g.score, moves: g.moves, recycles: g.recycles,
      status: g.status,
    };
  }

  /** Rebuild live piles from a snapshot, reusing the same card objects
   *  (stable identity keeps DOM nodes bound across undo). */
  function restore(g, snap) {
    const byId = new Map();
    for (const pile of [g.stock, g.waste, ...g.foundations, ...g.tableau]) {
      for (const c of pile) byId.set(c.id, c);
    }
    const pick = id => {
      const c = byId.get(id);
      if (!c) throw new Error('undo: unknown card ' + id);
      return c;
    };
    g.stock = snap.stock.map(pick);
    g.waste = snap.waste.map(pick);
    g.foundations = snap.foundations.map(f => f.map(pick));
    g.tableau = snap.tableau.map(t => t.map(pick));
    for (const [id, up] of snap.faceFlags) {
      const c = byId.get(id);
      if (c) c.faceUp = up;
    }
    g.score = snap.score; g.moves = snap.moves;
    g.recycles = snap.recycles; g.status = snap.status;
  }

  function pushHistory(g) {
    g.history.push(snapshot(g));
    if (g.history.length > 400) g.history.shift();   // bounded memory
  }

  function touch(g) {
    if (g.status === 'ready') g.status = 'playing';
  }

  /** Move `count` cards from the end of tableau column `from` to `to`.
   *  Returns true when the move happened. */
  function moveTableauToTableau(g, from, to, count) {
    const src = g.tableau[from];
    if (!src || count < 1 || count > src.length) return false;
    const i = src.length - count;
    if (!isValidRun(src, i)) return false;
    const moving = src.slice(i);
    if (!canMoveToTableau(g.tableau[to], moving[0])) return false;
    pushHistory(g);
    src.splice(i);
    g.tableau[to].push(...moving);
    revealTop(g, src);
    g.moves++; touch(g);
    return true;
  }

  /** Waste top → tableau column or foundation. */
  function moveWasteTo(g, dest) {
    if (g.waste.length === 0) return false;
    const card = g.waste[g.waste.length - 1];
    if (dest.kind === 'tableau') {
      if (!canMoveToTableau(g.tableau[dest.index], card)) return false;
      pushHistory(g);
      g.waste.pop();
      g.tableau[dest.index].push(card);
      g.score += SCORE.wasteToTableau;
    } else if (dest.kind === 'foundation') {
      if (!canMoveToFoundation(g.foundations[dest.index], card)) return false;
      pushHistory(g);
      g.waste.pop();
      g.foundations[dest.index].push(card);
      g.score += SCORE.wasteToFoundation;
    } else return false;
    g.moves++; touch(g);
    return true;
  }

  /** Tableau top card → foundation. */
  function moveTableauToFoundation(g, colIdx, fIdx) {
    const col = g.tableau[colIdx];
    if (!col || col.length === 0) return false;
    const card = col[col.length - 1];
    if (!canMoveToFoundation(g.foundations[fIdx], card)) return false;
    pushHistory(g);
    col.pop();
    g.foundations[fIdx].push(card);
    g.score += SCORE.tableauToFoundation;
    revealTop(g, col);
    g.moves++; touch(g);
    checkWin(g);
    return true;
  }

  /** Foundation top → tableau column (scored as a pull-back). */
  function moveFoundationToTableau(g, fIdx, colIdx) {
    const f = g.foundations[fIdx];
    if (!f || f.length === 0) return false;
    const card = f[f.length - 1];
    if (!canMoveToTableau(g.tableau[colIdx], card)) return false;
    pushHistory(g);
    f.pop();
    g.tableau[colIdx].push(card);
    g.score += SCORE.foundationToTableau;
    g.moves++; touch(g);
    return true;
  }

  /** Reveal the newly exposed top card of a tableau column (+5). */
  function revealTop(g, col) {
    if (col.length === 0) return false;
    const top = col[col.length - 1];
    if (top.faceUp) return false;
    top.faceUp = true;
    g.score += SCORE.reveal;
    return true;
  }

  /** Draw one card from the stock to the waste (draw-1). */
  function drawFromStock(g) {
    if (g.status === 'won') return false;
    if (g.stock.length === 0) return false;
    pushHistory(g);
    const card = g.stock.pop();
    card.faceUp = true;
    g.waste.push(card);
    g.moves++; touch(g);
    return true;
  }

  /** Recycle the waste back into the stock (standard Klondike order). */
  function recycleWaste(g) {
    if (g.stock.length !== 0 || g.waste.length === 0) return false;
    pushHistory(g);
    const n = g.waste.length;
    while (g.waste.length) {
      const c = g.waste.pop();
      c.faceUp = false;
      g.stock.push(c);
    }
    g.recycles++;
    if (g.options.recyclePenalty) g.score += SCORE.recycleWaste;
    g.moves++; touch(g);
    return { recycled: n };
  }

  /** Undo the last player action. Restores piles, faces, score, moves. */
  function undo(g) {
    if (g.history.length === 0) return false;
    const snap = g.history.pop();
    restore(g, snap);
    return true;
  }

  /** All 52 cards on foundations? */
  function checkWin(g) {
    const total = g.foundations.reduce((n, f) => n + f.length, 0);
    if (total === 52) { g.status = 'won'; return true; }
    return false;
  }

  /* ---- hints --------------------------------------------------------- */

  /** Enumerate every legal single move from the current position.
   *  Each entry: {kind, from:{...}, to:{...}, card, count?, why}. */
  function legalMoves(g) {
    const out = [];

    // Tableau → foundation.
    for (let c = 0; c < 7; c++) {
      const col = g.tableau[c];
      if (!col.length) continue;
      const card = col[col.length - 1];
      for (let f = 0; f < 4; f++) {
        if (canMoveToFoundation(g.foundations[f], card)) {
          out.push({ kind: 'tf', from: { pile: 'tableau', index: c },
            to: { pile: 'foundation', index: f }, card, count: 1,
            why: `${card.id} to foundation` });
        }
      }
    }

    // Waste → foundation, then waste → tableau.
    if (g.waste.length) {
      const card = g.waste[g.waste.length - 1];
      for (let f = 0; f < 4; f++) {
        if (canMoveToFoundation(g.foundations[f], card)) {
          out.push({ kind: 'wf', from: { pile: 'waste' },
            to: { pile: 'foundation', index: f }, card, count: 1,
            why: `${card.id} to foundation` });
        }
      }
      for (let c = 0; c < 7; c++) {
        if (canMoveToTableau(g.tableau[c], card)) {
          out.push({ kind: 'wt', from: { pile: 'waste' },
            to: { pile: 'tableau', index: c }, card, count: 1,
            why: `${card.id} onto ${g.tableau[c].length
              ? g.tableau[c][g.tableau[c].length - 1].id : 'empty column'}` });
        }
      }
    }

    // Tableau runs → other tableau columns.
    for (let c = 0; c < 7; c++) {
      const col = g.tableau[c];
      const first = firstFaceUp(col);
      if (first < 0) continue;
      for (let i = first; i < col.length; i++) {
        if (!isValidRun(col, i)) continue;
        const card = col[i];
        const count = col.length - i;
        for (let d = 0; d < 7; d++) {
          if (d === c) continue;
          if (!canMoveToTableau(g.tableau[d], card)) continue;
          // Skip no-op shuffles: a full run whose parent is already
          // face-up-and-compatible would just re-stack identically.
          if (count === col.length && g.tableau[d].length === 0
              && first === 0) continue;
          out.push({ kind: 'tt', from: { pile: 'tableau', index: c },
            to: { pile: 'tableau', index: d }, card, count,
            why: `${card.id}${count > 1 ? '+' + (count - 1) : ''} → column ${d + 1}` });
        }
      }
    }

    // Foundation → tableau (a real option, listed last).
    for (let f = 0; f < 4; f++) {
      const fd = g.foundations[f];
      if (!fd.length) continue;
      const card = fd[fd.length - 1];
      for (let c = 0; c < 7; c++) {
        if (canMoveToTableau(g.tableau[c], card)) {
          out.push({ kind: 'ft', from: { pile: 'foundation', index: f },
            to: { pile: 'tableau', index: c }, card, count: 1,
            why: `${card.id} back to column ${c + 1}` });
        }
      }
    }

    return out;
  }

  function firstFaceUp(col) {
    for (let i = 0; i < col.length; i++) if (col[i].faceUp) return i;
    return -1;
  }

  /** One useful hint, rule-aware, in priority order.
   *  Returns {text, move?} — move is null when only drawing remains. */
  function getHint(g) {
    const moves = legalMoves(g);

    // 1. Any tableau→foundation that uncovers a face-down card.
    const reveals = moves.filter(m => m.kind === 'tf'
      && g.tableau[m.from.index].length > 1
      && !g.tableau[m.from.index][g.tableau[m.from.index].length - 2].faceUp);
    if (reveals.length) return { text: `Play ${reveals[0].why}`, move: reveals[0] };

    // 2. Any foundation move at all.
    const anyF = moves.find(m => m.to.pile === 'foundation');
    if (anyF) return { text: `Play ${anyF.why}`, move: anyF };

    // 3. A tableau move that frees a face-down card or empties a column.
    const freeing = moves.filter(m => m.kind === 'tt'
      && (m.from.index !== undefined)
      && (g.tableau[m.from.index].length - m.count > 0
        && !g.tableau[m.from.index][g.tableau[m.from.index].length - m.count - 1].faceUp));
    if (freeing.length) return { text: `Move ${freeing[0].why}`, move: freeing[0] };

    // 4. A King-led run to an empty column.
    const king = moves.find(m => m.kind === 'tt' && m.card.rank === 13
      && g.tableau[m.to.index].length === 0);
    if (king) return { text: `Move ${king.why}`, move: king };

    // 5. Any other tableau consolidation that is not a pure no-op.
    const usefulTt = moves.find(m => m.kind === 'tt' && m.count < g.tableau[m.from.index].length);
    if (usefulTt) return { text: `Move ${usefulTt.why}`, move: usefulTt };

    // 6. Waste plays.
    const w = moves.find(m => m.kind === 'wt');
    if (w) return { text: `Play ${w.why}`, move: w };

    // 7. Nothing left but the stock.
    if (g.stock.length > 0) return { text: 'Draw from the stock', move: null };
    if (g.waste.length > 0) return { text: 'Recycle the waste pile', move: null };
    return { text: 'No moves available — try undo or start a new game', move: null };
  }

  /* ---- public API ---------------------------------------------------- */

  const Engine = {
    SUITS, RANKS, RANK_CHARS, SCORE,
    isRed, createDeck, shuffle, createGame, deal,
    canStartFoundation, canMoveToFoundation, canMoveToTableau, canMoveToWaste,
    isValidRun, movableIndex, firstFaceUp,
    drawFromStock, recycleWaste,
    moveTableauToTableau, moveWasteTo, moveTableauToFoundation,
    moveFoundationToTableau, revealTop,
    undo, checkWin, legalMoves, getHint, snapshot, restore,
  };

  if (typeof module !== 'undefined' && module.exports) module.exports = Engine;
  global.SolitaireEngine = Engine;
})(typeof window !== 'undefined' ? window : globalThis);
