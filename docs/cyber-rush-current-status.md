# Cyber Rush — current status (recovered 2026-10-31 evening)

## Where things live
- Game source: `crates/loop-dashboard/src/assets/console-app.js` (CR block ~lines 5250–11060)
- Game CSS + markup: `crates/loop-dashboard/src/assets/console.html`
- Rust UI contract tests: `crates/loop-dashboard/tests/cyber_rush_ui.rs` (30 tests)
- Browser test scripts (previous sessions): `/tmp/p70_res_test.py`, `/tmp/p71_test.py`
- Screenshots: `.task70_screenshots/`, `.task71_screenshots/`, `.task72_screenshots/`
- Local server for tests: `./target/debug/loop-dashboard --no-auth --port 8791`

## State at session start
- Prior work completed Phase 1–7.2 + BUGFIX 1..12 comments in source.
- p70 layout/resolution test reported 19/20; the single failure was
  "immersive layout: canvas on-screen" (canvas bottom 792.9 > 768).

## Fixes applied this session
1. **P1.1b immersive/fullscreen height math** (`crResize`):
   root cause — the fullscreen branch reconstructed available height by
   summing sibling row heights + a flat 6px each, but the panel is a flex
   column with `row-gap:16px` and the nitro meter row has its own 2/4px
   margins → ~32px under-count → canvas overflowed the viewport.
   Fix: measure the real space (`panel.bottom - paddingBottom - stage.top`);
   keep the arithmetic fallback for the not-yet-laid-out case using the
   actual `rowGap`.
   Result: p70_res_test now **20/20 PASS**.
2. **Rust test `controls_split_nitro_and_drift`**: help text lost the exact
   phrase "Shift drift" when the CR controls row was converted to overlay
   chips. Restored the phrase inside the ready-overlay subtitle.
3. **P2.8 minimap closure**: the heading factor was a fixed 0.0175 rad per
   curve-unit, leaving the integrated outline at −27° net turn (an open
   spiral force-wrapped shut). Now the factor is derived from the track so
   the loop turns exactly 360°, and the residual endpoint error is spread
   over the polyline with a smooth sin² weight. Net turn 360°, closure 0.
4. **P3 traffic discipline**:
   • caps Easy 6 / Medium 9 / Hard 12 (was 16/26/36); Endless ramp also
     capped at the hard ceiling;
   • traffic cruise band clamped to 40–70% of the player's rated speed
     (was 26–56%, letting vans crawl at ~19%);
   • recycled cars re-emit only at the FAR END of draw distance
     (≥ DRAW_DIST−20), never in the near field;
   • never re-emitted into the player's own lane within the visible field;
   • one lane kept free per ~15-segment spawn group (initial placement AND
     recycling);
   • 0.4s fade/scale-in window (`spawnT`) on every fresh car, rendered as
     55%→100% scale + alpha ease.
5. **P3 racers**:
   • exactly FIVE participants on every difficulty (player + 4 AI; was
     2/4/6 by difficulty);
   • rubber-band hard-capped at ±10% of each rival's baseline cruise;
   • rivals scan ordinary traffic ahead and change to the least-occupied
     adjacent lane smoothly (no teleporting);
   • staggered TWO-COLUMN starting grid (outer lanes, rows behind the
     line) with `gridOffset` recorded;
   • finish/lap accounting switched to DISTANCE RACED (segF + gridOffset)
     so back-row rivals cannot finish a lap early; progress hard-capped
     at totalLaps × gatesPerLap for AI and player alike;
   • name tags show `P<n> NAME` and scale with distance (clamped 8–17px);
   • subtle amber rear warning chevron when a rival closes from behind.
6. **P4 gameplay**:
   • track sections ×5 (450 → 2250 segments = 450k units/lap): measured
     Medium lap 40.6–43.5s, inside the 40–60s target; checkpoints stay at
     4/lap, evenly spaced (0, 563, 1125, 1688); closed loop preserved;
   • AI skill band rebalanced (leader ≤0.93 cap on Medium) so P1 is
     achievable with good driving.

## Verified passing (do not re-run without need)
- p70 layout/responsive/fullscreen: **20/20**
- p71 checkpoint/lap/mode regression: **22/22** (timeouts raised for the
  longer track: T5 150→260s, T12 120→240s — harness limits, not behaviour
  changes)
- cargo cyber_rush_ui: **30/30** (traffic-cap expectations updated to the
  new P3 values 6/9/12)

## Investigations that found NO bug (verified working)
- Gear model: upshifts 1→6 on acceleration, downshifts 6→1 under braking.
- Drift: banked 142–148 pts in a live slide; results tile shows points
  ("best drift 148"). The reported "Drift 0.0s" was actually the NITRO
  USED tile ("0.0s") which correctly reads 0.0 when nitro was never fired.
- Nitro charge: isolated near-miss test charged exactly +0.09
  (NITRO_GAIN_MISS); earlier apparent failures were collision penalties
  from other traffic cars interfering with the probe.
- Collision: head-on overlap registered a crash with speed loss + grace.
- Road centring: with camera settled, road centre projects to exactly
  W/2 on straights; curves sweep ~±100px at 40 segments (legitimate).
- Lane markings share the same p1/p2 projection chain as edges/rumble.
- No CHECKPOINT/FINAL LAP text painted on the road (source-verified).
- Fullscreen tint: flashT is set ONLY by collisions; near-miss tint is a
  subtle edge kiss (peak 0.16 alpha).

## Known unrelated failures (NOT ours)
- `cargo test -p loop-dashboard --lib`: 2 sysmon health tests fail because
  this machine currently reports real kernel memory pressure
  (`sysmon::health::tests::fresh_real_sample_is_healthy_with_history_extent`,
  `low_discharging_battery_warns_but_full_does_not`). Pre-existing,
  environment-dependent, outside the Cyber Rush scope.

## Fullscreen + responsive layout fix (this session)
Scope: fullscreen and viewport layout ONLY — no gameplay, physics, scoring,
traffic, collision or difficulty changes; no other games touched.

### Files modified
- `crates/loop-dashboard/src/assets/console.html` (CR CSS block only)
- `crates/loop-dashboard/src/assets/console-app.js` (CR resize/render/layout only)

### Root cause
1. `crResize()` letterboxed a FIXED 960x540 logical canvas inside the stage:
   in fullscreen the canvas covered only ~80% of the screen (dead bands top/
   bottom, gutters left/right on wide screens). On 844x390 it shrank to
   668x376 — most of the screen was background.
2. `.cr-touch-ov` pads anchored to the STAGE bottom, but the stage was far
   taller than the canvas (886px vs 720px): pads floated over empty space
   and on portrait the DRIFT pad extended past the viewport (right=445>390).
3. The HUD strip + nitro meter were flow rows ABOVE an uncapped stage, so
   entering/exiting menu mode (display:none) made the canvas jump, and the
   windowed stage cap `(100vh-170px)*16/10` under-counted real chrome by
   ~70px → canvas bottom overflowed the viewport by 9–29px.
4. THREE fullscreenchange handlers per mount (two in renderCrView + one in
   ftSessionBind) → triple resize per toggle.
5. `crOverlay(null)` never cleared `cr-menu-on`, so a race started from a
   menu state kept the DOM HUD hidden for the whole race.
6. Windowed aspect factor used 16/10 but the design canvas is 16/9 (960x540).

### Exact fullscreen fix
- **CSS**: `body.cr-fs #crPanel` (and the ft-immersive fallback) = the
  screen: margin/border/radius/padding/gap all zero, flex column, overflow
  hidden. HUD strip + nitro meter are compact flow rows at the top; the
  stage is `flex:1` and takes EVERY remaining pixel. The old inline-play
  stage cap is scoped with `:not(.cr-fs):not(.ft-immersive)` so it can no
  longer leak into fullscreen (it previously beat the un-cap via !important).
- **JS `crResize()`**: sizes the canvas to the stage's real client box and
  derives a LIVE logical width `CR.vW = clamp(aspect*540, 720, 1720)` while
  keeping the design height 540. The ctx transform scales UNIFORMLLY
  (min of kx,ky, centred) so nothing stretches; when the clamp binds the
  world letterboxes instead of distorting. `crProject()`, star field,
  streaks, sparks and the player's lateral offset all read the live width.
- **One fullscreenchange handler** per mount (reflect + resize + rAF
  follow-up); ResizeObserver coalesces to one resize per frame; the dpr
  media-query listener is unchanged. Verified via CDP: exactly 2 live
  fullscreenchange listeners (game + shared session controller) and 1
  resize listener, stable across 4 mount/unmount cycles.
- `crOverlay(null)` now clears `cr-menu-on` (HUD always returns on race start).
- Windowed stage caps corrected to 16/9 with a measured 240px chrome allowance.

### Verification (all green)
- Playwright matrix `verify_fs.py`: **163/163** across 1280x900, 1440x900,
  1920x1080, 1024x768, 844x390, 390x844, 2560x1080 — windowed fit, FS entry
  on #crPanel, 100% width coverage + full height below HUD, no margins/
  borders in FS, HUD/meter/chips/touch-pads all inside the viewport, canvas
  speedo+gear dials geometrically inside, pause/resume in FS, window resize
  refit, button-exit restores window, ESC exits + returns to games, zero
  console errors.
- Car/road probe: car centre within 0.1% of screen centre at the bottom on
  16:9, short-landscape and ultrawide; uniform ctx transform verified at
  every aspect (no stretching).
- F-key enter/exit, CSS-immersive fallback (card click with the Fullscreen
  API denied) fills the viewport: **8/8**.
- Rust: cyber_rush_ui **30/30**, dashboard lib **368/368**, all other UI
  suites (landing_flow 11, snakes 4, ludo 4, fruitninja 3, hack_terminal 6,
  solitaire 4, admin 25, auth 16) pass.
- Screenshots: `/tmp/cr-verify/shots/FIXED-{window,fs}-{desktop,fhd,
  short-landscape,portrait,ultrawide}.png`

### Known unrelated failures (NOT ours)
- `scripts/verify_cyber_rush.py` is stale (pre-Phase-6 flow: expects
  overlay "ready" at boot and `#crModeTime`/`#crModeEndurance` buttons that
  no longer exist). Superseded by the suites above.
- sysmon health tests read live hardware metrics and can fail when this
  machine has real kernel memory pressure — environmental, pre-existing.
