# Ludo AI Benchmark — Phase 4.6

*Generated 2026-11-01 06:07:39 · engine `477dd826d008…` (unmodified production `ludo-engine.js`)*

Headless tournament over the **real** production engine and AI selectors. Games ran in headless Chromium via Playwright; every match used `createGame` / `rollDice` / `legalMoves` / `applyMove` and the exact `selectAIMove*` functions the dashboard UI calls.

## Methodology

- Matchups: 7 (Classic vs Easy, Easy vs Medium, Medium vs Hard, Classic vs Hard, Easy vs Hard, Classic vs Medium, 4P Mixed (Classic/Easy/Medium/Hard))
- Target completed games per matchup: **30**
- Seed base: **20260406** (game seed = `seed_base + pair_idx*1009 + game_idx`; fully reproducible)
- Turn cap: **1000** — capped games reported as *unfinished*, never counted as losses
- Seat fairness: every matchup plays seat-swapped pairs (2P) or 4-way rotations (mixed 4P), so each difficulty occupies each color equally often
- One shared rules engine and one legal-move generator for all difficulties — only the move picker differs
- Invalid-move guard: the chosen move must appear in the legal list; violations are counted, never silently applied

## Headline comparison (all matchups pooled)

| Difficulty | Games | Finished | Wins | Win&nbsp;% | Avg turns to win | Captures/game (made) | Tokens finished | On board | No-move % | Invalid | p50 µs | p95 µs | max µs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Classic | 210 | 210 | 65 | 31.0% | 155.3 | 1.9 | 1.79 | 0.44 | 10.81% | 0 | 0.00 µs | 1.56 µs | 6.25 µs |
| Easy | 210 | 210 | 1 | 0.5% | 155.3 | 0.51 | 0.33 | 1.6 | 10.81% | 0 | 0.00 µs | 0.00 µs | 7.81 µs |
| Medium | 210 | 210 | 71 | 33.8% | 155.3 | 2.39 | 1.85 | 0.39 | 10.81% | 0 | 0.00 µs | 1.56 µs | 4.69 µs |
| Hard | 210 | 210 | 73 | 34.8% | 155.3 | 2.14 | 1.86 | 0.35 | 10.81% | 0 | 0.00 µs | 1.56 µs | 6.25 µs |

## Per-matchup results

| Matchup | Games | Finished | Capped | Classic | Easy | Medium | Hard | Avg turns | No-move % | Invalid |
|---|---|---|---|---|---|---|---|---|---|---|
| Classic vs Easy | 30 | 30 | 0 | 100.0% | 0.0% | 0.0% | 0.0% | 132.3 | 9.9% | 0 |
| Easy vs Medium | 30 | 30 | 0 | 0.0% | 3.3% | 96.7% | 0.0% | 129.8 | 10.01% | 0 |
| Medium vs Hard | 30 | 30 | 0 | 0.0% | 0.0% | 50.0% | 50.0% | 132.7 | 10.53% | 0 |
| Classic vs Hard | 30 | 30 | 0 | 40.0% | 0.0% | 0.0% | 60.0% | 131.2 | 10.25% | 0 |
| Easy vs Hard | 30 | 30 | 0 | 0.0% | 0.0% | 0.0% | 100.0% | 124.0 | 10.82% | 0 |
| Classic vs Medium | 30 | 30 | 0 | 46.7% | 0.0% | 53.3% | 0.0% | 135.8 | 10.08% | 0 |
| 4P Mixed (Classic/Easy/Medium/Hard) | 30 | 30 | 0 | 30.0% | 0.0% | 36.7% | 33.3% | 301.3 | 12.22% | 0 |

### Starting-seat advantage (seat 0 always opens)

| Matchup | Wins by seat | Seat-0 win rate (2P) / spread (4P) |
|---|---|---|
| Classic vs Easy | seat 0: 15, seat 2: 15 | 50.0% (z=+0.00) |
| Easy vs Medium | seat 0: 16, seat 2: 14 | 53.3% (z=+0.37) |
| Medium vs Hard | seat 0: 18, seat 2: 12 | 60.0% (z=+1.10) |
| Classic vs Hard | seat 0: 21, seat 2: 9 | 70.0% **significant** (z=+2.19) |
| Easy vs Hard | seat 0: 15, seat 2: 15 | 50.0% (z=+0.00) |
| Classic vs Medium | seat 0: 15, seat 2: 15 | 50.0% (z=+0.00) |
| 4P Mixed (Classic/Easy/Medium/Hard) | seat 0: 6, seat 1: 4, seat 2: 9, seat 3: 11 | expected 25% each; observed 20.0%, 13.3%, 30.0%, 36.7% |

## Decision latency (Part 5 micro-benchmark)

In-game per-decision timing is quantized by Chromium's ~0.1 ms timer, so Part 5 measures a dense mid-game position with N=2000 calls × 5 samples per difficulty (JIT-warmed).

| Difficulty | Median µs/call | Min | Max |
|---|---|---|---|
| Classic | 0.25 | 0.2 | 0.35 |
| Easy | 0.05 | 0.05 | 0.1 |
| Medium | 0.45 | 0.45 | 0.5 |
| Hard | 0.65 | 0.65 | 0.7 |

Cost attribution: scoring ONE move costs Hard 0.18 µs vs Medium 0.08 µs — the reply-threat scan (O(enemies) per candidate) is the only structural addition, and it stays micro-scale.

## Findings

- Pooled win-rate ordering across all matchups: Hard (34.8%) > Medium (33.8%) > Classic (31.0%) > Easy (0.5%). (Descriptive only — each difficulty faces a different opponent mix; see per-matchup results for fair head-to-head comparisons.)
- Medium vs Hard: Medium 50.0% vs Hard 50.0% over 30 games — dead even (z≈+0.00).
- Easy vs Medium: Easy 3.3% vs Medium 96.7% over 30 games — Medium significantly stronger (z≈-7.23).
- Classic vs Hard: Classic 40.0% vs Hard 60.0% over 30 games — Hard not significantly stronger (z≈-1.55).
- Easy vs Hard: Easy 0.0% vs Hard 100.0% over 30 games — Hard significantly stronger (z≈-7.75).
- Classic vs Medium: Classic 46.7% vs Medium 53.3% over 30 games — Medium not significantly stronger (z≈-0.51).
- Classic vs Easy: Classic 100.0% vs Easy 0.0% over 30 games — Classic significantly stronger (z≈+7.75).
- Mixed 4P: expected ~25% each; observed Classic 30.0%, Easy 0.0%, Medium 36.7%, Hard 33.3% — best Medium.
- Illegal-move attempts across the whole tournament: 0 (all difficulties share the engine's legal-move generator; selectors never bypass it).
- Classic decision latency: p50 0.00 µs, p95 1.56 µs, max 6.25 µs over 9487 decisions.
- Easy decision latency: p50 0.00 µs, p95 0.00 µs, max 7.81 µs over 8840 decisions.
- Medium decision latency: p50 0.00 µs, p95 1.56 µs, max 4.69 µs over 9569 decisions.
- Hard decision latency: p50 0.00 µs, p95 1.56 µs, max 6.25 µs over 9412 decisions.
- No games hit the 1000-turn cap (210/210 finished).
- Strategy validation suite: ALL CHECKS PASS — hardThreatPenaltyFires=True; threatGapClosesWithoutEnemy=True; mediumHardDifferPct=15; mediumHardDiffer=True; mediumHardDecisionsCompared=1800; nodeBudgetRespected=True; nodeBudgetValue=400; easyIsUnstrategic=True; classicStillPrefersFinish=True.
- Starting-seat advantage: seat 0 averages 55.6% across 2P matchups (50% would be fair); no individual matchup shows a statistically significant advantage (all |z| < 1.96).
- Micro-benchmark decision cost (dense board, JIT-warmed, N=2000×5): Classic 0.25 µs, Easy 0.05 µs, Medium 0.45 µs, Hard 0.65 µs. All comfortably sub-frame; no optimization warranted.

## Recommendations

- Hard edges out Medium only slightly; acceptable, but the reply-threat lookahead term could be revisited in a future phase if a clearer gap is desired.
- Decision latency is comfortably sub-frame (worst p95 0.001562 ms); no optimization warranted.
- Zero illegal-move attempts observed: every difficulty respects the shared legal-move generator. Keep the legality guard in future benchmarks.
- Re-run this benchmark after ANY future change to AI weights or the rules engine; the seed set makes results directly comparable.

## Reproduction

```bash
.venv-test/bin/python scripts/benchmark_ludo_ai.py \
    --games 30 --seed-base 20260406 \
    --json docs/ludo-ai-benchmark.json \
    --markdown docs/ludo-ai-benchmark.md
```
