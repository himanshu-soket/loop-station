# Snakes & Ladders — Visual & UI/UX Overhaul Progress Log

Agent: GLM 5.3 · Repo: `~/loop` · File: `crates/loop-dashboard/src/assets/snakes-game.js`
Method: small tasks → test → screenshot → log. No logic changes; engine untouched.

## Baseline measurements (BEFORE, real Chromium)

| Viewport | Board | Stage | Notes |
|---|---|---|---|
| 1366×768 default | **280×280** | 564×230 | board clamped to min! |
| 1366×768 4P playing | **280×280** | 564×151 | worse with 4 name rows |
| 1366×768 fullscreen | **414×414** | 996×416 | only 41% of stage width used |
| 1920×1080 windowed | 460×460 | 564×462 | panel capped at 980px |
| 1536×864 windowed | **280×280** | 564×246 | min clamp again |
| 390×844 windowed | 280×280 | 308×282 | |

Root cause of "board too small": vertical stack inside a 980px-max panel:
panel header (52) + HUD (38) + setup (151!) + layout + controls (58) +
msg (37) + help (34) → stage gets ~230px height → square fit = 280px (min).

BEFORE screenshots: `/tmp/sn-shots/before/`.

## Test contract (selectors that MUST survive — 228 checks across 3 suites)

IDs: snBoard snOverlay snOvTitle snOvSub snOvStats snOvTable snOvRow
snConfetti snTurnChip(+.v) snModeSeg snModeChip snNames snName{n} snPlayers
snLog snMsg snStart snRestartBtn snRollBtn snFsBtn snExit snSoundBtn snPanel
snDiceVal snRollLbl snRollCount snTurnDot snStageWrap(sn's parent=.sn-stage)
Classes: .sn-pl(.turn)[data-p] .nm .nm-t .sq .stats .sn-ov-* .sn-name-*
.sn-die(i) .sn-card h4 .sn-stage .sn-side #snLayout .sn-setup .sn-controls
Behavior: board square ratio, painted px > 200, width > 300 @1360×950,
Esc exits game, card-click auto-fullscreen, ft-panel-h hidden in FS.

---

## PHASE 1 — LAYOUT REDESIGN

### Task 1.1 — Restructure panel into a header bar + full-height play area ✅
Moved mode selector / names / Start into a compact top toolbar row so the
stage gets the entire remaining viewport height. Kept every ID/class above.
Result @1366×768: board 280→**612×612** (stage 640×636). Tests: 68+85+75 PASS.

### Task 1.2 — Robust canvas fit (rAF second pass) ✅
snResize() now defers a second fit on the next animation frame, catching
late flex/grid settles after viewport changes. Fixes a 6px board overflow
at 390×844 after fullscreen→Esc→re-enter→resize.
Tests: personalization 75/75 (responsive/390x844 was the failure).

### Task 2.1 — Premium board cells ✅
- Lacquered tile gradients + top sheen (skipped when C<34px to avoid noise)
- Hairline borders; emerald FINISH jewel cell; slate START cell
- Numbers repainted in an OVERLAY pass above snakes/ladders with a soft
  translucent chip behind each numeral → always legible (fixes cell 59
  being covered by a snake spine)
Tests: verify_snakes.js 68/68 (allNumbersLegible was the failure).

### Task 3.1 — Snake redesign ✅
- Filled-outline ribbon (was stroked segments) → constant thickness,
  no lumps; max half-width C*0.125 (was C*0.30 — much slimmer)
- Neck ease-in, mid-body fullness, tapered tail with rattle tip
- Crimson→rose gradient, top highlight, scale dashes, ground shadow
- Teardrop head (bezier) with sclera+pupil eyes and forked tongue
Tests: allSnakeHeadsPainted now ≥0.32 at every tested size (was 0.22 fail).

### Task 5.1 — Premium tokens ✅
- Radial gradient sphere lit upper-left + specular dot
- Light top rim / dark bottom rim for a 3D read; grounded soft shadow
- Active player = soft double halo ring (replaced dashed debug ring)
Tests: all suites pass.

### Test-suite maintenance (behaviour-preserving)
- verify_snakes_mp.py: settle-before-force in the PvC win check (the
  hardened SND.force hook correctly rejects rolls during an in-flight
  animation; the old assertion could observe a stale overlay).

### Task 6.1 — Premium sidebar (dice / players / history) ✅
- Cards: layered gradient + shadow, ruled section headers
- Dice: 74px bevelled die with specular sheen overlay, inset shadows,
  radial-gradient pips; value glows; landing "pop" keyframe on each roll
- Roll button: gradient fill, hover lift + glow, kbd-style space hint chip
- Player chips: glossy dots, AI tag pill, turn = emerald ring + glow,
  winner = gold ring
- Move history: timeline rail per entry, newest accented, thin scrollbar
Tests: all suites pass.

### Task 7.1 — Top bar & controls ✅
- One compact command bar: status chips + mode seg + names + Start
- Second row: Restart / Sound / Fullscreen / Back + live message
- In-game .ft-btn restyle (namespaced to #snPanel): hover lift, primary
  gradient for Start Game
Tests: all suites pass.

### Task 8.1 — Animation polish ✅
- Die "landing pop" (scale keyframe, retriggered per value)
- Snake slides dip; ladder climbs arc upward (sinusoidal, proportional
  to jump length) instead of straight lines
- prefers-reduced-motion disables die animations
Tests: all suites pass (timings unchanged — no gameplay slowdown).

### Task 9.1 — Responsive & fullscreen matrix ✅ (new suite)
scripts/verify_snakes_ui_matrix.py — 49 checks:
- Windowed: 1920×1080, 1536×864, 1366×768, 1280×720, 1024×768, 768×1024,
  390×844 → board square & inside viewport, no horizontal scrollbar,
  ALL controls visible, sidebar reachable
- Fullscreen: 1920×1080, 1536×864, 1366×768, 390×844 → same audit
- Esc exits fullscreen AND the game cleanly at every size
Result: 49/49 PASS.
Mobile specifics: single-row mode/start strip, wrapped name inputs,
42vh stage cap, phone-tuned dice/cards, panel header folds away.

### Task 10.1 — Phase 10 gameplay QA ✅ (new suite)
scripts/verify_snakes_ui_gameplay.py — 27 checks in real Chromium:
- PvC / 2P / 3P / 4P config with custom names (+ AI default names)
- dice roll → token moves → named log line (durable evidence; the die face
  legitimately clears when the next turn starts)
- snake slide (46+1 → 47 → 26) and ladder climb (8+1 → 9 → 31) per mode
- winner celebration: name, crown, confetti, per-mode stat table rows
- restart/rematch cleanliness; fullscreen enter + Esc exit
Result: 27/27 PASS. Screenshots in /tmp/sn-shots/after/qa-*.png.

### Pixel audit (programmatic visual verification) ✅
Because screenshots can't be eyeballed in this session, visuals were
verified by sampling canvas pixels at computed geometry:
- FINISH cell emerald:        0.77 green-dominant coverage
- ladder rails warm amber:    0.75 along the true rail lines
- snake bodies crimson:       0.77 along the true bezier spines
- tokens radially shaded:     centre vs rim delta confirmed
- number chips legible:       dark backing + light text over artwork

## FINAL RESULTS

| Suite | Checks | Result |
|---|---|---|
| verify_snakes_ui_gameplay.py (new) | 27 | PASS |
| verify_snakes_ui_matrix.py (new) | 49 | PASS |
| verify_snakes_personalization.py | 75 | PASS |
| verify_snakes.js (original) | 68 | PASS |
| verify_snakes_mp.py | 85 | PASS |
| Rust snakes_ui | 4 | PASS |
| Rust embedded_dashboard | 2 | PASS |

Board size gains: 1366×768 windowed 280→470px (+68%);
fullscreen 414→593px (+43%); 1920×1080 fullscreen 939px.

## Files changed
- crates/loop-dashboard/src/assets/snakes-game.js  (all UI work)
- scripts/verify_snakes_ui_matrix.py               (new)
- scripts/verify_snakes_ui_gameplay.py             (new)
- scripts/verify_snakes_mp.py                      (test-race hardening only)
- docs/snakes-ui-overhaul.md                       (this log)

Untouched: snakes-engine.js (rules), Ludo, other games, console shell.
