# Ludo — Phase 3 Progress Log

Phase: **3 — Dice, Movement & Animations** (started; in progress)

Baseline verified before starting:
- `scripts/verify_ludo.py` → 41/41 PASS
- `cargo test -p loop-dashboard --test ludo_ui` → 4/4 PASS

## Parts

- [x] Part A — (verified: rAF glide already present in baseline; Part B hardened it)
- [x] Part B — Token stacking & hit-testing (single coordinate source of truth)
- [x] Part C — Dice animation (tumbling faces, final = engine result, no dup rolls)
- [ ] Part D — State protection (`LUD.force` guards) + illegal-token feedback
- [ ] Part E — Phase 3 regression tests

### Task 1 — Smooth token movement hardening (done)
Files: `crates/loop-dashboard/src/assets/ludo-game.js`, `scripts/verify_ludo_glide.py`

The baseline already interpolated via `luGlideStep`/`_viz`; this task closed three
gaps found by runtime probing:

- **`luExecuteMove()` now calls `luCancelAnim()` before starting a walk.** The doc
  comment claimed only one rAF chain could be alive, but nothing enforced it; a
  second move started while an old frame was pending left the old chain to die
  via the `aa !== a` check WITHOUT deleting its `tok._viz` — a different-token
  animation would leave the first token frozen mid-square forever.
- **`luCancelAnim()` deletes `_viz` on the animated token** (a cancelled glide
  must never leave a fractional position behind; the renderer treats `_viz`
  as truth).
- **First-frame `t` clamped ≥ 0** — the first rAF frame-start timestamp can
  predate the scheduler's `performance.now()`, writing a negative `t` into
  `_viz` (observed live: t=-0.31 / -0.04 before the fix).
- Stale-clock death path now clears `LUG.anim` only when it still owns the slot.

Untouched: `_viz` render-only mechanism, `luMoveDone()` engine commit
(`applyMove` stays the single authority), dice flow, game rules.

New test `scripts/verify_ludo_glide.py` — 11/11 PASS at default pacing:
28 distinct fractional frames across 3 chained hops, t∈[0,1] monotonic,
engine pos == 3 after commit, `_viz` cleared, remount-mid-glide leaves no
stale `_viz`/`anim`, zero console errors.

Regressions: `verify_ludo.py` 40/41, `phase3b` 7/7, `phase3c` 5/5, `phase3d`
8/12, `cargo test -p loop-dashboard --test ludo_ui` 4/4. The `restart/freshState`
failure (expects an epoch bump that `luNewGame` deliberately no longer makes —
see the note in `luNewGame`) and the phase3d `stage_awaiting` staging failures
are PRE-EXISTING: reproduced identically with this task's edits reverted, and
belong to Part D's open work.

### Task 3.2 — Home-lane animation (done)
Files: `crates/loop-dashboard/src/assets/ludo-game.js`

Live-probed every lane scenario first (entry, deep walk, finish, all four
colors, boundary-crossing walks, rapid sequences, walker-through-blocker,
restart mid-walk). The rAF glide itself was already sound end-to-end — two
real defects were found around it and fixed:

- **Own tokens sharing a home-lane cell rendered perfectly overlapped.**
  `luBuildStacks()`/fan-out only keyed the shared track (`pos <= LAST_MAIN`),
  so two of a player's tokens resting on the same lane cell drew at the
  identical pixel centre — the lower one invisible and unclickable
  (measured: 0px apart; hit-test always resolved token 0). Added
  `luStackKey(p, pos)`: shared-track squares key by global index (player-free),
  lane cells key privately per player (`L<p>:<pos>`), yard/finish keep their
  own dedicated stagger. All three consumers (draw radius, `luTokenPoint`
  fan-out, hit-test radius) now share the one helper → draw/click still agree
  exactly. After: 10.12px quadrant-seat separation, hit-test resolves the
  clicked token ({p:0,i:1} case verified).
- **Inter-hop snap-back: a full-cell visual jump mid-walk.** On each completed
  hop the old code did `delete tok._viz` while the ENGINE still held the walk
  ORIGIN (`pos` commits only in `luMoveDone`); any draw in that window — the
  next rAF is up to a frame away, and the nudge timer / resize can fire too —
  snapped the token back to the origin square (observed live: ~135px jump).
  Now a completed hop parks a RESTING hint `{from:to, to:to, t:1}` that the
  next hop's first frame overwrites, and `luMoveDone()` deletes the hint right
  BEFORE the engine commit so the committed `pos` becomes the render truth.
  After: max inter-sample jump 17px (= polling granularity of a moving token),
  full trace hop-rest-hop continuous through every lane cell.

Untouched: engine rules, `applyMove()` as sole commit authority, `_viz`
render-only contract, `luCancelAnim()`/clock-guard lifecycle (stale chains and
restarts still delete `_viz` → engine state), Task 1 fixes, dice flow.

Verified: lane entry 50→52, multi-cell 52→55, finish 55→56 (home count +1,
finish-pool slot), all four colors identical hop chains, boundary-crossing
48→54 single walk (6 hops, t∈[0.041..0.997]), rapid `LUD.fast()` back-to-back
lane walks (state consistent, no `_viz`/anim residue), walker passes a resting
blocker on its lane cell (blocker displacement 0.00px), restart mid-lane-walk
(fresh game, no stale `_viz`, no zombie).

Regressions: `verify_ludo_glide.py` 11/11, `verify_ludo.py` 40/41 (same
pre-existing `restart/freshState`), `phase3b` 7/7, `phase3c` 5/5, `phase3d`
8/12 (same pre-existing `stage_awaiting` ×4; one unrelated boot-timing flake
in the test's own retry path observed once, not reproducible ×3), `cargo test
-p loop-dashboard --test ludo_ui` 4/4.


## Completed work

### Part B — Stacking & hit-testing (done)
Files: `crates/loop-dashboard/src/assets/ludo-game.js`, `scripts/verify_ludo_phase3b.py`

- Added `luTokenPoint(p, i, stacks)` — ONE function computing every token's pixel
  centre (yard / track / lane / mid-glide / finish). `luDraw()` and `luHitTest()`
  both call it → draw/click coordinates can never diverge again.
- Added `luBuildStacks()` — per-frame map global-square → member list; shared by
  renderer and hit-tester.
- `luStackLayout(n)` — deterministic seats: ≤4 quadrant seats; >4 inner quadrant +
  outer ring spiral so up to 16 tokens fit inside one cell with distinct centres.
- `luStackRadius(n)` — monotonic radius scaling (0.36C → 0.56C base factor down).
- `luHitTest` rewritten: uses same geometry; clickable radius matches drawn
  (stack-scaled) radius ×1.25; **legal tokens always beat illegal ones** when
  mixed-player stacks overlap (never locked out by an opponent on the square).
- Test hooks exposed: `__luTokenPoint/__luHitTest/__luStackRadius/__luBuildStacks/__luDraw`.
- New test `scripts/verify_ludo_phase3b.py` — 7/7 PASS:
  draw/hit agreement, mixed-stack legal preference, all-16-on-one-square clickable,
  in-cell containment, radius monotonicity.
- Regression: `verify_ludo.py` 41/41 PASS.

### Part C — Dice animation (done)
Files: `crates/loop-dashboard/src/assets/ludo-game.js`, `scripts/verify_ludo_phase3c.py`

- `luDiceSpin()` — rAF-driven tumble: random face every ~80ms while the roll is
  in flight; chain owned by `LUG.diceRaf`.
- `luCancelDiceSpin()` called from: roll resolution (BEFORE painting the final
  face), stale-clock path, `luNewGame()`, `luTeardown()` → no stray frame can
  repaint the die after the engine result lands.
- Final face painted only via `luSetDice(v)` from `luResolveRoll(v)` — always
  the engine-generated value (`LudoEngine.rollDice()`).
- Deduplicated pip tables into single `LU_PIPS`; `luDieShow()` shared by tumble
  + final face; removed dead `enabled` variable in `luRenderControls()`.
- Duplicate-roll protection verified end-to-end: button disabled during
  rolling/moving; `phase !== 'idle'` guard; one human roll per click burst.
- New test `scripts/verify_ludo_phase3c.py` — 5/5 PASS (tumble observed,
  final==engine, humanRolls==1 under 5-click hammer, disabled mid-roll).
  Note: the test counts HUMAN rolls — AI players legitimately roll on their
  own turns once the turn passes (no-legal-move → advanceTurn).
- Regression: `verify_ludo.py` 41/41, `verify_ludo_phase3b.py` 7/7.


### Task 3.3 — Draw / hit-test synchronization (done)
Files: `crates/loop-dashboard/src/assets/ludo-game.js`

Audited first, fixed only what reproduced. Measured draw-center vs
hit-center for every token category (track, lane, yard, finish, stacked
pairs own/mixed, mid-glide): **zero mismatches** — `luTokenPoint()` is the
single source both the renderer and `luHitTest()` consume, so they cannot
diverge. Mouse and touch share `luCanvasPoint()` (pointer events, one
coordinate system); DPR=2 verified.

One REAL defect found and fixed — **the board could never grow**:
`luResize()` measured the CANVAS's own rect, but the canvas's inline
`style.width/height` (written by the previous fit) override
`#luBoard{width:100%}` — so the rect was frozen at the first computed
size. Shrinking worked (rect > new parent), growing did not: a window
enlarge or fullscreen enter left a small board centered in a large stage
(reproduced: 300px forever at any size; fullscreen identical). Fixed by
measuring from the STAGE (`wrap.clientWidth/clientHeight`) with the
canvas rect as fallback only — the exact pattern `snResize()` already
uses in Snakes & Ladders, whose comment documents the same trap. After:
700→1400 viewport grows the board 300→602px; fullscreen refits; shrink
still works.

Verified after the fix: all four colors click-accurate on track/lane/
yard/finish; 4-stacks individually selectable; rapid clicks select only
the intended token; 71 mid-glide samples all hit-testable at their
visible positions; resize DURING a glide keeps interpolation finite,
in-board and lands exactly (30→36 through a mid-walk resize); DPR=2 +
touch taps accurate; fullscreen clicks land (10→13→15→19 chain).

Regressions: `verify_ludo_glide.py` 11/11, `verify_ludo.py` 40/41 (same
pre-existing `restart/freshState`), `phase3b` 7/7, `phase3c` 5/5 (one run
showed 4/5 — `noDuplicateRolls humanRolls=0`: the test's own dice race;
first roll 6 leaves the player in `awaiting`, its idle-wait times out and
every hammer click hits the correctly-disabled button. Reproduced at will
by forcing 6; 3 of 4 clean runs passed. Pre-existing, unrelated to this
task), `phase3d` 8/12 (same pre-existing `stage_awaiting` ×4),
`cargo test -p loop-dashboard --test ludo_ui` 4/4.


### Task 3.4 — Token stacking & collision handling (verified, no changes)
Audited `luBuildStacks()` / `luStackKey()` / `luTokenPoint()` / fan-out /
hit-testing end-to-end and reproduced every required scenario before
touching anything. **No defect found; no code changed** — the stacking
implementation is correct as designed.

Verified scenarios (all PASS):
- Same-player stacks n=2..8 on shared track: quadrant seats (n≤4) /
  inner+ring (n>4); all centres inside the cell (max extent 8.77px vs
  half-cell 16.87px), no excessive overlap, every member individually
  clickable at its exact drawn centre.
- Different-player stacks (2/3/4 colours on one square): geometry exact;
  during the turn owner's `awaiting`, clicks at an opponent's centre
  return the LEGAL token sharing the square — documented Phase 3B design
  (lock-out prevention), not a bug.
- Own-turn selectability matrix: all four colours' stacked pairs pickable
  on their own turns (8/8).
- Maximum stacking: 16 tokens on ONE shared cell (4×4) — outsideCell=0,
  red members 4/4 clickable at own centres; minSep 3.21px (tight but
  distinct; radius schedule asserted by phase3b).
- Dense ring regime n=5..8: redMembersClickableAtOwnCentre 4/4 for every
  n; minSep 6.2px (n=5,6,8), 3.21px (n=7, ring/inners tangent case).
- Home-lane stacking: private-keyed (`L{p}:{pos}`) — 2/4 same-player on
  one lane cell fanned correctly (sep 10.12px, in-cell, pickable);
  walker joining a resting own token forms a clean 2-stack (landed 52,
  picks 0#0/0#1); 4-on-one-lane-cell also clean.
- Animation consistency: walker gliding into an occupied square is
  excluded from the stack map (full size, no fan) while the resting
  token's seat is IDENTICAL across all 29 rAF samples; on commit the
  capture fires (engine-verified: green rel 10 → -1) and red rests at
  the exact cell centre, hit-testable.
- Yard layout intact (16/16 slots pickable); finish pool with 16
  finished tokens: all inside the centre pool, individually keyed.
- Restart from a 16-stack: all tokens back to yard, epoch bumped, idle.
- Resize/fullscreen with full stacks: board 323→552→506px, outsideCell=0
  at every size, separations scale proportionally.

Probe-script notes (not product bugs): green staged at rel 10 (not 23)
for the capture test — rel 23 for green is global 36; and mid-glide
sampling must run inside rAF (polling from Playwright misses frames).

Regressions: glide 11/11 · phase3b 7/7 · verify_ludo 40/41 (same
pre-existing `restart/freshState`) · phase3c 4/5→5/5 flake (test's own
dice race on first roll = 6, documented in Task 3.3) · phase3d 8/12
(same pre-existing staging failures) · ludo_ui 4/4.


### Task 3.5 — Dice animation & state synchronization (verified, no changes)
Audited rendering (`luDieFace`/`luSetDice`/`luDieShow`), the roll handler
(`luDoRoll`), engine RNG (`LudoEngine.rollDice` — untouched fair d6),
the tumble rAF chain (`luDiceSpin`/`luCancelDiceSpin`, owned by
`LUG.diceRaf`), turn transitions (`luStartTurn`/`luEndRoll`/
`luStartTurnKeepPlayer`), disabled states (`luRenderControls`) and rapid
clicks. **No defect found; no code changed.**

Verified (all PASS):
- All six faces: forced 1–6 → displayed face == `LUG.dice` == forced
  value, 6/6 exact.
- Natural roll: `.rolling` class during tumble, 29 tumble frames ≈ 483ms
  (target 480ms), 4+ distinct random faces mid-tumble, final face ==
  engine value at resolve (owner-attributed trace).
- Repeated consecutive rolls: 5 rounds natural values, face == engine
  every round.
- Rapid clicks: 6 clicks in ~350ms → exactly ONE human roll (button
  disables on first click + `phase !== 'idle'` guard).
- Six-chain via real flow (stubbed Math.random→6): release → extra roll,
  exactly 3 consecutive sixes then turn passes (engine cap honored).
- No-move settle (real flow): phase stays `rolling` for the 850ms event
  window, button disabled — no re-roll possible; die already stopped on
  the engine value.
- Restart mid-tumble: `.rolling` cleared, `diceRaf=0`, dice null, all
  tokens yard, zero rolls, clock bumped — and the old 480ms timer fires
  into a dead clock (no-op verified).
- Route-away mid-tumble: teardown nulls game, kills rAF; re-enter gives
  a fresh playable game immediately.
- Invalid attempts: roll while `awaiting` (button disabled + guard
  rejects), `force()` while `rolling`/`moving` rejected, roll during AI
  turn impossible (button disabled).
- Fullscreen: tumble visible, resolves to engine value; resize mid-roll:
  board 323→602px, face still == engine, no stray repaint after resolve
  (61 post-resolve frames, single stable face).

Probe-script lessons (not product bugs): wrap `LUD.onRoll` ONCE and reuse
the stored original (re-wrapping double-records); attribute each roll to
its owner (AI turns legitimately follow); read `LUG.dice` at resolve time
(it is correctly nulled when the turn passes); `LUD.force()` skips the
tumble by design (test hook, guards intact).

Regressions: glide 11/11 · phase3b 7/7 · verify_ludo 40/41 (pre-existing
`restart/freshState`) · phase3c 5/5 · phase3d 8/12 (pre-existing) ·
ludo_ui 4/4. Identical to baseline.


### Task 3.6 — LUD.force protection (hardened)
File: `crates/loop-dashboard/src/assets/ludo-game.js` (only).

Three genuine protection gaps reproduced before fixing:

1. **No value validation** — `force(0/7/-1/null/'x'/3.5/NaN/undefined/true)`
   were all ACCEPTED. Worst case proven: `force(0)` with red on rel 5 and
   green sharing global square 5 produced a legal-looking move `{from:5,
   to:5}`; executing it through the real pipeline left red at 5 but
   CAPTURED green (victimsAt fires at the destination) — a gameplay-rule
   violation reachable from the debug hook. Also `LUG.dice='x'` garbage
   state with the die silently painting 6.
2. **AI-turn bypass** — during Green's (AI) turn, `force(6)` resolved a
   roll FOR the AI: no owner check existed.
3. **Settle-window interleave** — the no-move resolve path leaves
   `phase='idle'` during its 850ms hand-off window, so rapid `force(4)×5`
   injected 5 rolls (each consumed), each scheduling overlapping settle
   timers.

Fixes (all inside `force()` + a tiny self-expiring guard):
- `Number.isInteger(v) && 1..6` gate — engine never sees a forged value.
- `g.players[g.turn].kind !== 'human'` → reject (never act for an AI).
- New `LUG.settleUntil` deadline (ms): set by the no-move resolve,
  cleared by luEndRoll/luStartTurn/luNewGame/luTeardown; force() refuses
  while the window is genuinely open. Self-expiring by design — an
  externally killed timer (test clock bump) can never leave a stale
  block behind (first attempt used a sticky boolean; that hazard was
  caught during verification and reworked).

All ten required protections verified PASS: valid 1–6 accepted;
invalid inputs rejected with zero state mutation; rolling/moving/
awaiting/AI-turn/game-over all rejected unchanged; rapid force consumed
exactly once (`rolls=1`, `dice=4`); force→restart kills the pending
settle (rolls 0, all yard, stale timer no-ops); teardown mid-settle
clean, re-entry fresh; natural rolls after forcing still 3/3 engine
values (RNG authority intact).

Regression: glide 11/11 · phase3b 7/7 · verify_ludo 40/41 (pre-existing
`restart/freshState`) · phase3c 5/5 · phase3d 8/12 — the SAME four
staging failures as the documented baseline. Root cause of those four
(now fully diagnosed, pre-existing test-design flaw, NOT force()):
after any `force(6)` that yields moves the game sits in `awaiting`
until a token is clicked; the script's `wait_idle_human` can then never
succeed, so sections 3–5 report "could not stage awaiting". Verified
unchanged by this task's fix. ludo_ui 4/4.


### Task 3.7 — Illegal-move feedback (reasons refined)
File: `crates/loop-dashboard/src/assets/ludo-game.js` (luIllegalPick only).

Audited the full feedback matrix first. Already correct (no changes):
finished/opponent/non-token reasons, nudge flash + auto-fade (700ms flash,
message restored at 1400ms), zero state mutation on illegal clicks, no-legal-
move turn resolution, feedback cleared by turn change / restart / teardown
(including stale-timer resurrection), rapid-click stability, AI turns ignore
human clicks entirely.

Two genuine reason-accuracy gaps found and fixed:
1. **Yard token clicked with dice ≠ 6** said "That token can't move 3
   squares" — misleading; the honest reason is the release rule. Now:
   "You need a **six** to release a token from the yard." (dice=6 yard
   case is engine-unreachable for legality, kept an accurate fallback.)
2. **Overshoot** (pos+dice > FINISH) also got the generic message. Now:
   "Home needs an **exact roll** — that token is 2 steps away."

Verified after fix (all PASS): yard reason shown for all four colors as
turn owner; overshoot reason correct (rel 54 + dice 3 → "2 steps away");
finished/opponent reasons unchanged; state never mutates (phase/dice/
positions snapshot-identical); nudge fades and the standard prompt returns;
restart mid-feedback clears nudge + message with no timer resurrection;
teardown mid-feedback clean; clicks during AI turns produce no human-style
feedback; multi-legal selection picks the exact clicked token (token1
10→23); legal-vs-non-legal visual distinction measured perceptually:
highlight annulus luminance 86.2 vs 32.1 (**2.69× brighter**) around legal
tokens — clear without flashing (static dashed ring + soft mint fill).

Probe lessons: single-radius ring sampling misses the [4,3] dash pattern —
measure annulus luminance instead; staging must set the turn OWNER's kind
to human when testing per-color feedback (Task 3.6 correctly rejects force
on AI turns).

Regressions: glide 11/11 · phase3b 7/7 · verify_ludo 40/41 (pre-existing
`restart/freshState`) · phase3c 5/5 · phase3d 8/12 (same four pre-existing
staging failures; message assertions unaffected — "home"/"Green"/"token"
substrings all still present) · ludo_ui 4/4. Identical to baseline.


### Task 3.8 — Final regression & stability audit (Phase 3 complete)

**Part 1 — baseline failures resolved (both were TEST defects; zero
production changes):**

1. `verify_ludo.py` `restart/freshState`: asserted the VIEW epoch bumps on
   restart, but production deliberately keeps the epoch stable so board/
   control listeners survive restarts (documented since Part D); stale
   callbacks are invalidated by the GAME CLOCK bump instead (verified:
   clock 2→4 across a restart while epoch stayed 2, and post-restart
   clicks still work). Fix: assert the real mechanism — fresh state
   (rolls 0 / turn 0 / winner null) AND clock strictly greater — plus a
   NEW check `restart/epochStableForListeners` proving the epoch was NOT
   bumped. Net: assertion strengthened, not weakened. 40/41 → **42/42**.

2. `verify_ludo_phase3d.py` four "could not stage awaiting": once any
   section left the game in `awaiting`, nothing ever picked a token, so
   `wait_idle_human` could never return (root cause diagnosed in 3.6).
   Fix: `wait_idle_human` now consumes a blocking human-awaiting state by
   clicking a legal token — exactly what a player does; staging then
   succeeds deterministically. Second latent test bug found while fixing:
   sections 1c/5 computed click points with naive cell-centre math, which
   misses STACKED tokens (fan-out offsets); with staging now working,
   stacks actually occur. Switched both to the stack-aware
   `__luTokenPoint` helper already used elsewhere. 8/12 → **10/10**.

**Part 2 — gameplay regression:**
- Engine-level headless simulation through the real LudoEngine: 150 full
  games across all valid seat configurations (2h+2AI, 3h+1AI, 4-human
  hotseat, 1h+3AI, AI-only). All completed; every die face 1–6 exercised;
  releases/captures/safe-landings/stacking/lane-moves/exact-finishes/
  extra-rolls/three-six-forfeits/no-move turns all observed; zero
  out-of-bounds positions; zero home-count mismatches.
- UI-level rule spot checks (all PASS): capture sends victim home; star
  square blocks capture; exact finish increments home; six grants extra
  roll; third consecutive six forfeits the turn; no-legal-move message +
  turn hand-off.
- Mode inventory finding: the engine is strictly 4-seat (createGame
  validates 4 kinds of 'human'|'ai'); there is no 2/3-player table or
  difficulty selector in the product — one AI personality exists
  (scored greedy + jitter). Building either would be a new feature
  (out of scope for 3.8), so verified via engine API instead.
- Restart during dice animation / mid-glide / visible feedback: all clean
  (fresh state, no rAF/timer/nudge residue, no _viz leftovers).
- Back-nav teardown + re-entry: clean fresh game. Resize 800×640→1500×1000:
  canvas rescales, state preserved. Turn isolation: AI never acts on human
  turns and vice versa (sampled through natural mixed play).

**Part 3 — lifecycle & stability (all PASS):**
- No duplicate listeners: resize handler fires exactly once per event
  across 6 mount/unmount cycles.
- No stale timers/rAFs after churn (5 play/restart cycles): diceRaf 0,
  anim null, nudge null, settle window closed.
- No overlapping dice rolls under 5× button hammering (phase guard holds).
- No stuck phases over 12s continuous natural play.
- No console/page errors in any scenario (probe artifacts excluded).
- Other content intact: funtime cards (10), snakes canvas renders,
  dashboard renders. Full crate lib suite 339/339 (one transient
  environmental flake: sysmon::health samples live kernel memory pressure
  created by parallel test load itself; passes in isolation and on rerun —
  unrelated to Ludo).

**Final Phase 3 suite:** glide 11/11 · verify_ludo 42/42 · phase3b 7/7 ·
phase3c 5/5 · phase3d 10/10 · ludo_ui 4/4 · lib 339/339. **All green.**


## Phase 4

### Task 4.1 — Dynamic player-count configuration (2 / 3 / 4 players)

**Four-seat assumptions found (engine + game):**
- Engine `PLAYERS=4` is *physical* board geometry (corners, TRACK, START_G,
  LANES, YARD_SLOTS) — deliberately kept at 4; it is not "active players".
- `createGame(kinds)` required exactly 4 kinds with no inactive notion;
  `advanceTurn()` rotated `(turn+1)%4` through every seat; `checkWinner()`
  scanned all 4 seats; UI hardcoded `['human','ai','ai','ai']` in two places
  (boot + restart), HUD showed a flat 4-wide home count, blurb said "vs three
  AI", and every render loop iterated `p<4` (fine for geometry, but needed
  activity awareness for tinting).
- `victimsAt()` already excluded yard tokens (`pos>=0` guard), so inactive
  seats can never be capture victims — no change needed there.

**Seat-selection policy (deterministic, documented in both files):**
- 4P → seats [0,1,2,3] Red/Green/Yellow/Blue (unchanged default)
- 3P → seats [0,1,2] Red/Green/Yellow (first three corners clockwise)
- 2P → seats [0,2] Red/Yellow — diagonal corners, the standard two-seat
  Ludo arrangement (opposite starts, maximum racing distance)
- Seat 0 is always active and always the human; other active seats are AI
  this phase. Invalid requests (no seat 0, out of range, dupes) fall back
  to the classic 4-seat game.

**Engine changes (ludo-engine.js):** `createGame(kinds, {seats})` stores
`players[i].active` + `game.seats`; new `isActive()`/`activeSeats()` helpers
exported; `advanceTurn()` rotates among ACTIVE seats only (identical to the
old modulo rotation when all four are active); `checkWinner()` skips
inactive seats. Legacy `createGame(kinds)` calls behave exactly as before.

**Game changes (ludo-game.js):** `LUG.seatCount` (2|3|4) survives restarts;
`LU_SEATS_FOR` policy map + `luKindsForCount()`; boot & restart build games
from the setting; a compact `PLAYERS 2P|3P|4P` radiogroup in the controls
row (selected state styled like the rest of the chrome, aria-checked kept in
sync, colour-dot legend in the HUD shows which seats participate — inactive
seats dimmed); board renders inactive yards/lanes/pool wedges/tokens faded;
intro message + panel blurb name the participating colours; selecting a
count applies by starting a fresh game (clicking the already-selected count
is a no-op — never restarts mid-game); `luStartTurn` carries a defence-in-
depth guard that hands off if a turn ever lands on an inactive seat.

**Tests:** new `scripts/verify_ludo_phase4a.py` — 26 checks: engine seat
semantics per config, invalid-request fallback, legacy compat, rotation
sequences, winner-active-only, selector UI state, live 2P/3P rotation through
real play, restart preservation, same-count no-restart, 2P capture +
safe-square + winner, back-nav re-entry, churn with stale-timer/rAF sweep,
single resize listener, no console errors. **26/26 passed.**

**Full regression:** glide 11/11 · verify_ludo 42/42 · phase3b 7/7 ·
phase3c 5/5 · phase3d 10/10 · phase4a 26/26 · ludo_ui 4/4 · lib 339/339.
All green — existing four-player behaviour unchanged (legacy createGame
path asserted byte-for-byte in the new suite).

**Remaining limitations (by design, later phases):** no AI seat
configuration yet (every non-human active seat is AI); no difficulty tiers;
player count is per-session (not persisted across page reloads); selecting
a count mid-game restarts the game (the only safe apply point).


### Task 4.2 — Human + AI seat configuration

**Configuration model:** `LUG.humanSeats` (1..seatCount) joins Phase 4.1's
`LUG.seatCount`. New `luKindsForConfig(n, humans)` builds the kinds array;
the legacy `luKindsForCount(n)` is kept as a thin wrapper (one human).
Invalid values are rejected safely by clamping into [1, active-count].

**Assignment policy (deterministic, documented in code):** the FIRST
`humans` seats in the ACTIVE-seat order are human; remaining active seats
are AI; inactive seats stay 'ai' and never act. Seat 0 is always first in
the active order → always human. Examples: 3P·2H → humans {Red, Green},
AI {Yellow}; 2P·2H → both Red and Yellow human; 4P·1H → Red human,
Green/Yellow/Blue AI.

**Turn handling:** zero turn-flow changes were needed — every path already
branches on `g.players[turn].kind` (roll-button enablement, AI roll/pick
timers, board-pick guard, keyboard-space guard), so multi-human hotseat and
mixed games work through the existing single-controller pipeline. Verified:
manual rolls only on human turns, automatic AI rolls, input ignored during
AI turns (10 synchronous force-enabled clicks during a frozen AI turn →
zero roll starts), consecutive AI turns (human→ai→ai→ai observed),
no duplicate AI timers.

**UI:** a compact `HUMANS 1H|2H|3H|4H` radiogroup beside the player-count
selector; options beyond the current player count hide+disable; the choice
clamps when the player count shrinks; selecting applies via fresh game and
clicking the current selection is a no-op (never restarts mid-game). The
panel blurb now names every seat's controller ("Red (human), … Blue (AI)");
the HUD chip distinguishes You / Human / AI for hotseat clarity; roll and
turn messages address non-zero human seats by colour.

**Tests:** new `scripts/verify_ludo_phase4b.py` — 31 checks: all 9 valid
combinations' kind assignment, inactive-never-human, clamp-low/high/NaN,
per-count selector availability + hidden options, blurb controllers,
all-human no-auto-roll, human→human handoff, human→AI transition,
input-ignored-during-AI (deterministic synchronous-click probe),
consecutive AI sequence, no duplicate AI rolls (instrumented), restart
preservation (3P2H), back-nav re-entry, rapid configuration churn with
timer/rAF/listener cleanup, no console errors. **31/31 passed.**

**Full regression:** glide 11/11 · verify_ludo 42/42 · phase3b 7/7 ·
phase3c 5/5 · phase3d 10/10 · phase4a 26/26 · phase4b 31/31 · ludo_ui 4/4 ·
lib 339/339. All green; Phase 4.1 seat configurations and all Phase 3
fixes preserved.

**Remaining limitations (by design):** no AI difficulty tiers yet; the
configuration is per-session (not persisted across reloads); changing either
selector mid-game restarts the game (the only safe apply point).


### Task 4.3 — Easy AI difficulty

**Configuration model:** `LUG.aiDifficulty` — `'easy'` or `null`
(null = Classic, the unchanged legacy scorer). Medium (Phase 4.4) and
Hard (Phase 4.5) are reserved placeholders in the UI only: disabled,
dashed "coming soon" buttons; the selector rejects any unimplemented
value, and the engine falls back to the legacy scorer for them rather
than pretending they exist.

**Engine additions (`ludo-engine.js`):**
- `selectAIMoveEasy(moves, rand)` — beginner strategy: picks only from
  engine-provided legal moves; prefers yard release (`from === -1`,
  only offerable on a six); otherwise uniform random among the remaining
  legal moves via injectable `rand`. No capture math, no defensive or
  offensive planning. Empty move list → `null`.
- `selectAIMoveForDifficulty(state, player, moves, difficulty, rand)` —
  routes 'easy' to the above; anything else to the legacy
  `selectAIMove` (default behaviour bit-for-bit unchanged).
- `makeRng(seed)` — mulberry32 seeded PRNG so tests are deterministic
  without touching `Math.random`.

**Game wiring (`ludo-game.js`):** the AI pick timer now calls the
difficulty-aware selector with `LUG.aiDifficulty` and a per-game
`LUG.aiRng` (freshly seeded on every new game / restart / boot). Human
turns, dice animation, movement interpolation and all Phase 4.1/4.2
configuration are untouched.

**UI:** compact `AI LEVEL Classic | Easy | Medium | Hard` radiogroup
under the humans selector. Classic is the default; Easy selectable;
Medium/Hard visibly disabled. Selection applies via fresh game and
re-clicking the current choice never restarts mid-game. The panel blurb
appends "· AI level: Easy" when active. Human-only games are unaffected
(no AI seats exist; verified no auto-rolls).

**Tests:** new `scripts/verify_ludo_phase4c.py` — 33 checks: engine
strategy (six offers release; mixed six always releases; same seed →
same pick; hashed-seed spread across both legal moves; every pick within
the legal set; empty moves → null; null difficulty keeps legacy path),
UI states (Classic default, Easy available, Medium/Hard disabled+soon,
existing selectors preserved, disabled click ignored), live flow (Easy
picks only legal moves with difficulty flowing through; human turns stay
manual; no-legal-move AI turn passes; extra roll on six; three-sixes
chain cap via the engine contract r1/r2 true, r3 false; multiple AI
seats picking — pre-staged tokens remove dice luck), restart & back-nav
preservation, switch back to Classic, human-only unaffected, no
duplicate AI timers/rolls, no console errors. **33/33 passed.**

**Full regression:** glide 11/11 · verify_ludo 42/42 · phase3b 7/7 ·
phase3c 5/5 · phase3d 10/10 · phase4a 26/26 · phase4b 31/31 · phase4c
33/33 · ludo_ui 4/4 · lib 339/339. Note: two pre-existing
`sysmon::health` live-sample tests (`fresh_real_sample_is_healthy…`,
`low_discharging_battery_warns…`) intermittently fail when this Mac's
live kernel memory pressure is at warning level — environmental flake in
this branch's earlier sysmon work, unrelated to Ludo (no Rust files were
touched in 4.3; the dedicated `memory_pressure_warning_degrades` unit
test passes).

**Remaining limitations (by design):** Medium and Hard are not
implemented (reserved for 4.4/4.5); Easy's randomness is seeded per game
so a re-seeded identical game replays identically, but the seed itself
is time-derived and not user-visible; difficulty applies to ALL AI seats
equally (no per-seat difficulty yet).


### Task 4.4 — Medium AI difficulty

**Strategy (`ludo-engine.js`):** `selectAIMoveMedium(state, player, moves,
rand, record)` scores every engine-legal move with a transparent weighted
model (`MEDIUM_WEIGHTS`, exported for tests):

| weight | value | meaning |
|---|---|---|
| finish | 800 | bring a token home outright |
| capture | 220 | per victim captured |
| captureStep | 2 | bonus per step the victim had travelled |
| release | 200 | yard → track on a six |
| safeLanding | 120 | land on own star square |
| homeStretch | 40 | enter the private lane |
| progress | 3 | per dice-step advanced |
| depth | 1 | absolute position — keep leaders running |
| exposure | −90 | leave a protected square for open track |
| dangerAt | −60 | land where a real enemy sits 1..6 behind |
| diversity | −35 | re-move the token moved last turn |
| jitter | 12 | controlled randomness among near-equal scores |

Non-greedy by construction: jitter 12 dominates whenever the score gap is
smaller (e.g. two plain advances one step apart split ~50/50 across
seeds), while clear strategic gaps (capture vs plain move ≈ +230) stay
decisive. Threat detection uses only real positions and the genuine 1..6
dice window — no invented probabilities.

**Statefulness:** the diversity term reads a per-state WeakMap memo of the
player's last moved token, recorded when `record !== false`. Pure-selection
tests pass `record=false` so repeated calls are idempotent; the live router
records normally.

**Routing:** `selectAIMoveForDifficulty` — Classic → legacy scorer
(unchanged); Easy → Phase 4.3 strategy (unchanged); Medium → new strategy;
Hard → still falls back to legacy (reserved for Phase 4.5).

**UI:** Medium enabled in the AI LEVEL radiogroup; Hard remains disabled
with its "Coming soon — Phase 4.5" title. No other setup-UI change.

**Tests:** new `scripts/verify_ludo_phase4d.py` — 32 checks: capture
preferred (20→25 over 10→15 with a staged victim), safe square (12→13 star
over 45→46), home progress (44→46 beats 30→32 on memo-free raw scores 52
vs 38), yard release on six, exposure avoidance (22→26 star-path over
13→17 into a staged threat), multiple legal moves, fixed-seed determinism
(record=false, 25/25), controlled near-equal randomness (30/30 split),
all-picks-legal over 120 randomized boards, empty list → null, Classic and
Easy unchanged through the router, strategic diversity via memo, weights
exactly as documented, UI states, live Medium flow (picks only legal,
difficulty flows through), human turns manual, multiple AI seats, restart
and back-nav preservation, Easy↔Medium switching, human-only isolation,
timer hygiene, no console errors. **32/32 passed (×3 consecutive runs).**

**Suite robustness work (both 4c & 4d):** the live-flow driver now (a)
waits for a clean human-idle state before staging, (b) commits red's move
through the real `luPick()` path instead of mouse coordinates, (c) stages
AI-seat tokens so picks are guaranteed rather than dice-luck dependent,
and (d) selector clicks use JS `element.click()` to avoid layout-shift
races. Phase 4.3's suite was updated for the Phase 4.4 contract: Medium is
now asserted ENABLED (was disabled), and the disabled-click probe targets
Hard (the still-reserved level). These are contract updates, not weakened
assertions — each check still pins exact button state.

**Full regression:** glide 11/11 · verify_ludo 42/42 · phase3b 7/7 ·
phase3c 5/5 · phase3d 10/10 · phase4a 26/26 · phase4b 31/31 · phase4c
33/33 (×4) · phase4d 32/32 (×3) · ludo_ui 4/4 · lib 339/339.

**Remaining limitations (by design):** Hard is not implemented (Phase
4.5); Medium evaluates single-ply (no lookahead beyond the immediate
move's consequences); the diversity memo keys off the live state object so
cross-restart history resets naturally.


Note on Part A: inspection showed the baseline already implements the full rAF
interpolation (`luGlideStep` + `_viz {from,to,t}` + `luCancelAnim` + clock
guards). Part B kept it intact and extended it through the new single-source
geometry helpers; verified via the existing human-move and restart tests.

