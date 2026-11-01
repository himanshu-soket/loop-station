# Cyber Rush — Premium Visual Overhaul (Phase 1)

Agent: GLM 5.3 · Repo: `~/loop`
File: `crates/loop-dashboard/src/assets/console-app.js` (Cyber Rush section only)
Approach: preserve architecture (segment-based pseudo-3D Canvas 2D, epoch
lifecycle, crOn listeners); only rendering + effects changed. No new deps.

## Baseline (BEFORE, real Chromium, pixel-audited)

| Metric | BEFORE |
|---|---|
| Cyan neon edge coverage | 21% of scan rows (rumble bleed only) |
| Magenta neon edge coverage | 29% |
| Lane-dash visibility | ~5% (old probe) |
| Guardrail detectability | 0% (old probe) |
| Player-car colour detail | 58 quantised colours |
| City lights above horizon | 71 lit px |
| Neon sun | flat 16%-alpha disc |

## Changes

### 1. Road rendering
- **Solid neon edge lines** just inside each rumble: cyan left / magenta
  right (`CR_NEON_L/R`), drawn after asphalt so they stay crisp; two wider
  translucent quads emulate a glow on the nearest third (no shadowBlur).
- Distance-eased alpha (`edgeA`) so edges melt into haze instead of popping.
- **Wet-asphalt sheen**: faint light band across lit segments — glossiness.
- Guardrail top highlights now match the edge neon (cyan/magenta).
- Rumble strips kept as the classic OutRun pink/slate strobe.

### 2. Player car (complete redraw)
Layered vector build: double ground shadow → tyres → carbon diffuser with
fins → gradient body with paint-sheen band → fender highlight arcs → tinted
rear windshield with cyan reflection streak → GT wing with endplates +
neon-lit top edge → dual tail-light bars with bright inner cores + EV centre
strip → number plate (SYN-01) → twin exhausts with dark throats → nitro
flames. Wider stance (148px). Added speed-scaled idle vibration for life.

### 3. Opponent cars
Three distinct silhouettes rebuilt with layered shading:
- **van**: roof marker lamps, split rear doors, bumper, window band
- **sport**: raked cabin glass, big wing on stands with neon-lit top,
  haunch curves, diffuser fins
- **sedan**: boot-lid highlight, cabin + rear window with reflection,
  bumper bar
All get depth-based alpha fade, brighter/glowing tail lights up close, and
headlight spill onto the road ahead when near.

### 4. Environment
- Skyline towers: vertical body gradients, **neon edge lighting** (cyan
  left half / magenta right half), crown cap lights, richer window grids,
  **horizontal neon sign bands** with "text" dashes, antenna beacons.
- **OutRun sun**: gradient disc with horizontal scanline gaps that widen
  downward (clipped), plus a soft radial halo.
- Stronger horizon atmosphere band (taller, more saturated).

### 5. Effects
- **Speed streaks**: pooled screen-space particles flying outward from the
  vanishing point, length/count scale with speed; additive blending + violet
  tint during nitro.
- **Collision sparks**: pooled particles with gravity, white-hot → amber →
  ember colour ramp, additive blending; fired from `crCrash()`.
- Pools cleared on restart/teardown (no leaks between games).

### 6. Performance & lifecycle
- All new effects use fixed pools (`CR_STREAKS`, `CR_SPARKS`) — zero
  allocation in the loop.
- Gradients created per-draw only where cheap (cars/sun/skyline); road
  additions are plain quads.
- `prefers-reduced-motion` respected (streaks/vibration/sparks skipped).
- FPS measured: **58–60 sustained** at 1920×1080 fullscreen with nitro +
  steering (was 59–60 before — no regression).

### Testability
Added `window.CRD` diagnostics hook (state getters, fps ticker,
start/pause/restart) mirroring the `SND` convention used by other games.

## Verification

New suite `scripts/verify_cyber_rush.py` — **25/25 PASS**:
- functional: ready→countdown→playing, accelerate, steer, nitro, pause,
  resume, restart, sound toggle, mode switch, fullscreen, Esc exit
- visual (pixel audit while driving): cyan edge 79%, magenta edge 53%,
  lane dashes 47%, guardrail 87%, car detail 111 colours, city lights 478,
  sun 516 px, 60 fps
- responsive: 1920×1080, 1536×864, 1366×768, 390×844 — canvas in viewport,
  no horizontal scroll
- zero console errors

A/B check: disabling only the neon lines drops cyan/magenta detection to
21%/29% — the detector is honest and the effect is real.

Regression suites (shared console-app.js): snakes personalization 75/75,
UI matrix 49/49, UI gameplay 27/27, Rust snakes_ui 4/4,
embedded_dashboard 2/2 — all PASS.

Crash path verified separately: collision registers, speed sheds,
sparks render (19 white-hot px sampled at impact), no console errors.

## Screenshots
`/tmp/cr-shots/after/`: countdown, playing, nitro, paused, fullscreen,
driving-rich, 4 viewports, crash moment.
