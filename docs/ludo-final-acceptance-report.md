# Ludo — Final End-to-End Acceptance Report

**Audit date:** 2026-10-31 · **Auditor:** GLM 5.3 final acceptance pass
**Scope:** Complete verification of Ludo (Phases 3, 4, 4a–e, 5.1–5.6) integrated into Loop Dashboard.
**Nature:** Acceptance and verification only — no new features, no Phase 5.7.

---

## 1. Executive Summary

The Ludo game was audited end-to-end against every requirement of the
final acceptance brief: the full 9-entry configuration matrix × 4 AI
difficulties, deep rules verification through the real engine, 15 seeded
full-game simulations plus 2 complete games driven through the actual UI,
lobby/identity behaviors, all 8 required lifecycle sequences, responsive
layout at all five breakpoints, console-error sweeps, and the complete
14-suite regression battery plus `cargo test -p loop-dashboard`.

**No genuine Ludo defect was found.** Every check that initially failed
was traced to a bug in the *audit harness itself* (wrong function arity,
wrong element id, wrong coordinate math, or Playwright argument-passing
quirks), reproduced as such, and re-verified green against the real
implementation. **Zero production-code changes were made in this audit**
(`ludo-engine.js` sha `6692379…` unchanged; `ludo-game.js` and
`console-app.js` untouched). One new artifact was added:
`scripts/audit_ludo_simulations.py` (the reproducible seeded simulation
driver).

**Verdict: ACCEPTED WITH KNOWN LIMITATIONS** (limitations are documented
design choices and environment-dependent non-Ludo test failures, not
defects — see §12).

---

## 2. Git Status and Files Modified

- Branch: `feat/system-monitor`; HEAD `53d7938`.
- Pre-existing uncommitted work on this branch (system monitor, CLI,
  dashboard core, other games) was **inspected and left untouched**.
- Ludo files are untracked-but-present from prior phases:
  `crates/loop-dashboard/src/assets/ludo-engine.js`,
  `ludo-game.js`, `docs/ludo-*`, `scripts/verify_ludo*.py`.
- **Files modified by this audit: none.**
- **Files added by this audit:** `scripts/audit_ludo_simulations.py`
  (reproducible seeded AI-simulation driver, 15 scenarios).
- The two previously documented `sysmon::health` cargo failures did
  **not** recur this run (machine now on AC power); see §10. sysmon was
  not modified.

---

## 3. Configuration Matrix Results — 13/13 PASS

Engine-level (`LudoEngine.createGame` with the UI's seat/kind builders)
and UI-level (driving the real lobby buttons) both verified:

| Config | Active seats | Humans | Result |
|---|---|---|---|
| 2P/1H | [0,2] | {0} | PASS (engine + UI) |
| 2P/2H | [0,2] | {0,2} | PASS (engine + UI) |
| 3P/1H | [0,1,2] | {0} | PASS (engine + UI) |
| 3P/2H | [0,1,2] | {0,1} | PASS (engine + UI) |
| 3P/3H | [0,1,2] | {0,1,2} | PASS (engine + UI) |
| 4P/1H | [0,1,2,3] | {0} | PASS (engine + UI) |
| 4P/2H | [0,1,2,3] | {0,1} | PASS (engine + UI) |
| 4P/3H | [0,1,2,3] | {0,1,2} | PASS (engine + UI) |
| 4P/4H | [0,1,2,3] | {0,1,2,3} | PASS (engine + UI) |

Additional matrix checks, all PASS:
- Inactive seats flagged `active:false`; inactive seat rotation never
  receives a turn over 200 simulated advances (2P→[0,2], 3P→[0,1,2],
  4P→all).
- Seat colors fixed per seat: red/green/yellow/blue
  (`COLORS=['red','green','yellow','blue']`, panel palette
  `#ff5d5d/#5ef2c0/#ffd166/#6ea8ff`).
- Winner detection ignores inactive seats (inactive seat with 4 home
  tokens does not win; active one does).
- Fresh games start `turn=0`, `winner=null`, all tokens in yard, `home=0`.
- Difficulty selection UI: Classic (null), Easy, Medium, Hard all select
  and persist their state; live seat preview shows `AI · Hard` labels.

## 4. Gameplay Verification — 14/14 PASS (after harness corrections)

All via the real engine/UI (`LudoEngine`, `luPick`, `luDoRoll`):

**Dice**
- Range 1–6 over 10,000 rolls (mulberry32): 0 invalid; distribution
  1578–1746 per face (fair).
- Six releases a yard token (`from:-1 → to:0`); non-six yields no
  release move.
- Extra roll on six, capped: chain 2 → extra granted, chain 3 → denied;
  non-six never grants.
- Roll button correctly disabled during `rolling`/`awaiting`/`moving`/
  `over`; enabled only for a human's `idle` phase.
- Rapid double-click produces a single human roll (state machine guards
  + settle window; `force()` rejects forged values, AI turns, and
  mid-flight rolls).

**Token movement**
- Exact step-by-step path progression (0→5 by ones) verified.
- Overshoot rejected (pos 54 + dice 3 → no move; dice 2 → finish 56).
- Finished tokens (pos 56) can never be selected again.
- Hit-test alignment proven with a **real mouse click** at the computed
  canvas coordinate of a yard token: `luHitTest` returned `{p:0,i:0}`
  and the click released the token onto its start square.

**Captures**
- Real capture flow via `luPick`: victim returned to yard (`pos:-1`),
  lander on target, message “You moved. Captured Player 2! Token sent
  home.” rendered (DOM-observed).
- Safe squares (`SAFE_G = {0,8,13,21,26,34,39,47}`) protect victims
  (`victimsAt` returns empty).
- Stacking of same-player tokens on one square allowed and rendered.
- Extra-roll policy: sixes only (documented engine rule).

**Home lane & victory**
- Lane entry (51+3→54), exact finish (`applyMove` sets `finished`,
  increments `home`), winner only at 4 tokens (`checkWinner` null at 3).
- Victory overlay presents `Play Again` / `Back to Games`; **Play Again
  verified to start a clean game** (winner null, rolls 0, all yard,
  overlay hidden).

## 5. AI Verification and Simulation Results — 15/15 PASS

`scripts/audit_ludo_simulations.py` (new, reproducible seeds) drives
complete games on the **real engine** with `makeRng(seed)` for both dice
and AI jitter:

| Scenario set | Difficulties | Result |
|---|---|---|
| 2P human-vs-AI | classic/easy/medium/hard | 4/4 complete, legal |
| 2P AI-vs-AI | classic/easy/medium/hard | 4/4 complete, legal |
| 3P mixed (1H, 2H, AI-only) | medium/hard/easy | 3/3 complete, legal |
| 4P mixed + AI-only | hard/medium/classic/hard | 4/4 complete, legal |

Across all 15 games: **0 illegal moves** (every AI pick re-validated
against `legalMoves`), **0 turns to inactive seats**, **0 stuck games**
(all terminated within 149–486 turns), **0 unexpected winners**, correct
`home=[4,…]` for every winner, chain always within 0–3.

Additionally, **two full games were played through the real UI**
(4P/1H fast-mode, 349 rolls, winner P3; 3P/2H, 277 rolls, winner P0)
with a human-turn driver using only public hooks (`LUD.force`,
`luPick`): both completed, victory overlay shown, zero page errors.

Per-difficulty static checks: all four selectors return only legal
moves; empty move list → `null` for every difficulty; AI never acts for
inactive seats (rotation excludes them); AI timers cancelled on restart
(§6 S6). No AI weights were tuned in this audit.

## 6. Lifecycle Verification — 8/8 sequences PASS

| # | Sequence | Result |
|---|---|---|
| S1 | Lobby→Game→Restart→Lobby→Game | PASS — fresh game (rolls 0, all yard) |
| S2 | Game→Fullscreen→Exit Fullscreen | PASS — classes/state sync, clean exit |
| S3 | Game→Route Away→Re-enter | PASS — board unmounted, cleanup 30→30 (no leaks) |
| S4 | Audio settings→change→reload | PASS — master 0.3 before = stored = after reload |
| S5 | Mid-animation restart | PASS — phase idle, dice null, moves 0, anim none, fx 0, musicTimer 0 |
| S6 | AI turn→restart | PASS — AI timer cancelled, no stale state |
| S7 | Victory→New Game | PASS — overlay buttons work, clean reset |
| S8 | Lobby→Fullscreen→Start→Exit FS | PASS — layout intact, start works in FS, Esc exits cleanly |

Hygiene across all churn: no duplicate listeners (registry constant),
no orphaned rAF (31 frames/500 ms ≈ headless baseline), no stale audio
scheduler, no duplicate game instances, no broken handlers, fullscreen
state always truthful.

## 7. Responsive UI Results — 20/20 PASS

Five breakpoints (1280×900, 1440×900, 820×1180, 390×844, 844×390), each
checked inline and in fullscreen:

- Board perfectly square, horizontally centered (offset ≤ 4 px inline,
  ≤ 2 px fullscreen), fits viewport.
- Player panels readable (≥ 2 visible, all 4 on desktop/tablet/mobile).
- Dice visible; controls accessible; minimum touch target ≥ 24 px
  (37–58 px observed); zero clipped controls; no control overlap.
- Lobby usable at every breakpoint (card fits, all inputs reachable).
- No horizontal document overflow anywhere.
- Mobile landscape fullscreen board 235 px (Phase 5.6 compaction);
  known height-bound limitation documented below.

## 8. Browser Console Findings

Two full play sessions (desktop + mobile), each covering mount, lobby,
multiple forced turns, AI turns, fullscreen toggle, and restart:
**0 errors, 0 warnings.** All 14 regression suites additionally assert
`noConsoleErrors` — all green.

## 9. Full Regression Test Results — all green

| Suite | Result |
|---|---|
| verify_ludo.py | 42/42 |
| verify_ludo_glide.py | 11/11 |
| verify_ludo_phase3b.py | 7/7 |
| verify_ludo_phase3c.py | 5/5 |
| verify_ludo_phase3d.py | 10/10 |
| verify_ludo_phase4a.py | 26/26 |
| verify_ludo_phase4b.py | 31/31 |
| verify_ludo_phase4c.py | 33/33 |
| verify_ludo_phase4d.py | 32/32 |
| verify_ludo_phase4e.py (Hard AI) | 24/24 |
| verify_ludo_phase53.py | 19/19 |
| verify_ludo_phase54.py | 23/23 |
| verify_ludo_phase55.py | 37/37 |
| verify_ludo_phase56.py | 27/27 |
| **audit_ludo_simulations.py (new)** | **15/15** |
| cargo test -p loop-dashboard | **363 passed, 0 failed** (lib) + all integration targets ok |

Notes: `phase3b`/`phase3c` first showed 6/7 and 4/5 in a summary grep;
full-output reruns confirmed **7/7 and 5/5** (the grep had truncated the
tally lines of multi-line outputs) — both attempts reported honestly.
No timing flake required a retry. No test was weakened, skipped, or
modified.

## 10. sysmon::health Cargo Failures (out-of-scope status)

The two previously documented failures
(`fresh_real_sample_is_healthy_with_history_extent`,
`memory_pressure_warning_degrades`) **did not occur in this audit run**
— the machine is now on AC power, so the real-sample health doc no
longer reports `battery-low`. They remain machine-state-dependent tests
of pre-existing uncommitted sysmon work, outside Ludo scope, and were
not modified. If the laptop runs on battery below the warning threshold
again, they will fail again for environmental reasons.

## 11. Bugs Discovered and Fixes Applied

**None.** Five candidate failures surfaced during the audit; each was
investigated to root cause and determined to be an audit-harness bug,
then re-verified green against the untouched implementation:

1. `AI dispatch returns null` — wrong call arity in my probe
   (`selectAIMoveForDifficulty(state, player, moves, difficulty, rand)`;
   I had passed difficulty first). Correct call: all difficulties legal.
2. `capture/victimReturnsToYard FAIL` — my probe placed the victim at
   pos 51, which is inside the private home lane (LAST_MAIN=50) and
   therefore uncapturable **by design**; and I called `victimsAt` with
   the dice value instead of the landing position. Correct setup:
   capture works exactly per rules.
3. `dice/noRollDuringAnimationOrAwaiting FAIL` — I read `btn.disabled`
   without invoking the renderer that owns it (`luRenderControls`);
   the real state machine is correct.
4. `nameFieldsRendered FAIL` — name inputs exist only for human seats
   (3P/2H → 2 inputs); intentional Phase 5.5 design. maxlength=14,
   whitespace fallback, and HTML escaping all verified working.
5. `UI full-game loop stuck at rolls=0` — Playwright scalar-argument
   passing quirk delivered `undefined` into `LUD.force`, which
   correctly rejected it (the guard worked as designed). With literal
   injection the games complete.

Per the fix policy, since no genuine Ludo bug was demonstrated, **no
production-code change was made**.

## 12. Remaining Known Limitations (documented, not defects)

1. **Setup lobby overlays the control bar.** While the lobby is open,
   the Fullscreen/Audio/Exit buttons are visually covered (the lobby is
   a modal layer over the stage). Fullscreen remains available via the
   `F` key, and all controls return on close/start. Deliberate Phase
   5.5 design.
2. **Mobile-landscape fullscreen board is height-bound** (~235 px on
   844×390). A square board cannot consume the free width without a
   non-square layout, which would be a redesign. Phase 5.6 already
   reclaimed ~40 % of the lost size.
3. **Programmatic/Esc fullscreen exit navigates back to Fun Time.**
   Shared session semantics across all ten games; consistent and
   intended.
4. **Name/configuration persistence is per-session (module state);
   only audio settings persist to localStorage.** Matches the
   established product behavior.
5. **Environment-dependent sysmon tests** (see §10) can fail when the
   host runs on a low battery — unrelated to Ludo.

## 13. Final Production-Readiness Verdict

Every critical requirement of the acceptance brief was exercised and
passed: complete configuration matrix, rules-correct gameplay through
the real engine, four legal-move-guaranteed AI difficulties with
reproducible terminating simulations, a fully functional identity/lobby
system, leak-free lifecycle across all eight required sequences,
responsive layouts without clipping or overflow at all five breakpoints,
a clean browser console, and a fully green 14-suite regression battery
plus cargo tests.

**VERDICT: ACCEPTED WITH KNOWN LIMITATIONS** — no critical defects; the
limitations above are documented design choices and an out-of-scope
environmental test dependency.
