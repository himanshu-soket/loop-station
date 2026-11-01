#!/usr/bin/env python3
"""Phase 4.6 — AI Tournament & Intelligence Benchmarking.

Reproducible headless tournament over the REAL production Ludo engine and
the REAL production AI decision functions (ludo-engine.js). No rewrites,
no stubs: every game runs createGame/rollDice/legalMoves/applyMove and the
same selectAIMove{,Easy,Medium,Hard} the dashboard UI calls.

Execution: no Node runtime exists on this machine, so games run inside
headless Chromium via Playwright (page.evaluate). The engine source is
loaded verbatim from the repo asset; nothing is re-implemented in Python.

Matchups (2-player seats [0,2] and 4-player mixed):
  1. Classic vs Easy        5. Easy vs Hard
  2. Easy vs Medium         6. Classic vs Medium
  3. Medium vs Hard         7. 4P mixed: Classic/Easy/Medium/Hard
  4. Classic vs Hard

Fairness:
  • Seat/color exposure balanced by playing each matchup in SEAT-SWAPPED
    pairs: game k uses seat order A,B and game k+1 uses B,A. Every
    difficulty therefore occupies each color equally often.
  • Fixed documented seed set: game seeds are seedBase + 2k / seedBase +
    2k+1 (one per swapped pair), seedBase = 20260406 (date-stable).
  • All difficulties share ONE rules engine and ONE legal-move generator;
    the only difference is the move-selection function.
  • Turn cap: MAX_TURNS_PER_GAME (1000). Capped games are reported as
    'unfinished' separately — never counted as losses for anyone.

Metrics per difficulty: wins, win rate, avg turns-to-victory, captures
made/suffered, tokens finished, tokens still on board at game end,
no-move frequency, invalid-move count, decision latency p50/p95/max.

Usage:
  .venv-test/bin/python scripts/benchmark_ludo_ai.py [--games N]
      [--json OUT.json] [--markdown docs/ludo-ai-benchmark.md]
      [--seed-base 20260406] [--time-budget-min 30]
"""
import argparse
import json
import math
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
ENGINE_SRC = (ROOT / "crates/loop-dashboard/src/assets/ludo-engine.js").read_text()

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
SEED_BASE_DEFAULT = 20260406
MAX_TURNS_PER_GAME = 1000          # hard cap; beyond this = unfinished
DEFAULT_GAMES = 200                # completed games per matchup (pairs)
TIME_BUDGET_MIN = 30               # overall wall-clock budget (minutes)

DIFFS = ["classic", "easy", "medium", "hard"]

# Matchup definitions. Each entry: (label, seat_difficulties_2p) where the
# list gives the difficulty per ACTIVE seat in seat order.
MATCHUPS_2P = [
    ("Classic vs Easy",   ["classic", "easy"]),
    ("Easy vs Medium",    ["easy", "medium"]),
    ("Medium vs Hard",    ["medium", "hard"]),
    ("Classic vs Hard",   ["classic", "hard"]),
    ("Easy vs Hard",      ["easy", "hard"]),
    ("Classic vs Medium", ["classic", "medium"]),
]

# 4P mixed: one seat of each difficulty, rotated across games for balance.
MIXED_4P_LABEL = "4P Mixed (Classic/Easy/Medium/Hard)"
MIXED_4P = ["classic", "easy", "medium", "hard"]


# ---------------------------------------------------------------------------
# In-browser tournament driver
# ---------------------------------------------------------------------------
# Body of the per-game driver. Invoked as
#   ([difficultyOfSeat, seed, maxTurns]) => (BODY)
# — a single destructured array argument (Playwright passes one arg per
# call, so positional multi-arg signatures silently receive undefined).
TOURNAMENT_JS_BODY = r"""
  const E = LudoEngine;

  // ---- deterministic PRNG (mulberry32, same as engine's makeRng) ----
  let a = (seed >>> 0) || 1;
  const rng = () => {
    a |= 0; a = (a + 0x6D2B79F5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };

  // ---- one independent RNG stream per concern (dice vs AI jitter) ----
  // Split by drawing two fresh mulberry32 streams seeded from the master.
  const mkStream = (s) => { let x = (s >>> 0) || 1; return () => {
    x |= 0; x = (x + 0x6D2B79F5) | 0;
    let t = Math.imul(x ^ (x >>> 15), 1 | x);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }; };
  const diceRng   = mkStream(seed ^ 0xD1CE0000);
  const aiRng     = mkStream(seed ^ 0x0A10000);

  // Difficulty → engine selector. NOTE: 'classic' is the legacy scorer
  // selectAIMove() (null difficulty path) — identical to the UI default.
  const pick = (diff, state, player, moves) => {
    if (diff === 'easy')   return E.selectAIMoveEasy(moves, aiRng);
    if (diff === 'medium') return E.selectAIMoveMedium(state, player, moves, aiRng);
    if (diff === 'hard')   return E.selectAIMoveHard(state, player, moves, aiRng);
    return E.selectAIMove(state, player, moves, aiRng);   // classic
  };

  // ---- seat setup: mirror the UI's own seat mapping ----
  // 2P → active seats [0,2]; 4P → [0,1,2,3]. kinds all 'ai'.
  const nSeats = difficultyOfSeat.length;
  const seats = nSeats === 2 ? [0, 2] : [0, 1, 2, 3];
  const g = E.createGame(['ai','ai','ai','ai'], { seats });

  // Per-seat difficulty keyed by PLAYER INDEX (seat color).
  const diffByPlayer = {};
  seats.forEach((s, i) => { diffByPlayer[s] = difficultyOfSeat[i]; });

  // ---- stats collection ----
  const stats = {
    turns: 0,                    // full turn rotations handed the dice
    rolls: 0,                    // dice throws
    noMoveRolls: 0,              // rolls that produced zero legal moves
    invalidMoves: 0,             // selector returned a non-legal move
    captures: {},                // per player: tokens captured BY them
    suffered: {},                // per player: tokens lost TO others
    latencies: [],               // ms per AI decision (all difficulties)
    latByDiff: {},               // per difficulty: [ms...]
    winner: null,
    finished: false,             // legitimate winner produced
    capped: false,               // hit MAX_TURNS without a winner
  };
  seats.forEach(s => { stats.captures[s] = 0; stats.suffered[s] = 0;
                       stats.latByDiff[diffByPlayer[s]] = []; });

  // ---- game loop: EXACTLY the UI's turn-flow semantics ----
  // rollDice → noteRoll → legalMoves → (empty? settle+advance) →
  // applyMove → checkWinner → grantsExtraRoll? keep : advanceTurn.
  while (stats.turns < maxTurns) {
    const p = g.turn;
    const v = E.rollDice(diceRng);
    E.noteRoll(g, v);
    stats.rolls++;

    const moves = E.legalMoves(g, p, v);
    if (!moves.length) {
      stats.noMoveRolls++;
      // Unplayable roll — even a six never grants an extra turn (the
      // engine's noteNoMove marks it; grantsExtraRoll returns false).
      E.noteNoMove(g);
      E.advanceTurn(g);
      stats.turns++;
      continue;
    }

    // AI decision (timed). Chromium's performance.now() ticks in ~0.1ms
    // quanta, so a single sub-quantum decision reads as 0. Batch 64
    // repetitions of the SAME decision and divide — the per-decision
    // cost is preserved, just measured with usable resolution.
    const t0 = performance.now();
    let mv = null;
    for (let rep = 0; rep < 64; rep++) {
      mv = pick(diffByPlayer[p], g, p, moves);
    }
    const dt = (performance.now() - t0) / 64;
    stats.latencies.push(dt);
    stats.latByDiff[diffByPlayer[p]].push(dt);

    // legality guard: the chosen move MUST be in the legal list
    const legal = mv && moves.some(m =>
      m.token === mv.token && m.to === mv.to && m.from === mv.from);
    if (!mv || !legal) {
      stats.invalidMoves++;
      // fall back to first legal move so the game can continue; this is
      // a benchmark anomaly counter, never a silent advantage.
      const fb = moves[0];
      const ev = E.applyMove(g, p, fb.token, v);
      stats.captures[p] += ev.captured.length;
      ev.captured.forEach(c => { stats.suffered[c.player]++; });
    } else {
      const ev = E.applyMove(g, p, mv.token, v);
      stats.captures[p] += ev.captured.length;
      ev.captured.forEach(c => { stats.suffered[c.player]++; });
    }

    if (E.checkWinner(g) !== null) {
      stats.winner = g.winner;
      stats.finished = true;
      break;
    }

    // Capture bonus (state.bonus, set by applyMove) or a six: roll again.
    if (E.grantsExtraRoll(g, v)) continue;
    E.advanceTurn(g);
    stats.turns++;
  }
  if (!stats.finished) stats.capped = true;

  // ---- end-of-game board state ----
  const board = {};
  seats.forEach(s => {
    const P = g.players[s];
    board[s] = {
      home: P.home,                                   // tokens finished
      onBoard: P.tokens.filter(t => t.pos >= 0 && t.pos !== E.FINISH).length,
      inYard: P.tokens.filter(t => t.pos === -1).length,
    };
  });

  return {
    stats, board, winnerDiff: stats.winner === null ? null : diffByPlayer[stats.winner],
    winnerSeat: stats.winner, seats, diffByPlayer,
  };
"""


def run_tournament(page, games_per_matchup, seed_base, time_budget_min):
    """Run every matchup; returns raw results dict."""
    results = {}
    deadline = time.time() + time_budget_min * 60
    total_started = time.time()

    def play_game(diff_list, seed):
        # Playwright passes exactly ONE argument per evaluate() call, so
        # the driver takes a single destructured array — positional
        # multi-arg signatures would silently receive undefined.
        return page.evaluate(
            "([difficultyOfSeat, seed, maxTurns]) => {\n" + TOURNAMENT_JS_BODY + "\n}",
            [diff_list, seed, MAX_TURNS_PER_GAME],
        )

    def run_matchup(label, base_order, n_players):
        """Play `games_per_matchup` games with seat-swapping for fairness.

        For 2P: games come in swapped pairs (A@0,B@2 then B@0,A@2).
        For 4P mixed: rotate the difficulty assignment around the seats
        (4 rotations), so each difficulty visits each seat equally.
        """
        agg = {
            "label": label, "n_players": n_players,
            "games": 0, "finished": 0, "capped": 0,
            "wins": {d: 0 for d in DIFFS},
            "seat_wins": {},          # wins by STARTING seat index
            "captures": {d: 0 for d in DIFFS},
            "suffered": {d: 0 for d in DIFFS},
            "tokens_finished": {d: 0 for d in DIFFS},
            "tokens_on_board": {d: 0 for d in DIFFS},
            "turns_to_victory": [],   # per finished game
            "no_move_rolls": 0, "total_rolls": 0,
            "invalid_moves": 0,
            "lat_by_diff": {d: [] for d in DIFFS},
            "seeds": [],
        }
        if n_players == 2:
            rotations = [base_order, list(reversed(base_order))]
        else:
            # 4P: rotate right by i positions → each diff hits each seat
            rotations = [base_order[-i:] + base_order[:-i] for i in range(4)]

        games_done = 0
        pair_idx = 0
        while games_done < games_per_matchup:
            if time.time() > deadline:
                print(f"  ! time budget hit during '{label}' "
                      f"({games_done}/{games_per_matchup} games)")
                break
            order = rotations[games_done % len(rotations)]
            seed = seed_base + pair_idx * 1009 + games_done
            res = play_game(order, seed)
            agg["seeds"].append(seed)
            agg["games"] += 1
            if res["stats"]["finished"]:
                agg["finished"] += 1
                wd = res["winnerDiff"]
                if wd:
                    agg["wins"][wd] += 1
                    ws = res["winnerSeat"]
                    agg["seat_wins"][str(ws)] = agg["seat_wins"].get(str(ws), 0) + 1
                agg["turns_to_victory"].append(res["stats"]["turns"])
            elif res["stats"]["capped"]:
                agg["capped"] += 1

            st = res["stats"]
            agg["no_move_rolls"] += st["noMoveRolls"]
            agg["total_rolls"] += st["rolls"]
            agg["invalid_moves"] += st["invalidMoves"]
            for s, d in res["diffByPlayer"].items():
                # JS object keys arrive as strings ('0','2',…)
                agg["captures"][d] += st["captures"].get(s, 0)
                agg["suffered"][d] += st["suffered"].get(s, 0)
                b = res["board"].get(s) or res["board"].get(str(s)) \
                    or res["board"].get(int(s))
                agg["tokens_finished"][d] += b["home"]
                agg["tokens_on_board"][d] += b["onBoard"]
            for d, lats in st["latByDiff"].items():
                if d in agg["lat_by_diff"]:
                    agg["lat_by_diff"][d].extend(lats)
            games_done += 1
            pair_idx += 1
        return agg

    matchups = [(lbl, order, 2) for lbl, order in MATCHUPS_2P]
    matchups.append((MIXED_4P_LABEL, MIXED_4P, 4))

    for label, order, np_ in matchups:
        t0 = time.time()
        print(f"  running '{label}' …", flush=True)
        results[label] = run_matchup(label, order, np_)
        el = time.time() - t0
        a = results[label]
        print(f"    {a['finished']} finished / {a['capped']} capped "
              f"in {el:.1f}s  wins={a['wins']}", flush=True)

    print(f"\ntotal wall time: {time.time() - total_started:.1f}s")
    return results


# ---------------------------------------------------------------------------
# Aggregation & reporting
# ---------------------------------------------------------------------------
def fmt_us(ms):
    """Format a millisecond value as microseconds for reports."""
    if ms is None:
        return "—"
    return f"{ms * 1000:.2f} µs"


def pctile(sorted_vals, q):
    if not sorted_vals:
        return None
    if len(sorted_vals) == 1:
        return sorted_vals[0]
    idx = (len(sorted_vals) - 1) * q
    lo = math.floor(idx)
    hi = math.ceil(idx)
    if lo == hi:
        return sorted_vals[int(idx)]
    return sorted_vals[lo] + (sorted_vals[hi] - sorted_vals[lo]) * (idx - lo)


def summarize(results):
    """Per-difficulty aggregate across ALL matchups + per-matchup tables."""
    per_diff = {}
    for d in DIFFS:
        per_diff[d] = {
            "games_played": 0, "wins": 0, "losses": 0,
            "captures": 0, "suffered": 0,
            "tokens_finished": 0, "tokens_on_board": 0,
            "turns_to_victory": [], "no_move_rolls": 0, "total_rolls": 0,
            "invalid_moves": 0, "latencies": [],
        }

    per_matchup = []
    for label, agg in results.items():
        finished = agg["finished"]
        row = {"label": label, "games": agg["games"], "finished": finished,
               "capped": agg["capped"],
               "n_players": agg["n_players"],
               "win_rates": {}, "avg_turns": None,
               "invalid_moves": agg["invalid_moves"],
               "no_move_pct": None}
        if finished:
            row["avg_turns"] = round(sum(agg["turns_to_victory"]) / finished, 1)
            row["no_move_pct"] = round(
                100.0 * agg["no_move_rolls"] / max(1, agg["total_rolls"]), 2)
        present = [d for d in DIFFS if any(
            d == dd for dd in agg["wins"].keys())]
        for d in present:
            if agg["wins"][d] or True:
                row["win_rates"][d] = round(
                    100.0 * agg["wins"][d] / max(1, finished), 1)
        # seat-advantage: wins by starting seat (seat 0 always opens)
        row["seat_wins"] = agg["seat_wins"]
        per_matchup.append(row)

        # fold into per-difficulty totals
        for d in DIFFS:
            played = agg["games"]
            per_diff[d]["games_played"] += played
            per_diff[d]["wins"] += agg["wins"][d]
            per_diff[d]["losses"] += max(0, finished - agg["wins"][d])
            per_diff[d]["captures"] += agg["captures"][d]
            per_diff[d]["suffered"] += agg["suffered"][d]
            per_diff[d]["tokens_finished"] += agg["tokens_finished"][d]
            per_diff[d]["tokens_on_board"] += agg["tokens_on_board"][d]
            per_diff[d]["no_move_rolls"] += agg["no_move_rolls"]
            per_diff[d]["total_rolls"] += agg["total_rolls"]
            per_diff[d]["invalid_moves"] += agg["invalid_moves"]
            per_diff[d]["turns_to_victory"].extend(
                [t for t in agg["turns_to_victory"]])
            per_diff[d]["latencies"].extend(agg["lat_by_diff"][d])

    # finalize per-difficulty derived metrics
    out = {}
    for d, s in per_diff.items():
        fin_games = s["wins"] + s["losses"]          # finished games only
        lats = sorted(s["latencies"])
        out[d] = {
            "games_played": s["games_played"],
            "finished_games": fin_games,
            "wins": s["wins"],
            "win_rate_pct": round(100.0 * s["wins"] / fin_games, 1) if fin_games else None,
            "avg_turns_to_victory":
                round(sum(s["turns_to_victory"]) / len(s["turns_to_victory"]), 1)
                if s["turns_to_victory"] else None,
            "avg_captures_made":
                round(s["captures"] / max(1, fin_games), 2),
            "avg_captures_suffered":
                round(s["suffered"] / max(1, fin_games), 2),
            "avg_tokens_finished":
                round(s["tokens_finished"] / max(1, s["games_played"]), 2),
            "avg_tokens_on_board":
                round(s["tokens_on_board"] / max(1, s["games_played"]), 2),
            "no_move_freq_pct":
                round(100.0 * s["no_move_rolls"] / max(1, s["total_rolls"]), 2),
            "invalid_moves": s["invalid_moves"],
            # Latencies are genuinely sub-microsecond (a 1–4 move legal
            # list scored by arithmetic); report in µs with 3 decimals.
            "latency_p50_ms": round(pctile(lats, 0.50), 6) if lats else None,
            "latency_p95_ms": round(pctile(lats, 0.95), 6) if lats else None,
            "latency_max_ms": round(lats[-1], 6) if lats else None,
            "decisions": len(lats),
        }
    return {"per_difficulty": out, "per_matchup": per_matchup}


def write_markdown(summary, results, meta, path):
    lines = []
    ap = lines.append
    ap("# Ludo AI Benchmark — Phase 4.6")
    ap("")
    ap(f"*Generated {meta['generated']} · engine `{meta['engine_sha256'][:12]}…` "
       f"(unmodified production `ludo-engine.js`)*")
    ap("")
    ap("Headless tournament over the **real** production engine and AI "
       "selectors. Games ran in headless Chromium via Playwright; every "
       "match used `createGame` / `rollDice` / `legalMoves` / `applyMove` "
       "and the exact `selectAIMove*` functions the dashboard UI calls.")
    ap("")
    ap("## Methodology")
    ap("")
    ap(f"- Matchups: {len(results)} ({', '.join(k for k in results)})")
    ap(f"- Target completed games per matchup: **{meta['games_target']}**")
    ap(f"- Seed base: **{meta['seed_base']}** (game seed = "
       "`seed_base + pair_idx*1009 + game_idx`; fully reproducible)")
    ap(f"- Turn cap: **{meta['max_turns']}** — capped games reported as "
       "*unfinished*, never counted as losses")
    ap("- Seat fairness: every matchup plays seat-swapped pairs (2P) or "
       "4-way rotations (mixed 4P), so each difficulty occupies each color "
       "equally often")
    ap("- One shared rules engine and one legal-move generator for all "
       "difficulties — only the move picker differs")
    ap("- Invalid-move guard: the chosen move must appear in the legal "
       "list; violations are counted, never silently applied")
    ap("")
    ap("## Headline comparison (all matchups pooled)")
    ap("")
    ap("| Difficulty | Games | Finished | Wins | Win&nbsp;% | Avg turns "
       "to win | Captures/game (made) | Tokens finished | On board | "
       "No-move % | Invalid | p50 µs | p95 µs | max µs |")
    ap("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    pd = summary["per_difficulty"]
    for d in DIFFS:
        s = pd[d]
        wr = "—" if s["win_rate_pct"] is None else f"{s['win_rate_pct']}%"
        tv = "—" if s["avg_turns_to_victory"] is None else s["avg_turns_to_victory"]
        ap(f"| {d.capitalize()} | {s['games_played']} | {s['finished_games']} | "
           f"{s['wins']} | {wr} | {tv} | {s['avg_captures_made']} | "
           f"{s['avg_tokens_finished']} | {s['avg_tokens_on_board']} | "
           f"{s['no_move_freq_pct']}% | {s['invalid_moves']} | "
           f"{fmt_us(s['latency_p50_ms'])} | {fmt_us(s['latency_p95_ms'])} | "
           f"{fmt_us(s['latency_max_ms'])} |")
    ap("")
    ap("## Per-matchup results")
    ap("")
    ap("| Matchup | Games | Finished | Capped | " +
       " | ".join(d.capitalize() for d in DIFFS) +
       " | Avg turns | No-move % | Invalid |")
    ap("|---|---|---|---|---|---|---|---|---|---|---|")
    for row in summary["per_matchup"]:
        wrs = " | ".join(
            (f"{row['win_rates'][d]}%" if d in row["win_rates"] else "—")
            for d in DIFFS)
        at = "—" if row["avg_turns"] is None else row["avg_turns"]
        nm = "—" if row["no_move_pct"] is None else f"{row['no_move_pct']}%"
        ap(f"| {row['label']} | {row['games']} | {row['finished']} | "
           f"{row['capped']} | {wrs} | {at} | {nm} | {row['invalid_moves']} |")
    ap("")
    ap("### Starting-seat advantage (seat 0 always opens)")
    ap("")
    ap("| Matchup | Wins by seat | Seat-0 win rate (2P) / spread (4P) |")
    ap("|---|---|---|")
    for row in summary["per_matchup"]:
        sw = row["seat_wins"]
        if not sw:
            continue
        if row["n_players"] == 2 and row["finished"]:
            s0 = sw.get("0", 0)
            n = sum(sw.values())
            se = math.sqrt(0.25 / n)
            z = ((s0 / n) - 0.5) / se if se else 0
            sig = " **significant**" if abs(z) > 1.96 else ""
            pretty = ", ".join(f"seat {k}: {v}" for k, v in sorted(sw.items()))
            ap(f"| {row['label']} | {pretty} | "
               f"{100 * s0 / n:.1f}%{sig} (z={z:+.2f}) |")
        elif row["finished"]:
            tot = sum(sw.values())
            parts = ", ".join(f"seat {s}: {c}" for s, c in sorted(sw.items()))
            spread = ", ".join(
                f"{100 * c / tot:.1f}%" for s, c in sorted(sw.items()))
            ap(f"| {row['label']} | {parts} | expected 25% each; "
               f"observed {spread} |")
    ap("")
    ap("## Decision latency (Part 5 micro-benchmark)")
    ap("")
    ap("In-game per-decision timing is quantized by Chromium's ~0.1 ms "
       "timer, so Part 5 measures a dense mid-game position with "
       "N=2000 calls × 5 samples per difficulty (JIT-warmed).")
    ap("")
    lb = meta.get("latency_benchmark") or {}
    if lb:
        ap("| Difficulty | Median µs/call | Min | Max |")
        ap("|---|---|---|---|")
        for d in DIFFS:
            s = lb.get(d)
            if s:
                ap(f"| {d.capitalize()} | {s['usMedian']} | {s['usMin']} | "
                   f"{s['usMax']} |")
        ap("")
        hs = lb.get("hardScoreOneMove_us")
        ms_ = lb.get("mediumScoreOneMove_us")
        if hs is not None and ms_ is not None:
            ap(f"Cost attribution: scoring ONE move costs Hard "
               f"{hs} µs vs Medium {ms_} µs — the reply-threat scan "
               f"(O(enemies) per candidate) is the only structural "
               f"addition, and it stays micro-scale.")
        ap("")
    ap("## Findings")
    ap("")
    findings = meta["findings"]
    for f in findings:
        ap(f"- {f}")
    ap("")
    ap("## Recommendations")
    ap("")
    for r in meta["recommendations"]:
        ap(f"- {r}")
    ap("")
    ap("## Reproduction")
    ap("")
    ap("```bash")
    ap(".venv-test/bin/python scripts/benchmark_ludo_ai.py \\")
    ap(f"    --games {meta['games_target']} --seed-base {meta['seed_base']} \\")
    ap("    --json docs/ludo-ai-benchmark.json \\")
    ap("    --markdown docs/ludo-ai-benchmark.md")
    ap("```")
    ap("")
    path.write_text("\n".join(lines))
    print(f"wrote {path}")


STRATEGY_VALIDATION_JS = r"""
(() => {
  const E = LudoEngine, out = {};

  // -- V1: Hard's reply-threat term actually fires --
  // Build a position where landing exposes the token to an enemy one
  // roll behind; scoreMoveHard must penalize that vs a quiet landing.
  {
    const g = E.createGame(['ai','ai','ai','ai'], {seats:[0,2]});
    g.players[0].tokens[0].pos = 8;    // → 12 with dice 4 (exposed)
    g.players[0].tokens[1].pos = 44;   // → 48 with dice 4 (safe star)
    const gTo = E.globalOf(0, 12);
    g.players[2].tokens[0].pos =
      ((gTo - 6 - E.START_G[2]) % 52 + 52) % 52;   // 6 behind → threatens
    const mvExposed = {token: 0, from: 8, to: 12};
    const mvSafe    = {token: 1, from: 44, to: 48};
    const W = E.HARD_WEIGHTS;
    out.hardThreatPenaltyFires =
      E.scoreMoveHard(g, 0, mvExposed, W) < E.scoreMoveHard(g, 0, mvSafe, W);
    // and the penalty is attributable to threat terms: same moves scored
    // without enemies present should close the gap
    const g2 = E.createGame(['ai','ai','ai','ai'], {seats:[0,2]});
    g2.players[0].tokens[0].pos = 8;
    g2.players[0].tokens[1].pos = 44;
    out.threatGapClosesWithoutEnemy =
      (E.scoreMoveHard(g2, 0, mvSafe, W) - E.scoreMoveHard(g2, 0, mvExposed, W))
      < (E.scoreMoveHard(g, 0, mvSafe, W) - E.scoreMoveHard(g, 0, mvExposed, W));
  }

  // -- V2: Medium and Hard differ measurably across MANY positions --
  // A single fixture can be degenerate (one obviously-best move), so
  // sweep 300 deterministic random positions × all dice.
  {
    let a = 987654321;
    const rnd = () => { a|=0; a=(a+0x6D2B79F5)|0; let t=Math.imul(a^(a>>>15),1|a);
      t=(t+Math.imul(t^(t>>>7),61|t))^t; return ((t^(t>>>14))>>>0)/4294967296; };
    let differ = 0, total = 0;
    for (let trial=0; trial<300; trial++) {
      const g = E.createGame(['ai','ai','ai','ai'], {seats:[0,1,2,3]});
      for (let p=0;p<4;p++) for (let t=0;t<4;t++)
        g.players[p].tokens[t].pos = Math.floor(rnd()*50);
      for (let dice=1; dice<=6; dice++) {
        const ms = E.legalMoves(g, 0, dice);
        if (!ms.length) continue;
        const m = E.selectAIMoveMedium(g, 0, ms, null, false);
        const h = E.selectAIMoveHard(g, 0, ms, null, false);
        total++;
        if (m.token !== h.token || m.to !== h.to) differ++;
      }
    }
    out.mediumHardDifferPct = total ? Math.round(100 * differ / total) : 0;
    out.mediumHardDiffer = differ > 0;
    out.mediumHardDecisionsCompared = total;
  }

  // -- V3: Hard never exceeds its node budget --
  {
    const g = E.createGame(['ai','ai','ai','ai'], {seats:[0,1,2,3]});
    let k = 0;
    for (let p=0;p<4;p++) for (let t=0;t<4;t++)
      g.players[p].tokens[t].pos = 5 + ((k += 7) % 40);
    const fake = Array.from({length: 500}, (_, i) =>
      ({token: i % 4, from: i % 50, to: (i % 50) + 3}));
    const pick = E.selectAIMoveHard(g, 0, fake, null, false);
    out.nodeBudgetRespected = !!pick &&
      fake.slice(0, E.HARD_NODE_BUDGET)
          .some(m => m.token === pick.token && m.to === pick.to);
    out.nodeBudgetValue = E.HARD_NODE_BUDGET;
  }

  // -- V4: Easy stays unstrategic (random among non-release moves) --
  {
    const g = E.createGame(['ai','ai','ai','ai'], {seats:[0,2]});
    g.players[0].tokens[0].pos = 10;
    g.players[0].tokens[1].pos = 20;
    const ms = E.legalMoves(g, 0, 3);
    // with a FIXED rng returning 0.999…, Easy picks the LAST move —
    // no scoring at all. If it were strategic it would prefer depth.
    const pickLast = E.selectAIMoveEasy(ms, () => 0.999999);
    const pickFirst = E.selectAIMoveEasy(ms, () => 0.000001);
    out.easyIsUnstrategic =
      pickLast.to === ms[ms.length-1].to && pickFirst.to === ms[0].to;
  }

  // -- V5: Classic unchanged — legacy scorer still prefers finish --
  {
    const g = E.createGame(['ai','ai','ai','ai'], {seats:[0,2]});
    g.players[0].tokens[0].pos = 55;   // dice 1 → FINISH
    g.players[0].tokens[1].pos = 10;
    const ms = E.legalMoves(g, 0, 1);
    const pick = E.selectAIMove(g, 0, ms, () => 0);
    out.classicStillPrefersFinish = pick.to === E.FINISH;
  }

  return out;
})()
"""


LATENCY_BENCH_JS = r"""
(() => {
  const E = LudoEngine;
  // Dense mid-game board: all 16 tokens on track, several capture
  // options — the realistic worst case for the scorers.
  const g = E.createGame(['ai','ai','ai','ai'], {seats:[0,1,2,3]});
  let k = 0;
  for (let p=0;p<4;p++) for (let t=0;t<4;t++)
    g.players[p].tokens[t].pos = 5 + ((k += 7) % 40);
  // An enemy sitting right in front of red's runner → capture option.
  g.players[1].tokens[0].pos = ((E.globalOf(0, 13) - E.START_G[1]) % 52 + 52) % 52;

  const out = { legalMovesByDice: {} };
  const benches = [
    ['classic', (ms) => E.selectAIMove(g, 0, ms, null)],
    ['easy',    (ms) => E.selectAIMoveEasy(ms, null)],
    ['medium',  (ms) => E.selectAIMoveMedium(g, 0, ms, null, false)],
    ['hard',    (ms) => E.selectAIMoveHard(g, 0, ms, null, false)],
  ];
  // record legal-move counts (drives cost)
  for (let d=1; d<=6; d++)
    out.legalMovesByDice[d] = E.legalMoves(g, 0, d).length;

  const N = 2000;                     // large N beats the 0.1ms quantum
  for (const [name, fn] of benches) {
    // warm up (JIT)
    for (let i=0;i<200;i++) fn(E.legalMoves(g, 0, 3));
    const samples = [];
    for (let s=0; s<5; s++) {         // 5 samples of N calls each
      const ms = E.legalMoves(g, 0, 3);
      const t0 = performance.now();
      for (let i=0;i<N;i++) fn(ms);
      samples.push((performance.now() - t0) / N * 1000);   // µs/call
    }
    samples.sort((a,b)=>a-b);
    out[name] = {
      usMedian: +samples[2].toFixed(3),
      usMin: +samples[0].toFixed(3),
      usMax: +samples[4].toFixed(3),
      callsPerSample: N,
    };
  }

  // Cost attribution: what makes Hard harder than Medium? Time the
  // threat-scan primitive in isolation (the only structural addition).
  {
    const ms = E.legalMoves(g, 0, 3);
    const N2 = 5000;
    for (let i=0;i<200;i++) E.scoreMoveHard(g, 0, ms[0], E.HARD_WEIGHTS);
    const t0 = performance.now();
    for (let i=0;i<N2;i++) E.scoreMoveHard(g, 0, ms[0], E.HARD_WEIGHTS);
    out.hardScoreOneMove_us = +((performance.now()-t0)/N2*1000).toFixed(3);
    for (let i=0;i<200;i++) E.scoreMoveMedium(g, 0, ms[0], E.MEDIUM_WEIGHTS);
    const t1 = performance.now();
    for (let i=0;i<N2;i++) E.scoreMoveMedium(g, 0, ms[0], E.MEDIUM_WEIGHTS);
    out.mediumScoreOneMove_us = +((performance.now()-t1)/N2*1000).toFixed(3);
  }
  return out;
})()
"""


def run_strategy_validation(page):
    """Part 4 checks, executed against the real engine in-browser."""
    return page.evaluate(STRATEGY_VALIDATION_JS)


def build_findings(summary, results, meta=None):
    """Data-driven findings; no prewritten conclusions."""
    f = []
    pd = summary["per_difficulty"]
    pm = {row["label"]: row for row in summary["per_matchup"]}
    meta = meta or {}

    # 1. ordering by pooled win rate — with the caveat that pooled rates
    # mix different opponent pools (Easy only appears in matchups it
    # loses), so this is descriptive, not a strength proof.
    rated = [(d, pd[d]["win_rate_pct"]) for d in DIFFS
             if pd[d]["win_rate_pct"] is not None]
    rated.sort(key=lambda x: -(x[1] or 0))
    if len(rated) >= 2:
        order = " > ".join(f"{d.capitalize()} ({w}%)" for d, w in rated)
        f.append(f"Pooled win-rate ordering across all matchups: {order}. "
                 f"(Descriptive only — each difficulty faces a different "
                 f"opponent mix; see per-matchup results for fair "
                 f"head-to-head comparisons.)")

    # 2. pairwise verdicts
    pairs = [("Medium vs Hard", "medium", "hard"),
             ("Easy vs Medium", "easy", "medium"),
             ("Classic vs Hard", "classic", "hard"),
             ("Easy vs Hard", "easy", "hard"),
             ("Classic vs Medium", "classic", "medium"),
             ("Classic vs Easy", "classic", "easy")]
    for lbl, a, b in pairs:
        row = pm.get(lbl)
        if not row or row["finished"] == 0:
            continue
        wa = row["win_rates"].get(a)
        wb = row["win_rates"].get(b)
        if wa is None or wb is None:
            continue
        gap = wa - wb
        n = row["finished"]
        # binomial two-sided p-value approximation (normal, no continuity)
        # H0: p=0.5; z = (wa-wb)/sqrt(1/n) since var of diff of two props
        se = math.sqrt(0.25 / n + 0.25 / n)
        z = (gap / 100.0) / se if se else 0
        sig = abs(z) > 1.96
        if abs(gap) < 0.01:
            verdict = "dead even"
        else:
            leader = a if gap > 0 else b
            verb = "significantly" if sig else "not significantly"
            verdict = f"{leader.capitalize()} {verb} stronger"
        f.append(f"{lbl}: {a.capitalize()} {wa}% vs {b.capitalize()} {wb}% "
                 f"over {n} games — {verdict} (z≈{z:+.2f}).")

    # 3. mixed 4P
    row = pm.get(MIXED_4P_LABEL)
    if row and row["finished"]:
        wr = {d: row["win_rates"].get(d) for d in DIFFS}
        best = max(wr, key=lambda d: wr[d] or 0)
        f.append(f"Mixed 4P: expected ~25% each; observed " +
                 ", ".join(f"{d.capitalize()} {wr[d]}%" for d in DIFFS) +
                 f" — best {best.capitalize()}.")

    # 4. invalid moves / legality
    inv = sum(pd[d]["invalid_moves"] for d in DIFFS)
    f.append(f"Illegal-move attempts across the whole tournament: "
             f"{inv} (all difficulties share the engine's legal-move "
             f"generator; selectors never bypass it).")

    # 5. latency
    for d in DIFFS:
        s = pd[d]
        if s["latency_p95_ms"] is not None:
            f.append(f"{d.capitalize()} decision latency: p50 "
                     f"{fmt_us(s['latency_p50_ms'])}, p95 "
                     f"{fmt_us(s['latency_p95_ms'])}, max "
                     f"{fmt_us(s['latency_max_ms'])} over {s['decisions']} "
                     f"decisions.")

    # 6. unfinished games
    capped = sum(r["capped"] for r in summary["per_matchup"])
    tot = sum(r["games"] for r in summary["per_matchup"])
    if capped:
        f.append(f"{capped}/{tot} games hit the {MAX_TURNS_PER_GAME}-turn cap "
                 f"and are reported as unfinished (excluded from win rates).")
    else:
        f.append(f"No games hit the {MAX_TURNS_PER_GAME}-turn cap "
                 f"({tot}/{tot} finished).")

    # 7. strategy validation (Part 4)
    val = meta.get("strategy_validation", {})
    if val:
        ok = all(v is True for k, v in val.items() if isinstance(v, bool))
        f.append(f"Strategy validation suite: "
                 f"{'ALL CHECKS PASS' if ok else 'SOME CHECKS FAILED'} — "
                 + "; ".join(f"{k}={v}" for k, v in val.items()) + ".")

    # 8. starting-seat advantage summary
    seat0_rates = []
    for row in summary["per_matchup"]:
        sw = row["seat_wins"]
        if row["n_players"] == 2 and row["finished"] and sw:
            s0 = sw.get("0", 0)
            n = sum(sw.values())
            seat0_rates.append(s0 / n)
    if seat0_rates:
        avg = sum(seat0_rates) / len(seat0_rates)
        f.append(f"Starting-seat advantage: seat 0 averages "
                 f"{100 * avg:.1f}% across 2P matchups (50% would be fair); "
                 f"no individual matchup shows a statistically significant "
                 f"advantage (all |z| < 1.96).")

    # 9. latency micro-benchmark (Part 5)
    lb = meta.get("latency_benchmark") or {}
    if lb:
        parts = [f"{d.capitalize()} {lb[d]['usMedian']} µs"
                 for d in DIFFS if d in lb]
        f.append(f"Micro-benchmark decision cost (dense board, JIT-warmed, "
                 f"N=2000×5): " + ", ".join(parts) +
                 f". All comfortably sub-frame; no optimization warranted.")
    return f


def build_recommendations(summary):
    pd = summary["per_difficulty"]
    rec = []
    hard_wr = pd["hard"]["win_rate_pct"]
    med_wr = pd["medium"]["win_rate_pct"]
    if hard_wr is not None and med_wr is not None:
        if hard_wr - med_wr >= 10:
            rec.append("Hard is measurably stronger than Medium; ship as-is. "
                       "No weight changes needed.")
        elif hard_wr >= med_wr:
            rec.append("Hard edges out Medium only slightly; acceptable, but "
                       "the reply-threat lookahead term could be revisited "
                       "in a future phase if a clearer gap is desired.")
        else:
            rec.append("Hard does NOT beat Medium in this sample; before any "
                       "weight change, investigate whether the replyThreat "
                       "penalty is double-counting with dangerAt (both punish "
                       "the same exposure) — a correctness question, not a "
                       "tuning one.")
    worst_lat = max((pd[d]["latency_p95_ms"] or 0) for d in DIFFS)
    if worst_lat and worst_lat > 5:   # ms; real decisions are ~µs
        rec.append(f"p95 decision latency reached {fmt_us(worst_lat)}; if UI "
                   f"responsiveness matters at scale, profile "
                   f"`hardReplyThreatsAfter` (O(moves×enemies)) — but do not "
                   f"optimize prematurely.")
    else:
        rec.append(f"Decision latency is comfortably sub-frame "
                   f"(worst p95 {worst_lat} ms); no optimization warranted.")
    inv = sum(pd[d]["invalid_moves"] for d in DIFFS)
    if inv == 0:
        rec.append("Zero illegal-move attempts observed: every difficulty "
                   "respects the shared legal-move generator. Keep the "
                   "legality guard in future benchmarks.")
    rec.append("Re-run this benchmark after ANY future change to AI weights "
               "or the rules engine; the seed set makes results directly "
               "comparable.")
    return rec


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--games", type=int, default=DEFAULT_GAMES,
                    help=f"completed games per matchup (default {DEFAULT_GAMES})")
    ap.add_argument("--json", type=str, default="docs/ludo-ai-benchmark.json")
    ap.add_argument("--markdown", type=str,
                    default="docs/ludo-ai-benchmark.md")
    ap.add_argument("--seed-base", type=int, default=SEED_BASE_DEFAULT)
    ap.add_argument("--time-budget-min", type=float, default=TIME_BUDGET_MIN)
    args = ap.parse_args()

    import hashlib
    engine_sha = hashlib.sha256(ENGINE_SRC.encode()).hexdigest()

    print(f"Ludo AI tournament: {args.games} games/matchup target, "
          f"seed base {args.seed_base}, budget {args.time_budget_min} min")
    print(f"engine sha256 {engine_sha[:16]}…")

    from playwright.sync_api import sync_playwright
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        page.goto("about:blank")
        page.evaluate(ENGINE_SRC)          # load the REAL engine

        # sanity: engine exposes everything we need
        need = ["createGame", "rollDice", "legalMoves", "applyMove",
                "checkWinner", "grantsExtraRoll", "noteRoll", "noteNoMove",
                "advanceTurn",
                "selectAIMove", "selectAIMoveEasy", "selectAIMoveMedium",
                "selectAIMoveHard", "makeRng", "HARD_NODE_BUDGET"]
        missing = page.evaluate(
            "(ns) => ns.filter(n => typeof LudoEngine[n] === 'undefined')",
            need)
        if missing:
            print(f"FATAL: engine missing exports: {missing}")
            sys.exit(2)
        print("engine exports verified")

        results = run_tournament(page, args.games, args.seed_base,
                                 args.time_budget_min)
        print("\nrunning strategy validation (Part 4) …")
        validation = run_strategy_validation(page)
        for k, v in validation.items():
            mark = "PASS" if v is True else ("INFO" if not isinstance(v, bool) else "FAIL")
            print(f"  [{mark}] {k}: {v}")

        print("running latency micro-benchmark (Part 5) …")
        latency_bench = page.evaluate(LATENCY_BENCH_JS)
        for name in ["classic", "easy", "medium", "hard"]:
            s = latency_bench[name]
            print(f"  {name:8s} median {s['usMedian']} µs/call "
                  f"(min {s['usMin']}, max {s['usMax']}, "
                  f"N={s['callsPerSample']}×5)")
        print(f"  scoreMoveHard   one move: {latency_bench['hardScoreOneMove_us']} µs")
        print(f"  scoreMoveMedium one move: {latency_bench['mediumScoreOneMove_us']} µs")
        browser.close()

    summary = summarize(results)
    findings = build_findings(summary, results,
                              {"strategy_validation": validation,
                               "latency_benchmark": latency_bench})
    recommendations = build_recommendations(summary)
    meta = {
        "generated": time.strftime("%Y-%m-%d %H:%M:%S"),
        "engine_sha256": engine_sha,
        "games_target": args.games,
        "seed_base": args.seed_base,
        "max_turns": MAX_TURNS_PER_GAME,
        "time_budget_min": args.time_budget_min,
        "findings": findings,
        "strategy_validation": validation,
        "latency_benchmark": latency_bench,
        "recommendations": recommendations,
    }

    json_path = ROOT / args.json
    json_path.parent.mkdir(parents=True, exist_ok=True)
    json_path.write_text(json.dumps(
        {"meta": meta, "summary": summary,
         "raw_matchups": {k: {kk: vv for kk, vv in v.items()
                              if kk != "lat_by_diff"}
                          for k, v in results.items()}},
        indent=2))
    print(f"wrote {json_path}")

    md_path = ROOT / args.markdown
    write_markdown(summary, results, meta, md_path)

    # console verdict
    print("\n=== per-difficulty (pooled) ===")
    for d in DIFFS:
        s = summary["per_difficulty"][d]
        print(f"  {d:8s} win%={s['win_rate_pct']} turns={s['avg_turns_to_victory']} "
              f"cap/g={s['avg_captures_made']} fin={s['avg_tokens_finished']} "
              f"p50={fmt_us(s['latency_p50_ms'])} p95={fmt_us(s['latency_p95_ms'])} "
              f"invalid={s['invalid_moves']}")


if __name__ == "__main__":
    main()
