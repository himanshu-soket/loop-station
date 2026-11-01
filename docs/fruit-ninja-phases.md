# Fruit Ninja — improvement phases

**Scope lock:** all work is limited to Fruit Ninja only
(`crates/loop-dashboard/src/assets/fruitninja-game.js`, namespace `FN` / `fn*` /
`.fn-`). No other game, shared navigation, global style or dashboard component
is modified.

## Files

| File | Role |
|---|---|
| `crates/loop-dashboard/src/assets/fruitninja-game.js` | Game logic, namespaced CSS, WebAudio, teardown (only file edited) |
| `crates/loop-dashboard/src/web.rs` | Serves `/console/fruitninja-game.js` (read-only wiring) |
| `crates/loop-dashboard/src/assets/console.html` | Loads the script (read-only wiring) |
| `crates/loop-dashboard/src/assets/console-app.js` | Route `#funtime/fn`, card, shared FTFS/FTSESS controllers (read-only wiring) |
| `crates/loop-dashboard/tests/fruitninja_ui.rs` | Rust UI contract tests |
| `scripts/verify_fruitninja_phase1.py` | Playwright QA for Phase 1 |

Local server for tests: `./target/debug/loop-dashboard --no-auth --sys-demo --port 8912`
(assets are `include_str!`-embedded — rebuild after editing).

## Phase 1 — Time-based gameplay speed & difficulty balancing ✅

Implemented in `fruitninja-game.js` only:

- **New elapsed-play clock** `FNG.elapsedMs`: advances only while
  `state === 'playing'`, so pausing freezes the difficulty ramp exactly like
  the arcade countdown. Reset on every `fnStart()`.
- **`fnDiff()`** — normalised difficulty `0→1` via smoothstep easing.
  Classic ramps over 80 s (`FN_DIFF_CLASSIC_MS`), Arcade across its full
  60 s countdown (`FN_DIFF_ARCADE_MS`).
- **Bug fixed — inverted Arcade gap curve:** the old `fnWaveGapMs()` computed
  `1 - progress`, making wave 1 the *fastest* of a run (~0–260 ms gaps,
  near-instant spam) and the final seconds the slowest. Gaps now shrink
  monotonically with elapsed time, floored at 450 ms (Classic) / 380 ms
  (Arcade).
- **`fnSpawnWave()`** now derives bomb odds and wave size from `fnDiff()`
  instead of wave count; wave size grows smoothly but never exceeds the
  original per-mode maximum (Classic ≤ 4, Arcade ≤ 5).
- **`fnLaunchSpeed()`** gains a difficulty-scaled boost (up to +16 % apex)
  with the apex hard-capped at 96 % of field height so nothing launches
  off-screen.
- Behaviour note: an Arcade bomb penalty (−10 s) now reduces the countdown
  only; it no longer distorts the difficulty curve, which tracks real play
  time.
- **Untouched by design:** scoring, points per fruit, combo bonuses, lives,
  miss/bomb penalties, collision detection, blade mechanics and game-over
  logic.

### Verification

- `cargo test -p loop-dashboard --test fruitninja_ui` → 3/3 pass.
- `scripts/verify_fruitninja_phase1.py` (real Chromium) → 22/22 pass:
  clock advances/pauses/resumes, curves monotonic and clamped, gaps shrink
  in both modes, floors respected, launch speed grows with apex capped,
  wave-size bounds held, slicing still scores, misses still cost lives,
  game over + restart + teardown clean, zero console errors.
- Other games' suites unaffected: ludo 4/4, snakes 4/4, solitaire 4/4,
  hack_terminal 6/6. (One Cyber Rush test fails due to concurrent edits to
  `console-app.js` by another session — unrelated to Fruit Ninja.)

## Phase 2 — Arcade background music & sound effects 🔄

Planned: looped synthesized BGM via the existing `FNA` WebAudio context,
music/SFX toggle, ducking during game over — scoped entirely to `FNA`/`fnSfx`.

**Background music — DONE (Phase 2B).** Same file only:

- **Approach:** no local music assets exist in the repo, so a fully
  procedural arcade soundtrack was built on the existing Phase 2A
  AudioContext (no second context, no files, no network).
- **Composition:** Am–F–C–G progression at 120 BPM over an 8-bar loop —
  sub bass on chord roots, warm triangle chord pads, kick/snare/hats,
  and an A-minor-pentatonic square-wave lead restricted to bars 5–8
  with sparse syncopation (long-session friendly, never hammers).
- **Seamless looping:** look-ahead scheduler (120 ms tick, 0.35 s
  horizon) keeps the grid scheduled ahead of the audio clock; the
  step-127→0 wrap is gapless and click-free.
- **Signal chain:** voices → fade bus (`bgm.gain`) → DynamicsCompressor
  (−18 dB, 3:1) → master → destination. Stacked music + effects cannot
  clip. Every voice uses soft attack/exponential-release envelopes.
- **Lifecycle:** starts with every `fnStart()` (idempotent — one
  interval, no duplicate instances); fades out on pause and back in on
  resume (grid position preserved); stops + disconnects on game over,
  on return-to-menu, and in `fnAudioDispose()` (teardown).
- **Autoplay policy:** music can only start after the first user
  gesture because `fnBgmStart()` requires `fnAudioInit()`, which the
  existing `fnAudioUnlock()` gate already drives.
- **Volume architecture:** independent buses — `FNA.sfx.gain` (all
  existing effects route through it now; `fnSfxSetVolume()` ready) and
  `FNA.bgm` (`fnBgmSetVolume()`, default 0.34) under the shared master
  gate (`FNA.on` × `FNA.volume`). The Sound On/Off button mutes both.
- **Untouched:** all slicing/effect sounds (Phase 2C scope), scoring,
  lives, difficulty (P1), display fitting (P3A), touch input (P3B).

### Remaining

- Phase 2C: slicing/effect sound polish (separate task).

**Slicing & combo SFX — DONE (Phase 2C).** Same file only:

- **Slice sound (`fnSfxSlice()`)** — every successful fruit cut triggers
  a three-layer juicy voice: fast varied noise swish (blade), low-passed
  splat thump, and a sub "thock" tone. Pitch/length vary ±20% per cut
  so consecutive slices never sound identical; all voices < 150 ms with
  conservative levels (≤0.30).
- **Combo milestones (`fnSfxCombo(n)`)** — keyed to the existing combo
  counter at the existing call site: 3× = rising fifth (E5→B5),
  4× = rising arpeggio + sparkle top, 5+ = four-note fanfare + octave
  shimmer; each further fruit in the same swipe re-triggers only the
  top sparkle. Scoring, windows, points untouched.
- **Bus integration** — new `fnSfxVoice`/`fnSfxNoise` helpers route
  through `FNA.sfx.gain`; a bus-level DynamicsCompressor (−14 dB, 8:1,
  3 ms attack) keeps stacked bursts from getting loud or clipping.
  Master mute (`FNA.on`) and `fnSfxSetVolume()` apply automatically;
  music stays independently controlled on its own bus.
- **Node budget** — hard cap of 24 live SFX voices (`FN_SFX_MAX`), each
  self-decrementing via `onended`; rapid slicing stays responsive while
  an uncontrolled node pile can never build up. Counter reset in dispose.
- **Legacy cleanup** — superseded `case 'slice'`/`case 'combo'` removed
  from `fnSfx()`; all other effect cases unchanged. Phase 2B engine
  byte-identical (verified by needle scan).

### Remaining

- Phase 2D (future): per-bus volume UI wiring.

**Game-event SFX — DONE (Phase 2D).** Same file only:

- **Bomb explosion (`fnSfxBoom`)** — trigger: every bomb slice
  (`fnExplode`, via `fnBombHit`). Punchy low impact + noise burst;
  no pitched blade components, fully distinct from slicing.
- **Life lost (`fnSfxLifeLost`)** — trigger: Classic-mode missed fruit
  (`fnMiss`). Gentle descending G4→Eb4→Bb3 pair; Arcade keeps a soft
  chain-break blip instead of a penalty tone.
- **Game over (`fnSfxOver`)** — trigger: exactly once per transition
  (`fnGameOver`, guarded by `state='over'` set first). Four descending
  notes + final low thud.
- **New high score (`fnSfxRecord`)** — trigger: genuine record only
  (`score > prevBest`, captured before the best is updated; also fixed
  the old tie-matching `isRecord` display condition). Rising major
  arpeggio + sparkle.
- **Countdown (`fnSfxCountdown`)** — trigger: every `fnStart()`.
  3 ticks + GO cue at 0.45 s spacing, audio-clock scheduled — no
  timers, immune to restart races and render storms.
- **Menu click (`fnSfxUi`)** — trigger: mode-select buttons, pause/
  resume, Play-again/menu actions. Soft tick + airy chime. No new
  listeners — existing `fnOnLocal` handlers only.
- **Integration:** all six route through `FNA.sfx.gain` → compressor →
  master on the single AudioContext; Sound On/Off and
  `fnSfxSetVolume()` apply automatically; music bus untouched. The 2C
  helpers gained an optional audio-clock `delay` parameter (backward
  compatible) so sequences need zero setTimeout timers. Legacy unused
  `'boom'`/`'over'`/`'ui'` cases removed from `fnSfx` (`'miss'` still
  used by Arcade mode).
- Verified: syntax-parse clean (JavaScriptCore), balanced structure,
  2B engine and all prior phases needle-scanned intact.

### Remaining

- Phase 2E (future): per-bus volume UI wiring.

**Audio controls & final integration — DONE (Phase 2E).** Same file only:

- **UI:** two compact sliders added to the existing `.fn-controls` row —
  ♫ background-music volume and ✦ effects volume (0–100, styled in the
  game's own injected CSS, responsive at narrow widths, proper
  `aria-label`s). Master Sound On/Off remains the existing button.
- **Behavior:** sliders feed `fnBgmSetVolume()`/`fnSfxSetVolume()`
  (smooth `setTargetAtTime` ramps); levels persist in localStorage
  (`fnVolMusic`/`fnVolSfx`) and restore on every mount via
  `fnAudioLoadPrefs()`. Mute state visually mirrors onto both slider
  wrappers (dimmed) plus the existing label/aria/class indication.
- **Architecture preserved:** master → {bgm bus → compressor,
  sfx bus → compressor → voice budget}. Master mute silences both;
  BGM and SFX levels are fully independent. No engine rebuild —
  compressors, scheduler, budget untouched.
- **Listeners:** exactly one `input` listener per slider, registered
  via `fnOnLocal` → removed by `fnTeardown()` with all others. No
  duplicates across remounts (verified counts).
- **Lifecycle audit (each event fires exactly once):** start/mode-select
  → UI click + countdown; Play Again & Restart & R-key → countdown
  only; pause/resume → click + music duck/return; game over → ending
  phrase (+ record flourish only on genuine best); menu → click +
  music stop; remount → full dispose, prefs restored, silent menu.
- Verified: JavaScriptCore parse OK, structure balanced, all prior
  phases needle-scanned intact, gameplay values untouched.

**Phase 2 complete.**

## Phase 3 — Fullscreen behaviour & responsive canvas resizing ✅ (3A display + 3B touch)

**Display fitting — DONE (Phase 3A).**

**Touch & swipe responsiveness — DONE (Phase 3B).** Same file only:

- **Rect caching:** `fnCacheRect()` caches the canvas box; refreshed on
  every resize (3A hooks), at every swipe start, and at mount. The
  per-`pointermove` `getBoundingClientRect()` (a forced layout flush —
  the main latency source) is gone from the hot path.
- **Full-rate sampling:** `pointermove` iterates
  `e.getCoalescedEvents()` when present (sub-frame samples during fast
  flicks, so no fruit between frame samples escapes); the coalesced list
  ends with the event's own position, so each point is processed exactly
  once — no duplicates.
- **Single-pointer ownership:** the first `pointerdown` owns the blade
  until it lifts; a second finger cannot splice into the active polyline
  (which would forge phantom segments/slices). Mouse and touch identical
  via Pointer Events.
- **Zero delay:** no throttle/debounce/rAF-deferral anywhere in the
  input path; the move listener is `{passive:true}` (it never calls
  preventDefault); `lostpointercapture` joins up/cancel as an end-of-
  swipe signal (idempotent).
- **Mapping after resize:** cached origin refreshes inside `fnResize()`,
  so coordinates stay exact through inline/fullscreen transitions.
- **Untouched:** `FN_MIN_SWIPE_SPEED` gate, segment↔circle collision,
  bomb handling, combo windows, scoring, lives, difficulty clock (P1),
  audio (P2A), display fitting (P3A).

### Deferred (separate task)

- Consolidated final testing (Phase 4 matrix).

Implemented in `fruitninja-game.js` only:

- **Root cause found (3A):** the fullscreen CSS consumed
  `aspect-ratio:1000/var(--fn-wh,625)` but no JS ever wrote `--fn-wh`
  (the previous session died before that step), so fullscreen silently
  fell back to a hardcoded ratio and could clip; `fnResize()` also
  measured the *canvas itself*, which is circular — the canvas box
  derived from the very variable nothing set.
- **CSS:** removed the superseded `aspect-ratio`/`width:auto;height:auto;
  max-width/max-height` canvas rule (kept only `margin:auto` for flex
  centring). The canvas's exact box is now written inline by
  `fnResize()`.
- **`fnResize()` completed:** measures `.fn-stage-inner` (never the
  canvas), letterboxes the canvas to ≤16:10 inside it, sets the inline
  CSS box, sizes the DPR-capped backing store from that same box, and
  syncs `FNG.w/h` (CSS px) + `FNG.wh` (world height). No-op guard skips
  identical refits.
- **Refit triggers:** window `resize` (existing) + a `ResizeObserver` on
  the stage and stage-inner (entering/leaving fullscreen or immersive
  only flips body classes — no window resize event fires) + native
  `fullscreenchange`/`webkitfullscreenchange` + one rAF-deferred refit
  after mount, when the session classes have settled.
- **Teardown:** clears `_fitKey` so every remount refits from scratch;
  the observer is disconnected via the existing cleanup list.
- **Untouched:** scoring, difficulty clock (Phase 1), audio (Phase 2A),
  blade mechanics, HUD, overlay, all other games and shared dashboard
  code.

### Deferred (separate task)

- Pointer/touch responsiveness optimization (`pointermove` sample rate /
  `getCoalescedEvents`, passive-listener tuning).

## Phase 4 — Regression testing of Fruit Ninja only ⏳

Planned: full-matrix QA run (both modes, pause/restart, resize, fullscreen,
teardown, localStorage bests).
