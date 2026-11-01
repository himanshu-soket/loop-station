# Hack the Terminal — UI Root-Cause Report (measured in real Chromium)

All numbers below were measured with Playwright against the live dashboard
(`/console#/funtime/ht`) before any changes. Gameplay was driven through the
real command engine (`start`, `begin`, `hint`, answers, `pause`, …).

## 1. TERMINAL LAYOUT — dead space & no viewport intelligence

| Viewport (windowed) | panel height | terminal output height | output as % of viewport | empty space below panel |
|---|---|---|---|---|
| 1280×720  | 611px | 274px | 38% | 109px |
| 1440×900  | 683px | 342px | 38% | 217px |
| 1680×1050 | 721px | 380px | 36% | 329px |
| 1920×1080 | 721px | 380px | 35% | 359px |
| 2560×1440 | 721px | 380px | 26% | **719px** |

**Root causes**

- `body.ft-playing #htPanel { max-width: min(1100px,100%) }` hard-caps the
  panel at 1100px and `.ft-game.ht-wide { max-width:900px }` is overridden by
  it; on wide screens half the width is unused.
- `.ht-out { height: clamp(240px, 38vh, 380px) }` — the terminal body is a
  *fixed* clamp. It never reads the actual free space, so on tall viewports
  the game occupies ~35% of the screen while ~40% of the viewport is blank.
- The panel is `display:flex; flex-direction:column` but nothing distributes
  the leftover height to the terminal (`.ht-term` is `display:block`,
  `flex:0 1 auto`), so all surplus height collapses to the bottom.

## 2. FULLSCREEN — broken in two directions (worst bugs)

Native fullscreen on `#htPanel` (`body.ht-fs`):

- **Short log:** `.ht-out { flex:1; height:auto }` is set, but its parent
  `.ht-term` is still `display:block`, so `flex:1` does nothing. Measured at
  1920×1080: terminal = **274px**, **731px of dead panel** below it, controls
  floating at y=365 in a 1080px screen.
- **Long log (30+ `status` commands):** because `.ht-out` has no working
  height constraint in FS, it grows to its content — measured `outClientH =
  6389px`, and the input row landed at **y=6498, completely off-screen**.
  The panel is `overflow:hidden`, so it cannot be scrolled to. The prompt is
  unreachable → the game is unplayable in fullscreen once the log grows.

## 3. MISSION PRESENTATION

- Mission briefings print a literal ASCII rule `══════…` (38 chars) **twice**
  around the mission title, plus `— Mission N · Title —` repeated again when
  the challenge opens. Over a full run: **12 `l-hd` separator lines / 8
  duplicated `═` rules** in one log.
- The `INTEL` block header is a text line styled like every other header;
  the actual clue lines are emitted with an **empty class** (`htOut(x,'')`),
  so `.ht-clue` styling only exists under `.ht-brief`, which is never used
  (dead CSS). Clues render as plain default-green text — indistinguishable
  from system chatter.
- Instructions, terminal transcript and intel are all the same visual weight,
  distinguished only by colour.

## 4. TYPOGRAPHY & TERMINAL FEEL

- Base output is 12px / 19.44px line-height — dense and uniform. Commands,
  errors, hints and intel differ *only* by colour; there is no prefixing,
  weight or spacing hierarchy.
- User commands echo as `$ cmd` in near-white, identical to system output
  lines like `$ system.connect()` that the *game* prints — you cannot tell
  what you typed from what the simulation said.
- The `MISSION SUMMARY` uses dot-padded `Final score ....... 2460 / 1800`
  where 1800 is not even the real maximum (base 1800 + time bonus ⇒ 2520).

## 5. CONTROL BAR

- Controls sit *below* the terminal in their own row, wrapped in the generic
  `.si-help` hint line beneath them — visually detached from the terminal
  frame rather than integrated with it.
- On phones the six buttons stack into a 130px block (3 rows) that pushes the
  panel to 793px in a 700px viewport; since `body.ft-playing` sets
  `overflow:hidden`, the last controls are **clipped and unreachable** until
  the user discovers the inner `.content` scroll.
- Sound button label is `♪sound off` (icon glyph + text jammed together).

## 6. DEAD / MISLEADING CODE & COPY

- `.ht-brief`, `.ht-summary`, `.ht-sum`, `.ht-answers`, `.ht-chip` CSS blocks
  are entirely unused by the JS (verified: zero call sites).
- Victory banner says `/ 1800` while 2520 is achievable.
- `#htOvLaunch` mount is rendered then immediately `hidden = true` — dead DOM.

## 7. What is already correct (must not regress)

Verified working before the change: all four missions solvable end-to-end
(score 2520 on a perfect run), hint halving, wrong-answer attempts, timer
warn/crit classes, pause/resume label sync, restart reset, sound toggle +
aria-pressed, Fullscreen ⇄ Exit Fullscreen label, Back to Games navigation,
Esc handling, `clear`, `scan`, `status`, unknown-command path, history
Up/Down, auto-pause on tab hide, no horizontal overflow at 320–2560px, no JS
errors.

> Note: mission 4’s checker splits on `[,\s>->]+`, which keeps the `-` of an
> arrow-form answer (`bypass 4 -> 5 -> 1 -> 2`) as a token and rejects it.
> That is pre-existing behaviour and out of scope for a visual fix; the
> comma form works. Not changed.

---

# Fixes implemented

All changes are confined to the HT block of `console.html` (CSS), the HT
section of `console-app.js` (markup + output classes), plus new tests.
**No engine, mission, puzzle, scoring, timer, hint or command logic was
touched.**

## 1. Terminal layout — flex chain instead of fixed clamp
- `.ht-term` is now a **flex column** (`bar / scrolling log / input row`),
  so `.ht-out`'s `flex:1` actually applies. The old
  `height:clamp(240px,38vh,380px)` is gone.
- Windowed play: `#htPanel` gets `flex:1; max-height:100vh; overflow:hidden`
  — it fills the viewport exactly and never exceeds it.
- Fullscreen/immersive: `.ht-term{flex:1;min-height:0}` + the panel's column
  flow, so the log takes every spare pixel.

Measured fill (output height as % of viewport, windowed):

| Viewport | before | after |
|---|---|---|
| 1280×720  | 38% | 44% |
| 1440×900  | 38% | 55% |
| 1920×1080 | 35% | 62% |
| 2560×1440 | 26% | **72%** |

Dead space below the panel: **217–719px → 0px** at every size.

## 2. Fullscreen — both failure modes fixed
- Short log: terminal was 274px with 731px of dead panel → now fills the
  screen (937px term at 1920×1080).
- Long log: the input row was measured at **y=6498, unreachable** (panel
  `overflow:hidden`, no scroll) → the log now scrolls internally
  (`outClientH` capped, `scrollHeight` grows) and the prompt stays pinned.
  Verified with a 40-command log in native FS, immersive fallback and
  windowed mode.

## 3. Mission presentation
- ASCII `════` bars removed (were printed twice per briefing; 8 duplicated
  rules over a full run). Replaced by one labelled section rule
  (`.l-hd` eyebrow with a hairline gradient tail).
- New persistent **mission header band** (`.ht-mhead`) above the terminal:
  `LOCK n/4 · title · phase · budget`, kept in sync on every state change.
- Intel clues now render through the styled `.ht-clue` group (indent +
  `▸` marker + cyan), visually separated from instructions and transcript.

## 4. Terminal experience
- Base type 12→12.5px, line-height 1.62→1.7, `l-big` 13.5→14px.
- New semantic classes: `l-sys` (system narrative), `l-sim` (simulation
  transcript, dimmed, `$`-prefixed) vs `l-cmd` (operator echo, `❯`-prefixed,
  bold) — you can now tell what you typed from what the simulation said.
- Errors/hints/ok keep their colours but gain weight/size cues, so meaning
  survives without colour vision.
- Victory summary now labels the real maximum (`2520`, base × 1.4 time
  bonus) instead of the wrong `/ 1800`.

## 5. Control bar
- Docked **inside the terminal frame** (shared border, closing the rounded
  bottom) instead of a loose row beneath it.
- All six controls verified reachable at 320–2560px; the phone case that
  previously clipped the last buttons (793px panel in a 700px viewport,
  `body{overflow:hidden}`) now fits exactly with nothing clipped.

## 6. Cleanup
- Dead CSS removed: `.ht-brief`, `.ht-summary`, `.ht-sum`, `.ht-answers`,
  `.ht-chip` (zero call sites).
- Dead DOM removed: the `#htOvLaunch` mount that was rendered then hidden.

# Verification

- `scripts/verify_hack_terminal.py` — **71/71 checks pass**: responsive
  matrix (9 viewports × windowed), native fullscreen, immersive fallback,
  long-log scrolling, all four missions end-to-end through the real command
  engine (perfect run = 2520), hints, attempts, timer warn/crit, pause/
  resume, restart, sound, fullscreen label round-trip, Back to Games,
  presentation contract, zero page errors.
- `crates/loop-dashboard/tests/hack_terminal_ui.rs` — **6 new Rust tests**
  covering the mission contract, presentation contract, responsive layout
  contract, line semantics, route/lifecycle wiring, and other-game survival.
- Full `cargo test -p loop-dashboard`: 363 lib + 41 integration tests pass.
  Two pre-existing failures are unrelated to this change:
  - `sysmon::health` tests assert against live machine state (kernel memory
    pressure was genuinely elevated during one run; they pass when it isn't).
  - `cyber_rush_ui::controls_split_nitro_and_drift` expects the literal
    string `"Shift drift"` while Cyber Rush's help text reads "hold Shift
    while steering to drift" — a stale test expectation in a game this task
    explicitly forbids touching.
- Contrast audit: every line class measures **7.3:1 – 17.0:1** against the
  terminal background (WCAG AA requires 4.5:1).

# Known pre-existing behaviour (unchanged, out of scope)
Mission 4's checker splits answers on `[,\s>->]+`; an arrow-form answer
(`bypass 4 -> 5 -> 1 -> 2`) leaves `-` tokens and is rejected, while the
comma form works. This is gameplay logic, not presentation, so it was left
exactly as-is.
