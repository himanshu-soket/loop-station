/* ============================================================
   FUN TIME · GAME 9 — LUDO (CLASSIC 4-PLAYER) · UI LAYER
   Route: #funtime/lu · Namespace: LU / lu* functions / .lu- CSS.

   Renders the LudoEngine state and owns NOTHING about the rules:
   every legality check, capture and finish flows through the engine.
   The UI may never bypass it.

   Turn lifecycle (single controller, epoch-guarded):
     START TURN → ROLL → LEGAL MOVES → [SELECT] → ANIMATE →
     RESOLVE CAPTURE/FINISH → CHECK WINNER → EXTRA ROLL or NEXT

   One `LUG.epoch` integer invalidates every stale async continuation:
   restart, teardown or route-change bumps it, so no old timer can
   roll a dice, move a token or hand the turn on for a dead game.

   Mirrors the other Fun Time games exactly (timberman/fruitninja):
   namespaced CSS, luOn() listener registry, ftSessionAfterMount /
   ftSessionBind shared fullscreen+Esc lifecycle, zero DOM leakage.
   ============================================================ */

/* ---- Injected styles (namespaced; no global selectors touched) ---- */
(function luInjectCss() {
  if (document.getElementById('lu-style')) return;
  const st = document.createElement('style');
  st.id = 'lu-style';
  st.textContent = `
.lu-wide{max-width:860px}
.lu-hud{display:flex;gap:10px;flex-wrap:wrap;align-items:center;padding:0 22px}
/* (Phase 5.2) retained for any residual HUD chips */
/* ---- Phase 5.2: player panels ------------------------------------------
   One compact card per ACTIVE seat: colour identity, human/AI + level,
   live yard/track/home counts, own die, current-turn indicator.
   Inactive seats are not rendered at all (the board already fades
   their geometry). Cards are single-line so the stage keeps height. */
.lu-panels{display:grid;gap:8px;padding:2px 22px 6px;
  grid-template-columns:repeat(auto-fit,minmax(178px,1fr))}
.lu-panel{position:relative;display:flex;align-items:center;gap:8px;
  border:1px solid var(--line2);border-radius:11px;padding:7px 10px;
  background:rgba(9,14,24,.55);min-width:0;
  transition:border-color .18s ease,box-shadow .18s ease,opacity .18s ease}
/* colour spine: a 3px rail on the panel's left edge */
.lu-panel::before{content:'';position:absolute;left:-1px;top:7px;bottom:7px;
  width:3px;border-radius:2px;background:var(--pc,#5ef2c0);opacity:.5;
  transition:opacity .18s ease}
.lu-panel.on{border-color:color-mix(in srgb,var(--pc) 55%,var(--line2));
  box-shadow:0 0 0 1px color-mix(in srgb,var(--pc) 30%,transparent),
    0 6px 18px -8px rgba(0,0,0,.55)}
.lu-panel.on::before{opacity:1}
/* identity block: name over kind, colour dot leading */
/* identity block: name over kind, colour dot leading.
   Phase 5.6 UI fix — flex-basis keeps a fair share for the identity
   so the stats row can never squeeze it to 0 (name/dice overlap). */
.lu-p-id{display:flex;flex-direction:column;gap:2px;min-width:0;
  flex:1 1 auto}
.lu-p-head{display:flex;align-items:center;gap:6px;min-width:0}
.lu-p-dot{width:9px;height:9px;border-radius:50%;background:var(--pc);
  flex:0 0 auto;box-shadow:0 0 8px var(--pc)}
.lu-p-name{font:700 13px Sora;color:#eaf2ff;letter-spacing:.01em;
  white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.lu-p-kind{font:600 8.5px Sora;letter-spacing:.09em;text-transform:uppercase;
  color:#aab6c8;white-space:nowrap;margin-left:2px;overflow:hidden;
  text-overflow:ellipsis}
.lu-p-kind.ai{color:#ffd166}
.lu-p-kind.human{color:#5ef2c0}
/* token counts: three tight inline chips (MIDDLE zone).
   min-width:0 so they yield space instead of overlapping; NO wrap —
   the three chips always stay on one line so every panel keeps the
   same height (Phase 5.6 UI fix). They shrink via their own padding
   when narrow, and at extreme narrowness the identity truncates first. */
.lu-p-row{display:flex;gap:4px;flex:0 1 auto;min-width:0;
  justify-content:flex-end;flex-wrap:nowrap}
.lu-p-stat{min-width:0}
.lu-p-stat .k{flex:0 0 auto}
.lu-p-stat{display:inline-flex;align-items:baseline;gap:3px;
  border:1px solid var(--line2);border-radius:7px;padding:2px 6px;
  background:rgba(0,0,0,.25)}
.lu-p-stat .k{font:600 8.5px Sora;letter-spacing:.1em;color:#7f8ca1;
  text-transform:uppercase}
.lu-p-stat .v{font:800 13px "JetBrains Mono",monospace;color:#eaf2ff;
  line-height:1.1}
.lu-p-stat.h .v{color:var(--pc)}
/* the seat's own die lives at the card's right edge (Phase 5.2).
   Phase 5.6 fix — the die renders ONLY on the current roller's panel.
   Inactive panels carry an invisible fixed-size SLOT (.lu-p-die-slot)
   instead of a dashed placeholder: dimensions stay identical across
   every panel, so nothing shifts when the turn (and the die) moves. */
.lu-p-die{flex:0 0 auto;display:inline-flex;margin-left:auto;
  width:26px;height:26px}
.lu-p-die-slot{width:26px;height:26px;border:0;background:transparent;
  visibility:hidden}
.lu-panel .lu-die{width:26px;height:26px;border-radius:7px;padding:4px;gap:1px;
  background:#f2f7ff;display:inline-grid;grid-template-columns:repeat(3,1fr);
  grid-template-rows:repeat(3,1fr);
  border:1.5px solid color-mix(in srgb,var(--pc) 70%,#0a0f1a);
  box-shadow:inset 0 -2px 4px rgba(0,0,0,.35),0 2px 6px -2px rgba(0,0,0,.5)}
.lu-panel .lu-die i{border-radius:50%;background:var(--pc);place-self:center;
  width:5px;height:5px}
.lu-panel.rolling .lu-die{animation:lushake .5s ease-in-out infinite}
/* current-turn flag: floats over the card's top-right corner */
.lu-p-turn{display:none;position:absolute;right:8px;top:-8px;
  align-items:center;gap:5px;font:700 9px Sora;letter-spacing:.08em;
  text-transform:uppercase;color:var(--pc);background:#0a0f1ae6;
  border-radius:6px;padding:1px 7px}
.lu-panel.on .lu-p-turn{display:flex}
.lu-p-turn .pulse{width:5px;height:5px;border-radius:50%;background:var(--pc);
  animation:luturnpulse 1.1s ease-in-out infinite}
@keyframes luturnpulse{0%,100%{opacity:.35;transform:scale(.8)}50%{opacity:1;transform:scale(1.15)}}
/* winner state */
.lu-panel.won{border-color:var(--pc);
  box-shadow:0 0 0 1px var(--pc),0 6px 20px -8px rgba(0,0,0,.6)}
.lu-p-won{display:none;position:absolute;right:8px;top:-8px;
  align-items:center;gap:5px;font:800 9px Sora;letter-spacing:.1em;
  text-transform:uppercase;color:var(--pc);background:#0a0f1ae6;
  border-radius:6px;padding:1px 7px}
.lu-panel.won .lu-p-won{display:flex}
.lu-panel.won .lu-p-turn{display:none}
.lu-stage{position:relative;margin:8px 22px;border:1px solid var(--line2);border-radius:14px;overflow:hidden;
  background:linear-gradient(180deg,#0b1322 0%,#0e1a2e 60%,#12233b 100%);
  user-select:none;-webkit-user-select:none;touch-action:manipulation}
#luBoard{display:block;width:100%;height:auto}
.lu-overlay{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;
  background:radial-gradient(ellipse at center,rgba(6,10,18,.62) 0%,rgba(6,10,18,.88) 100%);
  backdrop-filter:blur(3px);z-index:5;text-align:center;padding:18px}
.lu-overlay.hidden{display:none}
.lu-ov-inner{max-width:520px;width:100%}
.lu-ov-title{font:800 clamp(26px,4vw,40px) Sora;letter-spacing:-.02em;color:#f2f7ff;margin-bottom:8px;
  text-shadow:0 0 30px rgba(94,242,192,.25)}
.lu-ov-sub{font:500 14px Sora;color:#9fb0c7;line-height:1.65;margin-bottom:18px}
.lu-ov-sub b{color:#e5edf9}
.lu-ov-row{display:flex;gap:10px;justify-content:center;flex-wrap:wrap}
.lu-ov-btn{border:1px solid var(--line2);border-bottom-width:3px;border-radius:10px;padding:11px 22px;
  background:var(--panel2);color:var(--ink);font:700 13px Sora;cursor:pointer;transition:transform .12s,border-color .12s}
.lu-ov-btn:hover{transform:translateY(-1px);border-color:var(--acc-dim)}
.lu-ov-btn.primary{background:var(--acc);border-color:var(--acc-dim);color:#06231a}
.lu-ov-stats{display:flex;gap:14px;justify-content:center;flex-wrap:wrap;margin:0 0 16px}
.lu-ov-stat{border:1px solid var(--line2);border-radius:10px;padding:8px 14px;background:rgba(0,0,0,.35)}
.lu-ov-stat .k{font:600 10px Sora;letter-spacing:.12em;text-transform:uppercase;color:#7f8ca1}
.lu-ov-stat .v{font:800 20px "JetBrains Mono",monospace;color:#eaf2ff}
.lu-controls{display:flex;gap:10px;flex-wrap:wrap;padding:14px 22px 6px}
.lu-msg{padding:8px 22px 16px;font:500 13px Sora;color:#9fb0c7;min-height:20px}
.lu-seatpick{display:flex;align-items:center;gap:6px;padding:5px 10px;border:1px solid var(--line2);
  border-radius:10px;background:rgba(0,0,0,.32)}
.lu-sp-k{font:600 11px Sora;color:#aab6c8;letter-spacing:.08em;margin-right:2px}
.lu-sp-btn{border:1px solid var(--line2);border-radius:8px;padding:5px 10px;background:transparent;
  color:#aab6c8;font:700 12px Sora;cursor:pointer;transition:border-color .12s,color .12s,background .12s}
.lu-sp-btn:hover{color:var(--ink);border-color:var(--acc)}
.lu-sp-btn.on{background:rgba(94,242,192,.10);border-color:var(--acc);color:var(--acc)}
.lu-sp-dot{width:9px;height:9px;border-radius:50%;display:inline-block;margin-left:4px;vertical-align:middle}
.lu-sp-btn.lu-soon{opacity:.45;cursor:not-allowed;border-style:dashed}
.lu-sp-btn.lu-soon:hover{color:#aab6c8;border-color:var(--line2)}
.lu-msg b{color:#e5edf9}
.lu-nudge{color:#ffb4a2}
.lu-dice-btn{display:inline-flex;align-items:center;gap:10px;border:1px solid var(--acc-dim);
  border-bottom-width:3px;border-radius:10px;padding:10px 18px;background:rgba(94,242,192,.08);
  color:var(--acc);font:800 14px Sora;cursor:pointer;transition:transform .12s,border-color .18s,
  box-shadow .18s}
/* Phase 5.2: clickable state glows with the current player's colour */
.lu-dice-btn.on{border-color:color-mix(in srgb,var(--pc,var(--acc)) 55%,var(--line2));
  box-shadow:0 0 0 1px color-mix(in srgb,var(--pc,var(--acc)) 25%,transparent)}
.lu-dice-btn:hover:not(:disabled){transform:translateY(-1px)}
.lu-dice-btn:disabled{opacity:.38;cursor:not-allowed}
.lu-die{width:34px;height:34px;border-radius:8px;background:#f2f7ff;display:inline-grid;grid-template-columns:repeat(3,1fr);
  grid-template-rows:repeat(3,1fr);padding:5px;gap:1px}
.lu-die i{border-radius:50%;background:#10141d;place-self:center;width:6px;height:6px}
.lu-die.rolling{animation:lushake .5s ease-in-out infinite}
@keyframes lushake{0%,100%{transform:rotate(0)}25%{transform:rotate(-14deg)}75%{transform:rotate(14deg)}}
.lu-help{display:flex;gap:14px;flex-wrap:wrap;padding:4px 22px 18px;font:500 11.5px Sora;color:#7f8ca1}
/* ---- PHASE 1 RESPONSIVE · portrait phones --------------------------------
   The four player panels, the control row and the help line consumed
   ~450px of an 844px phone viewport, starving the square board to ~131px.
   On narrow portrait screens: fold the header + help away (the Back button
   and in-game messages carry the same information), make the seat panels a
   compact 2×2 grid, and let the control row wrap tightly. The board then
   gets the width and most of the height of the screen. */
@media (max-width:700px) and (orientation:portrait){
  body.ft-playing #luPanel .ft-panel-h{display:none}    /* title duplicates the top bar */
  body.ft-playing #luPanel{padding:10px 12px 12px;gap:6px}
  .lu-panels{grid-template-columns:repeat(2,minmax(0,1fr));gap:6px;
    padding:0 12px 2px}
  .lu-panel{padding:5px 8px}
  .lu-panel::before{top:4px;bottom:4px}
  .lu-p-name{font-size:12px}
  .lu-p-stat{padding:1px 5px}
  .lu-p-stat .v{font-size:12px}
  .lu-panel .lu-die,.lu-p-die,.lu-p-die-slot{width:22px;height:22px}
  .lu-panel .lu-die{padding:3px;border-radius:6px}
  .lu-panel .lu-die i{width:4px;height:4px}
  .lu-controls{gap:8px;padding:8px 12px 2px}
  .lu-controls .ft-btn{padding:8px 12px;font-size:11.5px}
  .lu-dice-btn{padding:8px 14px;font-size:13px}
  .lu-msg{padding:4px 12px 10px;min-height:16px;font-size:12px}
  .lu-help{display:none}                     /* hints live in the help overlay */
  .lu-stage{margin:4px 12px}
}
/* Very narrow phones (320px): seat panels stack 2×2 but their stat chips
   may still crowd — allow the identity to truncate earlier. */
@media (max-width:380px){
  .lu-p-kind{display:none}
  .lu-p-stat .k{letter-spacing:.04em}}

/* PHASE 1 · landscape phones (short viewports): the seat strip, message
   row and control row together eat ~180px of a 390px viewport, starving
   the stage to a ~146px board. Compact them so the board keeps the
   screen: seats collapse to one tight row, controls wrap tightly, the
   status line shrinks, panel gaps close up. */
@media (max-height:560px) and (orientation:landscape){
  body.ft-playing #luPanel .ft-panel-h{display:none}
  body.ft-playing #luPanel{padding:6px 12px 8px;gap:4px}
  .lu-panels{grid-template-columns:repeat(4,minmax(0,1fr));gap:4px;
    padding:0 12px 0}
  .lu-panel{padding:3px 6px;gap:5px;border-radius:8px}
  .lu-panel::before{top:3px;bottom:3px}
  .lu-p-name{font-size:10.5px}
  .lu-p-stat{padding:1px 4px}
  .lu-p-stat .v{font-size:10.5px}
  .lu-p-stat .k{font-size:8px}
  .lu-panel .lu-die,.lu-p-die,.lu-p-die-slot{width:18px;height:18px}
  .lu-panel .lu-die{padding:2px;border-radius:5px}
  .lu-panel .lu-die i{width:3px;height:3px}
  .lu-controls{gap:6px;padding:4px 12px 0}
  .lu-controls .ft-btn{padding:5px 9px;font-size:10.5px}
  .lu-dice-btn{padding:5px 11px;font-size:12px}
  .lu-msg{padding:2px 12px 6px;min-height:14px;font-size:11px}
  .lu-help{display:none}
  .lu-stage{margin:2px 12px}
}
/* ---- Phase 5.5: game setup lobby ----------------------------------------
   Covers the stage area (board visible but dimmed behind), with a card
   matching the Ludo surface language (.lu-seatpick / .lu-ov-*). Mobile:
   full-width sheet that scrolls if needed — nothing clips. */
.lu-lobby{position:absolute;inset:0;z-index:7;display:flex;
  align-items:center;justify-content:center;padding:18px;
  background:radial-gradient(ellipse at center,rgba(6,10,18,.55) 0%,rgba(6,10,18,.86) 100%);
  backdrop-filter:blur(3px)}
.lu-lobby.hidden{display:none}
.lu-lob-card{width:min(560px,100%);max-height:100%;overflow-y:auto;
  display:flex;flex-direction:column;gap:13px;padding:18px 20px;
  background:rgba(10,16,28,.97);border:1px solid var(--line2);border-radius:14px;
  box-shadow:0 18px 50px rgba(0,0,0,.5)}
.lu-lob-head{display:flex;align-items:center;justify-content:space-between}
.lu-lob-title{font:800 12px Sora;letter-spacing:.16em;color:#aab6c8}
.lu-lob-sec{display:flex;flex-direction:column;gap:7px}
.lu-lob-k{font:700 10px Sora;letter-spacing:.13em;color:#7f8ca1}
/* Phase 5.6 UI fix — three equal configuration columns (PLAYERS /
   HUMANS / AI LEVEL). The old auto-fit grid gave each column ~157px,
   but the selector rows are nowrap flex with min-width:auto content
   (AI LEVEL needs ~282px), so they overflowed their column and clipped.
   Fix: fixed 3-column grid + every group is allowed to WRAP and shrink
   (min-width:0, flex-wrap:wrap), buttons share width via flex:1 1 0
   so nothing is ever clipped or overlapped at any viewport. */
.lu-lob-cfg{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));
  gap:11px;padding:12px;border:1px solid var(--line2);border-radius:11px;
  background:rgba(0,0,0,.24)}
.lu-lob-cfg .lu-lob-sec{min-width:0}        /* let grid tracks shrink */
.lu-lobby .lu-seatpick{flex-wrap:wrap;min-width:0}
.lu-lobby .lu-sp-btn{flex:1 1 0;min-width:0;justify-content:center;
  display:inline-flex;align-items:center;text-align:center}
/* Phase 5.6 follow-up — AI LEVEL has four long single-word labels
   (Classic/Easy/Medium/Hard). Sharing one flex row with flex-basis:0
   squeezed each button to ~29px inside its ~157px column, so the
   labels overflowed their boxes and visually merged. Dedicated layout:
   a 2×2 button grid (tracks sized minmax(0,1fr), snug horizontal
   padding) — every label stays fully readable at every breakpoint,
   and the group simply never needs more than two button widths.
   Players/Humans keep the shared flex-row treatment above. */
#luDiffPick{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));
  gap:6px;align-items:stretch}
#luDiffPick .lu-sp-btn{flex:none;width:100%;padding-inline:6px;
  white-space:nowrap}
.lu-lob-cfg.off{opacity:.38;pointer-events:none}      /* Quick Start: fixed */
.lu-lob-prev{display:grid;gap:8px;
  grid-template-columns:repeat(auto-fit,minmax(232px,1fr))}
.lu-lob-seat{display:flex;align-items:center;gap:9px;padding:8px 10px;
  border:1px solid var(--line2);border-radius:11px;background:rgba(0,0,0,.30);
  min-width:0}
.lu-lob-seat.inactive{opacity:.32;filter:saturate(.4)}
.lu-lob-seat .lu-p-dot{flex:none}
.lu-lob-si{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px}
.lu-lob-sn{font:700 12.5px Sora;color:#eaf2ff;white-space:nowrap;
  overflow:hidden;text-overflow:ellipsis}
.lu-lob-sk{font:600 9px Sora;letter-spacing:.09em;text-transform:uppercase;color:#9fb0c7}
.lu-lob-sk.ai{color:#ffd166}.lu-lob-sk.human{color:#5ef2c0}
.lu-lob-nm{flex:1;min-width:0;max-width:150px;border:1px solid var(--line2);
  border-radius:8px;padding:5px 8px;background:rgba(0,0,0,.35);
  color:#eaf2ff;font:600 12px Sora;outline:none;
  transition:border-color .12s}
.lu-lob-nm:focus-visible{border-color:var(--acc);
  box-shadow:0 0 0 2px rgba(94,242,192,.25)}
.lu-lob-nm::placeholder{color:#5c6a80}
.lu-lob-actions{display:flex;justify-content:flex-end;padding-top:2px}
.lu-lob-start{min-width:170px}
@media (max-width:560px){
  .lu-lobby{padding:10px;align-items:flex-start}
  .lu-lob-card{padding:14px 14px 16px}
  /* mobile: configuration sections stack vertically (Players → Humans
     → AI Level); buttons still share the row width via flex-wrap */
  .lu-lob-cfg{grid-template-columns:minmax(0,1fr)}
  .lu-lob-prev{grid-template-columns:1fr}
  .lu-lob-actions{justify-content:stretch}
  .lu-lob-start{width:100%}
  .lu-lob-nm{max-width:none}
}
/* tablet band: two-column arrangement so each group keeps comfortable
   button widths while nothing overflows (Phase 5.6 UI fix) */
@media (min-width:561px) and (max-width:900px){
  .lu-lob-cfg{grid-template-columns:repeat(2,minmax(0,1fr))}
}
/* short landscape phones: the card becomes a compact two-column layout
   so it fits beside a squashed board without clipping.
   Phase 5.6 follow-up: the AI LEVEL group is now a 2×2 grid (readable
   labels), which adds ~36px of height — reclaim it here so the Start
   button stays inside the 390px-tall viewport (verify_ludo_phase55
   counts any viewport-clipped control as a failure). */
@media (max-height:520px) and (orientation:landscape){
  .lu-lobby{padding:8px;align-items:stretch}
  .lu-lob-card{width:min(640px,100%);gap:7px;padding:10px 14px}
  .lu-lob-cfg{grid-template-columns:repeat(3,minmax(0,1fr));padding:7px;
    gap:8px}
  .lu-lobby .lu-seatpick{padding:4px 8px}
  #luDiffPick{gap:5px}
  /* seat preview: single compact row of four slim seats reclaims the
     height the 2×2 AI grid needs, keeping Start inside the viewport */
  .lu-lob-prev{grid-template-columns:repeat(4,minmax(0,1fr))}
  .lu-lob-seat{padding:5px 7px;gap:6px}
  .lu-lob-actions{justify-content:flex-start}
}
/* keyboard focus states for all lobby controls (Phase 5.5 a11y) */
.lu-lobby .lu-sp-btn:focus-visible,
.lu-lobby .lu-lob-start:focus-visible,
.lu-lobby .lu-au-x:focus-visible{
  outline:2px solid var(--acc);outline-offset:2px}
.lu-lobby .lu-sp-btn[aria-checked="true"]{background:rgba(94,242,192,.10);
  border-color:var(--acc);color:var(--acc)}
/* touch-friendly: every lobby radio clears the 28px minimum target
   (WCAG 2.2 · 2.5.8) without changing the visual rhythm much */
.lu-lobby .lu-sp-btn{min-height:30px}

/* ---- Phase 5.4: audio settings popover --------------------------------
   Anchored to the top-right of the controls area; mobile keeps it inside
   the panel via max-width + right-anchoring. Same surface language as
   .lu-seatpick / .lu-ov-stat so it reads as native Ludo chrome. */
.lu-audio-pop{position:absolute;top:64px;right:18px;z-index:6;width:290px;
  padding:12px 14px;display:flex;flex-direction:column;gap:9px;
  background:rgba(10,16,28,.96);border:1px solid var(--line2);border-radius:12px;
  box-shadow:0 12px 34px rgba(0,0,0,.45);backdrop-filter:blur(4px)}
/* the anchor .ft-panel is not positioned by default — scope a local
   positioning context so the popover attaches to the Ludo panel only */
#luPanel{position:relative}
.lu-audio-pop.hidden{display:none}
.lu-au-row{display:flex;align-items:center;gap:10px}
.lu-au-head{justify-content:space-between;margin-bottom:2px}
.lu-au-title{font:700 11px Sora;letter-spacing:.14em;color:#aab6c8}
.lu-au-x{border:none;background:transparent;color:#7f8ca1;cursor:pointer;
  font-size:13px;line-height:1;padding:4px 6px;border-radius:6px}
.lu-au-x:hover{color:var(--ink);background:rgba(255,255,255,.06)}
.lu-au-l{flex:0 0 104px;font:600 11.5px Sora;color:#9fb0c7}
.lu-au-slider{flex:1;min-width:0;accent-color:var(--acc);height:18px}
.lu-au-val{flex:0 0 38px;text-align:right;font:700 11px "JetBrains Mono",monospace;color:#e5edf9}
.lu-au-toggles{gap:8px;margin-top:2px}
.lu-au-toggle{flex:1;display:inline-flex;align-items:center;justify-content:center;gap:7px;
  border:1px solid var(--line2);border-radius:9px;padding:7px 10px;background:transparent;
  color:#9fb0c7;font:700 11.5px Sora;cursor:pointer;
  transition:color .12s,border-color .12s,background .12s}
.lu-au-toggle:hover{color:var(--ink);border-color:var(--acc)}
.lu-au-dot{width:8px;height:8px;border-radius:50%;background:#4a5568;transition:background .12s}
.lu-au-toggle[aria-pressed="true"]{color:var(--acc);
  background:rgba(94,242,192,.09);border-color:rgba(94,242,192,.45)}
.lu-au-toggle[aria-pressed="true"] .lu-au-dot{background:var(--acc)}
@media (max-width:520px){
  .lu-audio-pop{top:auto;bottom:14px;right:12px;left:12px;width:auto}
}
.lu-help span::before{content:'·';margin-right:6px;color:#5ef2c0}
/* ---- Fullscreen / immersive layout (same convention as tm/fn) -------
   The shared session controller locks page scroll while a game is
   playing (body.ft-playing{overflow:hidden}), so the square board MUST
   fit the viewport: the panel gets a definite height, the stage flexes
   to the leftover space, and luResize() fits the board into
   min(width, height). Without this the width-only fit grows the board
   taller than the viewport and it clips (controls unreachable). */
body.lu-fs #luPanel,body.ft-playing.ft-immersive #luPanel{
  max-width:none;width:100vw;height:100vh;margin:0;border-radius:0;border:none;overflow:hidden;
  padding:clamp(10px,2vh,22px) clamp(12px,3vw,36px);box-sizing:border-box;justify-content:flex-start;
  background:var(--bg,#0a0d13)}
body.lu-fs #luPanel .ft-panel-h,body.ft-playing.ft-immersive #luPanel .ft-panel-h{display:none}
body.lu-fs #luStageWrap,body.ft-playing.ft-immersive #luStageWrap,
body.ft-playing #luStageWrap{
  flex:1;display:flex;flex-direction:column;justify-content:center;min-height:0}
/* Phase 5.6: the fitted square board must sit in the horizontal centre
   of the (usually wider) stage — the canvas carries an inline width from
   luResize(), so without this it hugs the left edge and looks off-balance
   in fullscreen on wide screens. Same for the inline play layout. */
#luStageWrap{align-items:center}
#luBoard{margin:0 auto}
/* Inline play (direct #funtime/lu visit): the session already hides the
   dashboard chrome, so give the panel a definite viewport height for the
   stage's flex to resolve against (keeps sizing non-circular). */
body.ft-playing #luPanel{height:100vh;box-sizing:border-box}

/* ---- Fun Time selector-card art (matches fn/tm/ch card conventions) ---- */
.ft-card-art.lu{background:radial-gradient(120% 120% at 20% 0%,#1d2a3f 0%,#101724 55%,#0b101b 100%);
  display:flex;align-items:center;justify-content:center}
.lu-card-scene{position:relative;width:84px;height:84px;display:block}
.lu-card-board{position:absolute;inset:0;border-radius:12px;background:#131c2e;
  border:2px solid #243149;
  background-image:
    linear-gradient(#ff5d5d33,#ff5d5d33),linear-gradient(#5ef2c033,#5ef2c033),
    linear-gradient(#ffd16633,#ffd16633),linear-gradient(#6ea8ff33,#6ea8ff33);
  background-size:50% 50%;background-position:0 0,100% 0,100% 100%,0 100%;
  background-repeat:no-repeat}
.lu-card-pawn{position:absolute;width:16px;height:16px;border-radius:50%;
  border:2px solid rgba(255,255,255,.6);box-shadow:0 0 12px currentColor}
.lu-card-pawn.r{background:#ff5d5d;color:#ff5d5d;left:18px;top:18px;animation:lucardr 2.4s ease-in-out infinite}
.lu-card-pawn.g{background:#5ef2c0;color:#5ef2c0;right:18px;top:18px;animation:lucardg 2.4s ease-in-out infinite}
.lu-card-pawn.y{background:#ffd166;color:#ffd166;right:18px;bottom:18px;animation:lucardy 2.4s ease-in-out infinite}
.lu-card-pawn.b{background:#6ea8ff;color:#6ea8ff;left:18px;bottom:18px;animation:lucardb 2.4s ease-in-out infinite}
@keyframes lucardr{0%,100%{transform:translate(0,0)}50%{transform:translate(10px,10px)}}
@keyframes lucardg{0%,100%{transform:translate(0,0)}50%{transform:translate(-10px,10px)}}
@keyframes lucardy{0%,100%{transform:translate(0,0)}50%{transform:translate(-10px,-10px)}}
@keyframes lucardb{0%,100%{transform:translate(0,0)}50%{transform:translate(10px,-10px)}}
/* ---- Mobile landscape (short viewport): keep the board on screen by
   compacting chrome instead of collapsing the stage flat. Placed LAST
   so it overrides the base rules above. Phase 5.2: panels collapse to a
   single dense row and dice shrink with them. ---- */
@media (max-height:520px) and (orientation:landscape){
  .lu-controls{flex-wrap:nowrap;overflow-x:auto;padding:8px 12px 4px;
    scrollbar-width:none}
  .lu-controls::-webkit-scrollbar{display:none}
  .lu-seatpick{flex:0 0 auto;white-space:nowrap}
  .lu-msg{padding:4px 12px 8px;min-height:16px;font-size:12px;
    white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .lu-help{display:none}                       /* hint text is desktop-only */
  #luPanel .ft-panel-h{display:none}           /* reclaim header space */
  .lu-hud{padding:0 12px}
  /* panels: horizontal scroll strip of compact cards */
  .lu-panels{grid-template-columns:none;grid-auto-flow:column;
    grid-auto-columns:minmax(168px,max-content);gap:6px;padding:2px 12px 4px;
    overflow-x:auto;scrollbar-width:none}
  .lu-panels::-webkit-scrollbar{display:none}
  .lu-panel{padding:4px 8px}
  .lu-panel::before{top:4px;bottom:4px}
  .lu-panel .lu-die{width:22px;height:22px;padding:3px}
  .lu-panel .lu-die i{width:4px;height:4px}
  /* keep the reserved die slot compact in the landscape strip too */
  .lu-p-die{width:22px;height:22px}
  .lu-p-die-slot{width:22px;height:22px}
  .lu-p-name{font-size:12px}
  .lu-p-stat{padding:1px 4px}
  .lu-p-stat .v{font-size:12px}
}
/* Fullscreen in a short landscape viewport: same compaction applies
   (the immersive header is already hidden by the fs rules above).
   Phase 5.6: additionally tighten the panel's flex gap (16px → 6px),
   the stage's vertical margins and the control-row height — without
   this the square board shrank to ~168px on an 844×390 phone because
   48px of gaps + a 66px control row claimed a third of the viewport.
   Touch targets stay ≥ 40px. */
@media (max-height:520px) and (orientation:landscape){
  body.lu-fs .lu-controls,body.ft-playing.ft-immersive .lu-controls{
    flex-wrap:nowrap;overflow-x:auto;padding:6px 10px 2px;
    scrollbar-width:none}
  body.lu-fs .lu-controls::-webkit-scrollbar,
  body.ft-playing.ft-immersive .lu-controls::-webkit-scrollbar{display:none}
  body.lu-fs #luPanel,body.ft-playing.ft-immersive #luPanel{gap:6px}
  body.lu-fs .lu-stage{margin:4px 22px}
  /* the message row duplicates the roll-button label in this cramped
     layout — reclaim its 27px for the board (aria-live stays in the DOM) */
  body.lu-fs .lu-msg,body.ft-playing.ft-immersive .lu-msg{
    position:absolute;left:-9999px;width:1px;height:1px;overflow:hidden}
  body.lu-fs .lu-controls .ft-btn,
  body.ft-playing.ft-immersive .lu-controls .ft-btn{padding:6px 12px}
  body.lu-fs .lu-dice-btn,
  body.ft-playing.ft-immersive .lu-dice-btn{padding:5px 12px}
}
`;
  document.head.appendChild(st);
})();

/* ---- state ------------------------------------------------------------ */
const LUG = {
  epoch: 0,            // VIEW epoch: bumped on mount/teardown only.
                       // Button/canvas/keyboard handlers check this.
  clock: 0,            // GAME clock: bumped by new-game/teardown. Every
                       // async game-flow callback captures and re-checks
                       // it, so a restart kills all pending turn timers.
  game: null,          // LudoEngine game state
  phase: 'idle',       // idle | rolling | awaiting | moving | over
  dice: null,          // last rolled value
  moves: [],           // legal moves for the current roll
  anim: null,          // {player, token, path:[steps…], i} while walking
  diceRaf: 0,          // rAF id of the dice tumble (0 = none)
  cleanup: [],
  canvas: null, ctx: null,
  w: 0, h: 0, cell: 0,
  hoverToken: null,
  nudge: null,         // {p, i, until} illegal-click flash (visual only)
  fx: [],              // Phase 5.3: live visual effects [{kind…, t0, dur}]
  fxRaf: 0,            // rAF id of the single FX pump (0 = none)
  settleUntil: 0,      // ms deadline of a pending no-move hand-off (force guard)
  seatCount: 4,        // active players: 2 | 3 | 4 (Phase 4.1)
  humanSeats: 1,       // of the active seats, how many are human (Phase 4.2)
  aiDifficulty: null,  // 'easy' | 'medium' | 'hard' | null (null = legacy default AI)
  aiRng: null,         // per-game seeded PRNG used by Easy AI picks
  /* Phase 5.5: pre-game lobby. On mount the view shows a setup card;
     `lobby` is true until Start Game (or a Quick Start) begins play.
     Restart returns here with the configuration preserved. */
  lobby: true,
  lobbyMode: 'custom', // 'quick' | 'custom' (Quick Start = 4P·1H·Classic)
  names: ['', '', '', ''],   // per-seat display names ('' → default)
  soundOn: true,
  reducedMotion: false,
};

/* ==========================================================================
   Phase 5.4 — PROFESSIONAL AUDIO SYSTEM
   --------------------------------------------------------------------------
   Architecture (single owner, lazy, resilient):

     LUA.ctx            ONE AudioContext for the whole view lifecycle.
                        Created only after a real user gesture (autoplay
                        policy); every entry point funnels through
                        luAudioReady() so init can never race or double.
     LUA.master         master GainNode — every SFX + music bus feeds it.
     LUA.sfxBus         GainNode for sound effects (own volume).
     LUA.musicBus       GainNode for background music (own volume).
     LUA.lastPlay       per-sound-kind timestamp map → identical sounds
                        closer than LU_SFX_GAP ms apart are dropped, so
                        rAF-driven triggers (steps) can never stack into
                        a buzz, and two events in the same frame play once.
     LUA.music          music state {on, timer, step, nextT, duckUntil}.
                        The "music" is a generative ambient loop (Web
                        Audio oscillators scheduled ahead on a lookahead
                        timer — no assets, no dependencies, seamless by
                        construction). Ducking during victory stingers
                        keeps them from competing.

   Settings persist under one localStorage key ('ludo-audio-v1') and are
   validated on load; failures fall back to defaults silently.

   Cleanup: luTeardown() closes the context exactly once (guard flag) and
   clears the music timer; route-away/restart can never leak a context.
   ========================================================================== */
const LUA = {
  ctx: null, master: null, sfxBus: null, musicBus: null,
  inited: false,        // init ran (success OR failure) — never retry-spam
  lastPlay: {},         // kind → last start time (dedup guard)
  music: null,          // active music state (null = stopped)
  musicTimer: 0,        // scheduler timer id (0 = none)
};

/* persisted settings shape: {m:0..1 master, mu:0|1 music on, s:0..1 sfx,
   mv:0..1 music vol, sv:0..1 sfx vol} — compact keys, one JSON blob. */
const LU_AUDIO_KEY = 'ludo-audio-v1';
const LU_SFX_GAP = 70;          // ms: identical sounds closer than this dedup
const LU_MUSIC_LOOKAHEAD = 0.9; // s of notes scheduled ahead of playback

function luAudioDefaults() {
  /* m: master vol · mu: music on · sx: sfx on · mv: music vol · sv: sfx vol */
  return { m: 0.7, mu: 1, sx: 1, mv: 0.45, sv: 0.9 };
}

/** Load + validate persisted settings (defensive: any bad value falls
 *  back to the default for that field). Returns a fresh object. */
function luAudioLoad() {
  const d = luAudioDefaults();
  try {
    const raw = localStorage.getItem(LU_AUDIO_KEY);
    if (!raw) return d;
    const o = JSON.parse(raw);
    const num01 = v => typeof v === 'number' && isFinite(v) &&
      v >= 0 && v <= 1 ? v : null;
    if (o && typeof o === 'object') {
      const m = num01(o.m), mv = num01(o.mv), sv = num01(o.sv);
      if (m !== null) d.m = m;
      if (mv !== null) d.mv = mv;
      if (sv !== null) d.sv = sv;
      if (o.mu === 0 || o.mu === 1) d.mu = o.mu;
      if (o.sx === 0 || o.sx === 1) d.sx = o.sx;   // sfx on/off
    }
  } catch { /* corrupted blob → defaults */ }
  return d;
}

/** Persist current settings (best effort; private mode may refuse). */
function luAudioSave() {
  try {
    localStorage.setItem(LU_AUDIO_KEY, JSON.stringify({
      m: LUG.audio.m, mu: LUG.audio.mu, mv: LUG.audio.mv, sv: LUG.audio.sv,
      sx: LUG.audio.sx,
    }));
  } catch { /* storage unavailable — settings stay session-only */ }
}

/** Live settings object (loaded once at script parse; safe pre-DOM). */
LUG.audio = luAudioLoad();

/** Derived: is ANY audible output enabled? (master gate) */
function luAudioAnyOn() {
  return (LUG.audio.sx === 1 || LUG.audio.mu === 1);
}

/**
 * Ensure the audio graph exists AND is running. Returns the context or
 * null. Safe to call from every trigger — the `inited` flag means the
 * constructor runs at most once per view mount, so a failing environment
 * (no Web Audio / blocked) costs one attempt, not one per frame.
 *
 * Autoplay policy: the context starts 'suspended' until a gesture. All
 * callers already run inside click/keydown handlers (roll button, token
 * pick, controls); for programmatic triggers (AI turns) the context is
 * already unlocked by the human's first interaction with the page.
 */
function luAudioReady() {
  if (LUA.inited) {
    /* already built — just make sure it's running (a tab can suspend
       the context after backgrounding; resume is async + ignorable) */
    if (LUA.ctx && LUA.ctx.state === 'suspended')
      LUA.ctx.resume().catch(() => {});
    return LUA.ctx;
  }
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) { LUA.inited = true; return null; }      // unsupported: silent
  try {
    LUA.ctx = new AC();
    LUA.master = LUA.ctx.createGain();
    LUA.sfxBus = LUA.ctx.createGain();
    LUA.musicBus = LUA.ctx.createGain();
    LUA.master.connect(LUA.ctx.destination);
    LUA.sfxBus.connect(LUA.master);
    LUA.musicBus.connect(LUA.master);
    luAudioApplyVolumes();
    LUA.inited = true;
    if (LUA.ctx.state === 'suspended')
      LUA.ctx.resume().catch(() => {});             // gesture-gated
    return LUA.ctx;
  } catch {
    /* creation failed (hard block / resource exhaustion): mark done so
       we never spin on it, run fully silent. */
    LUA.ctx = null; LUA.inited = true; return null;
  }
}

/** Push settings into the gain graph (immediate, click-free via ramp). */
function luAudioApplyVolumes() {
  if (!LUA.ctx || !LUA.master) return;
  const t = LUA.ctx.currentTime;
  const set = (node, v) => {
    if (!node) return;
    node.gain.cancelScheduledValues(t);
    node.gain.setTargetAtTime(Math.max(0.0001, v), t, 0.02);
  };
  set(LUA.master, LUG.audio.m);
  set(LUA.sfxBus, LUG.audio.sx ? LUG.audio.sv : 0);
  set(LUA.musicBus, LUG.audio.mu ? LUG.audio.mv : 0);
}

/**
 * One synthesized tone on the SFX bus. Volume is relative (0..1) — the
 * bus/master gains do the scaling, so global changes apply mid-sound.
 */
function luTone(freq, dur, vol, type, slideTo, when) {
  const ctx = luAudioReady();
  if (!ctx || !LUG.soundOn || !freq) return;
  if (!LUG.audio.sx) return;                        // SFX muted
  const t = (when != null && when > ctx.currentTime) ? when : ctx.currentTime;
  const osc = ctx.createOscillator(), gain = ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(Math.max(20, slideTo), t + dur);
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(Math.max(0.001, vol), t + 0.008);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(LUA.sfxBus);
  osc.start(t); osc.stop(t + dur + 0.02);
  /* nodes self-release on stop; no tracking needed (short-lived SFX) */
}

/** Soft percussive noise burst (dice clack, capture thud) — generated
 *  buffer, created ONCE and reused (never per-frame allocation). */
let LU_NOISE_BUF = null;
function luNoiseBuf(ctx) {
  if (LU_NOISE_BUF) return LU_NOISE_BUF;
  const n = Math.floor(ctx.sampleRate * 0.25);
  const buf = ctx.createBuffer(1, n, ctx.sampleRate);
  const d = buf.getChannelData(0);
  for (let i = 0; i < n; i++) d[i] = Math.random() * 2 - 1;
  LU_NOISE_BUF = buf;
  return buf;
}

function luNoise(vol, dur, freq, when) {
  const ctx = luAudioReady();
  if (!ctx || !LUG.soundOn || !LUG.audio.sx) return;
  const t = (when != null && when > ctx.currentTime) ? when : ctx.currentTime;
  const src = ctx.createBufferSource();
  src.buffer = luNoiseBuf(ctx);
  const f = ctx.createBiquadFilter();
  f.type = 'bandpass'; f.frequency.value = freq || 900; f.Q.value = 1.1;
  const g = ctx.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(Math.max(0.001, vol), t + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(f); f.connect(g); g.connect(LUA.sfxBus);
  src.start(t); src.stop(t + dur + 0.02);
}

/**
 * Play a named sound effect. Deduplicates identical kinds inside
 * LU_SFX_GAP ms (rapid steps / double-triggered events collapse to one)
 * and composes each cue from a few scheduled tones/noise hits so every
 * game event has its own recognisable voice. All cues are short and
 * quiet by design (peak ~0.16) — polished, never intrusive.
 */
function luSfx(kind) {
  if (!LUG.soundOn || !LUG.audio.sx) return;
  const now = performance.now();
  if (LUA.lastPlay[kind] && now - LUA.lastPlay[kind] < LU_SFX_GAP) return;
  LUA.lastPlay[kind] = now;
  const ctx = luAudioReady();
  if (!ctx) return;
  const T = ctx.currentTime;

  switch (kind) {
    /* dice tumble: 3 wooden clacks rising in pitch + a settle tick */
    case 'roll':
      [0, 90, 185].forEach((ms, i) =>
        luNoise(0.10, 0.05, 700 + i * 260, T + ms / 1000));
      luTone(320, 0.03, 0.05, 'triangle', 240, T + 0.24);
      break;
    /* one square hop: soft woody tick (quiet — fires every hop) */
    case 'step':
      luTone(292, 0.035, 0.05, 'sine', 250);
      break;
    /* legal token picked: bright affirmative blip */
    case 'pick':
      luTone(660, 0.05, 0.08, 'triangle');
      luTone(880, 0.06, 0.07, 'triangle', null, T + 0.05);
      break;
    /* released from yard onto the board: upward pair */
    case 'enter':
      luTone(392, 0.08, 0.10, 'triangle');
      luTone(523, 0.09, 0.10, 'triangle', null, T + 0.08);
      break;
    /* capture: low thud + descending snarl + impact noise */
    case 'capture':
      luNoise(0.12, 0.09, 420);
      luTone(330, 0.14, 0.13, 'sawtooth', 110);
      luTone(196, 0.16, 0.10, 'square', 98, T + 0.06);
      break;
    /* safe star landing: gentle warm chime */
    case 'safe':
      luTone(784, 0.09, 0.08, 'sine');
      luTone(1047, 0.11, 0.07, 'sine', null, T + 0.07);
      break;
    /* home-lane entry: two-note swell in the player's colour mood */
    case 'lane':
      luTone(440, 0.10, 0.08, 'sine');
      luTone(554, 0.12, 0.08, 'sine', null, T + 0.09);
      break;
    /* token reached home: rising triad */
    case 'finish':
      [523, 659, 784].forEach((f, i) =>
        luTone(f, 0.10, 0.11, 'triangle', null, T + i * 0.085));
      break;
    /* six rolled: sparkle pair up an octave */
    case 'six':
      luTone(880, 0.07, 0.10, 'triangle');
      luTone(1175, 0.09, 0.10, 'triangle', null, T + 0.08);
      break;
    /* victory fanfare: major arpeggio + high bell */
    case 'win':
      luMusicDuck(2.2);                              // music yields
      [523, 659, 784, 1047].forEach((f, i) =>
        luTone(f, 0.12, 0.12, 'triangle', null, T + i * 0.10));
      luTone(1319, 0.30, 0.10, 'sine', null, T + 0.42);
      break;
    /* AI won: descending minor cadence (soft, not punishing) */
    case 'over':
      luMusicDuck(2.2);
      [392, 311, 262, 196].forEach((f, i) =>
        luTone(f, 0.15, 0.11, 'triangle', null, T + i * 0.13));
      break;
    /* invalid move: single muted buzz, very short */
    case 'invalid':
      luTone(180, 0.08, 0.06, 'square', 150);
      break;
    /* generic UI button press */
    case 'ui':
      luTone(880, 0.03, 0.05, 'sine', 780);
      break;
  }
}

/* ---- background music (generative ambient loop) --------------------------
   A slow 4-chord pad progression with a sparse bell melody, scheduled on
   a lookahead timer (standard Web Audio scheduling pattern — the timer
   only BOOKS notes ahead; nothing runs per animation frame). Seamless by
   construction: the progression loops forever with no seam to crossfade.
   Volume lives on LUA.musicBus; toggling mutes instantly via the graph. */

const LU_MUSIC_CHORDS = [
  [220.00, 277.18, 329.63],   // A minor-ish pad
  [196.00, 246.94, 293.66],   // G
  [174.61, 220.00, 261.63],   // F
  [196.00, 246.94, 293.66],   // G
];
const LU_MUSIC_BELLS = [659.25, 587.33, 523.25, 587.33];

/** Start the music scheduler if (and only if) settings allow and the
 *  game is in an active phase. Idempotent: a second call while running
 *  is a no-op, so restart churn can never stack timers. */
function luMusicStart() {
  if (LUA.musicTimer) return;                       // already playing
  if (!LUG.audio.mu) return;                        // music muted
  /* Autoplay policy: music must never CREATE the context — only a real
     user gesture may (Phase 5.4 contract, re-asserted in 5.5 where
     luNewGame now runs programmatically at mount). If the context is
     missing or still suspended, skip; the next gesture-owned start
     (roll click, settings change) picks music up. */
  if (!LUA.ctx || LUA.ctx.state !== 'running') return;
  const ctx = LUA.ctx;                              // captured above (running)
  LUA.music = { step: 0, nextT: ctx.currentTime + 0.1, duckUntil: 0 };
  const tick = () => {
    LUA.musicTimer = 0;                             // own the slot again
    if (!LUA.music || !LUG.audio.mu) { LUA.music = null; return; }
    if (!LUA.ctx || LUA.ctx.state !== 'running') { LUA.music = null; return; }
    const m = LUA.music, ct = ctx.currentTime;
    /* book every chord whose start time enters the lookahead window */
    while (m.nextT < ct + LU_MUSIC_LOOKAHEAD) {
      const chord = LU_MUSIC_CHORDS[m.step % LU_MUSIC_CHORDS.length];
      const barDur = 3.2;                           // seconds per chord
      /* pad: three detuned sines per chord, slow attack/release */
      chord.forEach((f, ci) => {
        luPadNote(f, m.nextT, barDur * 1.05);
        if (ci === 0) luPadNote(f * 0.5, m.nextT, barDur * 1.05); // root octave down
      });
      /* sparse bell on beats 1 and 3 of the bar */
      luBell(LU_MUSIC_BELLS[m.step % LU_MUSIC_BELLS.length], m.nextT + 0.4);
      luBell(LU_MUSIC_BELLS[(m.step + 2) % LU_MUSIC_BELLS.length], m.nextT + 1.9);
      m.step++;
      m.nextT += barDur;
    }
    LUA.musicTimer = setTimeout(tick, 350);         // re-arm scheduler
  };
  LUA.musicTimer = setTimeout(tick, 50);
}

/** Stop the music and drop every pending booking. */
function luMusicStop() {
  if (LUA.musicTimer) { clearTimeout(LUA.musicTimer); LUA.musicTimer = 0; }
  LUA.music = null;
}

/** Temporarily lower music while a stinger plays (win/over). */
function luMusicDuck(sec) {
  if (!LUA.musicBus || !LUA.ctx) return;
  const t = LUA.ctx.currentTime;
  LUA.musicBus.gain.cancelScheduledValues(t);
  LUA.musicBus.gain.setTargetAtTime(LUG.audio.mu ? LUG.audio.mv * 0.18 : 0, t, 0.05);
  LUA.musicBus.gain.setTargetAtTime(LUG.audio.mu ? LUG.audio.mv : 0, t + sec, 0.4);
}

/** One sustained pad note on the music bus (long attack = unobtrusive). */
function luPadNote(freq, when, dur) {
  const ctx = LUA.ctx; if (!ctx || !LUA.musicBus) return;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = 'sine';
  osc.frequency.value = freq * (1 + (Math.random() - 0.5) * 0.004); // chorus detune
  g.gain.setValueAtTime(0.0001, when);
  g.gain.linearRampToValueAtTime(0.045, when + dur * 0.35);
  g.gain.linearRampToValueAtTime(0.0001, when + dur);
  osc.connect(g); g.connect(LUA.musicBus);
  osc.start(when); osc.stop(when + dur + 0.05);
}

/** One soft bell pluck on the music bus. */
function luBell(freq, when) {
  const ctx = LUA.ctx; if (!ctx || !LUA.musicBus) return;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = 'triangle';
  osc.frequency.value = freq;
  g.gain.setValueAtTime(0.0001, when);
  g.gain.exponentialRampToValueAtTime(0.05, when + 0.02);
  g.gain.exponentialRampToValueAtTime(0.0001, when + 1.1);
  osc.connect(g); g.connect(LUA.musicBus);
  osc.start(when); osc.stop(when + 1.2);
}

/** Music follows game life: start on game activity, stop when idle/over
 *  or the view goes away. Called from the same places that already
 *  manage the FX pumps, so it inherits their guards. */
function luMusicFollow() {
  const active = LUG.game && LUG.game.winner === null &&
    ['idle', 'rolling', 'awaiting', 'moving'].includes(LUG.phase);
  if (active && LUG.viewAlive()) luMusicStart();
  else luMusicStop();
}

/** Full audio teardown: stop music, close the context ONCE (the closed
 *  guard prevents the double-close throw on repeated teardowns). */
function luAudioTeardown() {
  luMusicStop();
  if (LUA.ctx) {
    try { LUA.ctx.close(); } catch { /* already closed */ }
  }
  LUA.ctx = null; LUA.master = null; LUA.sfxBus = null; LUA.musicBus = null;
  LUA.inited = false; LUA.lastPlay = {};
  LU_NOISE_BUF = null;                              // buffer belonged to ctx
}


const lu$ = id => document.getElementById(id);
function luOn(target, ev, fn, opts) {
  target.addEventListener(ev, fn, opts);
  LUG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
}

/* ---- audio -------------------------------------------------------------
   The full Phase 5.4 audio system (context, buses, SFX synth, music
   scheduler, persistence) lives above in the LUA block; these aliases
   keep the historical names working for any external callers.        */
const luAudioInit = luAudioReady;

/* =====================================================================
   Phase 5.3 — VISUAL FX LAYER
   Self-expiring canvas effects (impact rings, safe pulses, home bursts,
   six flares). Rules of the layer:
     • every effect is a plain record in LUG.fx with an absolute t0/dur;
       nothing writes engine state and nothing registers listeners;
     • ONE rAF pump (LUG.fxRaf) redraws while any effect is alive; it
       captures the game clock + view epoch and dies on restart/teardown/
       route-away exactly like the glide chain, so no stale loop can
       survive a lifecycle event;
     • the pump only calls luDraw() — all painting happens inside the
       normal draw pass via luDrawFx(), keeping one render path.
   ===================================================================== */

/** Spawn an effect. `o` must include {kind, x, y}; duration defaults
 *  live in LU_FX_DUR. Cheap by design: at most a handful alive at once. */
function luFx(o) {
  if (!LUG.viewAlive()) return;
  const dur = LU_FX_DUR[o.kind] || 400;
  LUG.fx.push({ ...o, t0: performance.now(), dur });
  if (LUG.fx.length > 24) LUG.fx.splice(0, LUG.fx.length - 24);  // clamp
  luFxPump();
}

const LU_FX_DUR = {
  impact: 420,   // capture hit ring
  safe:   500,   // landing on a star square
  lane:   550,   // entering the private home lane
  finish: 650,   // a token reaching the centre
  six:    600,   // rolled a six (on the roller's die)
  fly:    450,   // captured token ghost flying to its yard
  win:    900,   // game-winner bloom
};

/** Single rAF pump: runs while ≥1 effect is unexpired, then stops.
 *  Guarded by clock AND epoch — a restart or teardown mid-effect kills
 *  it on the next frame (effects are visual only, so dropping them is
 *  always safe). */
function luFxPump() {
  if (LUG.fxRaf) return;                     // already pumping
  const my = LUG.clock, ep = LUG.epoch;
  const tick = () => {
    LUG.fxRaf = 0;
    if (my !== LUG.clock || ep !== LUG.epoch || !LUG.viewAlive()) {
      LUG.fx = [];                            // dead view: drop everything
      return;
    }
    const now = performance.now();
    LUG.fx = LUG.fx.filter(f => now - f.t0 < f.dur);
    if (LUG.fx.length) {
      luDraw();
      LUG.fxRaf = requestAnimationFrame(tick);
    } else if (LUG.game) {
      luDraw();                               // final clean frame
    }
  };
  LUG.fxRaf = requestAnimationFrame(tick);
}

/** Stop the pump and discard effects (restart / teardown / route-away). */
function luFxClear() {
  if (LUG.fxRaf) { cancelAnimationFrame(LUG.fxRaf); LUG.fxRaf = 0; }
  LUG.fx = [];
}

/** Paint all live effects. Called from luDraw() AFTER tokens so the
 *  rings/bursts read as overlays. Positions are stored in CANVAS units
 *  and scale with the board on resize (recomputed nowhere — they are
 *  short-lived, so a mid-effect resize just re-scales the canvas). */
function luDrawFx() {
  const ctx = LUG.ctx; if (!ctx) return;
  const now = performance.now();
  const C = LUG.cell;
  for (const f of LUG.fx) {
    const k = Math.min(1, (now - f.t0) / f.dur);      // 0→1 progress
    const fade = 1 - k;
    switch (f.kind) {
      case 'impact': {
        /* capture: expanding double ring + flash core */
        const R = C * (0.30 + 0.85 * k);
        ctx.globalAlpha = fade * 0.9;
        ctx.beginPath(); ctx.arc(f.x, f.y, R, 0, Math.PI * 2);
        ctx.strokeStyle = '#ffffff'; ctx.lineWidth = 2.5;
        ctx.stroke();
        ctx.beginPath(); ctx.arc(f.x, f.y, R * 0.6, 0, Math.PI * 2);
        ctx.strokeStyle = f.col || '#ffd166'; ctx.lineWidth = 2;
        ctx.stroke();
        if (k < 0.35) {
          ctx.globalAlpha = (1 - k / 0.35) * 0.5;
          ctx.beginPath(); ctx.arc(f.x, f.y, C * 0.32, 0, Math.PI * 2);
          ctx.fillStyle = '#ffffff'; ctx.fill();
        }
        break;
      }
      case 'safe': {
        /* star square: soft gold pulse ring */
        ctx.globalAlpha = fade * 0.7;
        ctx.beginPath(); ctx.arc(f.x, f.y, C * (0.28 + 0.42 * k), 0, Math.PI * 2);
        ctx.strokeStyle = '#ffd166'; ctx.lineWidth = 2;
        ctx.stroke();
        break;
      }
      case 'lane': {
        /* home-lane entry: colour-tinted inward glow */
        ctx.globalAlpha = fade * 0.55;
        const g = ctx.createRadialGradient(f.x, f.y, 0, f.x, f.y, C * (0.4 + 0.5 * k));
        g.addColorStop(0, f.col || '#5ef2c0');
        g.addColorStop(1, 'rgba(0,0,0,0)');
        ctx.fillStyle = g;
        ctx.beginPath(); ctx.arc(f.x, f.y, C * (0.4 + 0.5 * k), 0, Math.PI * 2);
        ctx.fill();
        break;
      }
      case 'finish': {
        /* token finished: bright bloom + tiny sparkle cross */
        ctx.globalAlpha = fade * 0.85;
        const R = C * (0.35 + 0.75 * k);
        ctx.beginPath(); ctx.arc(f.x, f.y, R, 0, Math.PI * 2);
        ctx.strokeStyle = '#f2f7ff'; ctx.lineWidth = 2.5;
        ctx.stroke();
        ctx.globalAlpha = fade * 0.6;
        const s = C * 0.5 * (0.4 + 0.6 * k);
        ctx.strokeStyle = '#ffffff'; ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(f.x - s, f.y); ctx.lineTo(f.x + s, f.y);
        ctx.moveTo(f.x, f.y - s); ctx.lineTo(f.x, f.y + s);
        ctx.stroke();
        break;
      }
      case 'six': {
        /* six flare around the seat's die (canvas coords) */
        ctx.globalAlpha = fade * 0.8;
        const R = C * (0.25 + 0.45 * k);
        ctx.beginPath(); ctx.arc(f.x, f.y, R, 0, Math.PI * 2);
        ctx.strokeStyle = f.col || '#ffd166'; ctx.lineWidth = 3;
        ctx.setLineDash([5, 4]);
        ctx.stroke();
        ctx.setLineDash([]);
        break;
      }
      case 'fly': {
        /* captured token ghost: arcs from the capture square to its
           yard slot, shrinking + fading. The REAL token is already in
           the yard (engine committed); this is a readable echo only. */
        const e = 1 - Math.pow(1 - k, 2);              // easeOutQuad
        const x = f.x + (f.tx - f.x) * e;
        const y = f.y + (f.ty - f.y) * e - Math.sin(Math.PI * e) * C * 0.5;
        ctx.globalAlpha = fade * 0.85;
        const R = C * 0.30 * (1 - 0.45 * k);
        const g2 = ctx.createLinearGradient(x - R, y - R, x + R, y + R);
        g2.addColorStop(0, f.col || '#ff5d5d');
        g2.addColorStop(1, 'rgba(0,0,0,.4)');
        ctx.beginPath(); ctx.arc(x, y, R, 0, Math.PI * 2);
        ctx.fillStyle = g2; ctx.fill();
        ctx.lineWidth = 1.5;
        ctx.strokeStyle = 'rgba(255,255,255,.5)';
        ctx.stroke();
        break;
      }
      case 'win': {
        /* winner bloom at the centre */
        ctx.globalAlpha = fade * 0.75;
        const R = C * (0.8 + 2.2 * k);
        const g = ctx.createRadialGradient(f.x, f.y, R * 0.3, f.x, f.y, R);
        g.addColorStop(0, f.col || '#ffd166');
        g.addColorStop(1, 'rgba(0,0,0,0)');
        ctx.fillStyle = g;
        ctx.beginPath(); ctx.arc(f.x, f.y, R, 0, Math.PI * 2);
        ctx.fill();
        break;
      }
    }
    ctx.globalAlpha = 1;
  }
}


/* ---- palette ----------------------------------------------------------- */
const LU_COLORS = {
  red:    { fill: '#ff5d5d', dark: '#c93030', glow: 'rgba(255,93,93,.45)', soft: '#ff5d5d' },
  green:  { fill: '#5ef2c0', dark: '#2ea886', glow: 'rgba(94,242,192,.45)', soft: '#5ef2c0' },
  yellow: { fill: '#ffd166', dark: '#c99a2e', glow: 'rgba(255,209,102,.45)', soft: '#ffd166' },
  blue:   { fill: '#6ea8ff', dark: '#3671c9', glow: 'rgba(110,168,255,.45)', soft: '#6ea8ff' },
};
const LU_P = ['red', 'green', 'yellow', 'blue'];

/* ==========================================================================
   Phase 5.5 — PLAYER NAMES
   --------------------------------------------------------------------------
   Display names are a pure UI layer: the engine keeps its own NAMES
   (Red/Green/Yellow/Blue) and is NEVER modified. luSeatName(i) resolves
   what the UI shows for seat i:
     • human seat 0            → "You" unless a custom name is set
     • other human seats       → custom name, else "Player 2/3/4"
     • AI seats                → "<Colour> <Level>" e.g. "Green · Hard",
                                 or just the colour for Classic
   Validation: trimmed, collapsed whitespace, hard length cap (14). A
   name that fails validation falls back to the default — never an empty
   or layout-breaking string.
   ========================================================================== */
const LU_NAME_MAX = 14;

/** Sanitise a candidate name → safe display string or '' (use default). */
function luCleanName(raw) {
  if (typeof raw !== 'string') return '';
  const s = raw.replace(/\s+/g, ' ').trim().slice(0, LU_NAME_MAX);
  return s;
}

/** Default display name for seat i given the current configuration. */
function luDefaultName(i) {
  const active = luSeatsForCount(LUG.seatCount);
  const at = active.indexOf(i);
  if (at === -1) return LudoEngine.NAMES[i];           // inactive seat
  const kind = luKindsForConfig(LUG.seatCount, LUG.humanSeats)[i];
  if (kind === 'human') {
    if (at === 0) return 'You';                        // first human
    return `Player ${at + 1}`;                         // 2nd/3rd/4th human
  }
  const lvl = LUG.aiDifficulty
    ? LudoEngine.NAMES[i] + ' · ' + LUG.aiDifficulty[0].toUpperCase()
        + LUG.aiDifficulty.slice(1)
    : LudoEngine.NAMES[i];
  return lvl;
}

/** Resolved display name for seat i (custom if valid, else default).
 *  Always returns a non-empty, length-capped, HTML-escaped string. */
function luSeatName(i) {
  const custom = luCleanName(LUG.names && LUG.names[i]);
  const name = custom || luDefaultName(i);
  /* escape: names render into innerHTML in panels/messages */
  return String(name)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

/** Raw (unescaped) resolved name — for aria-labels built with
 *  textContent-equivalent semantics. */
function luSeatNameRaw(i) {
  const custom = luCleanName(LUG.names && LUG.names[i]);
  return custom || luDefaultName(i);
}

/* Phase 5.1 — premium board palette. All board chrome colors in one
   place so contrast stays consistent across every element. */
const LU_BOARD = {
  plate:     '#0b1220',   // outer board plate
  plateEdge: '#233452',   // plate rim highlight
};

/* ---- canvas sizing ------------------------------------------------------ */
function luResize() {
  const c = LUG.canvas; if (!c) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  /* Square board: it must fit BOTH the stage's width and its height.
     The playing session locks page scroll (body.ft-playing{overflow:hidden}),
     so a width-only fit would grow taller than the viewport and clip.
     Measured from the STAGE, never the canvas itself — the canvas's own
     rect is whatever the last fit wrote inline (style.width/height), so
     reading it back would freeze the board at its first size forever:
     it could shrink on window shrink but NEVER grow again, leaving a
     small board centered in a large stage (and in fullscreen). Same rule
     as snResize() in Snakes & Ladders. */
  const wrap = c.parentElement;                       // .lu-stage
  const availW = Math.max(300, (wrap && wrap.clientWidth) || c.getBoundingClientRect().width || 640);
  /* Stage height: a COLLAPSED stage (clientHeight 0–2px, e.g. mobile
     landscape where the control rows consume the panel) must NOT fall
     back to the stage width — that sizes the board by width alone and
     it overflows the viewport. When the stage has no usable height,
     fit the board to the VIEWPORT height minus the panel's non-stage
     chrome instead (Phase 5.1 landscape fix).
     RESPONSIVE FIX · the height budget is anchored to the VIEWPORT
     (window.innerHeight), never the panel's own box: in the CSS-immersive
     fallback the panel's height is content-driven, so subtracting chrome
     from it created a shrink feedback loop that collapsed the board to
     ~131px on phones. The viewport is the true budget in native
     fullscreen, immersive and inline play alike. */
  let availH;
  const stageH = wrap ? wrap.clientHeight : 0;
  const panel = wrap && wrap.parentElement;
  const measureChrome = () => {
    let chrome = 0;
    if (panel) {
      for (const el of panel.children) {
        if (el === wrap) continue;
        /* Overlays (#luLobby, #luAudioPop) are absolutely positioned and
           do NOT consume layout height — counting them starved the stage
           to the 120px floor while the lobby was open (the setup card is
           not chrome; it covers the stage). */
        const st = getComputedStyle(el);
        if (st.position === 'absolute' || st.position === 'fixed') continue;
        chrome += el.getBoundingClientRect().height;
      }
    }
    return chrome;
  };
  if (stageH >= 100) {
    /* Normal case: the stage flexed open. But in short landscape
       viewports the stage only gets the height LEFT OVER after the
       board's first (too-large) placement — a chicken-and-egg that
       starves it. Cap by the viewport minus measured chrome so the
       board never exceeds what actually fits (Phase 5.1 fix).
       PHASE 1 · measure the panel's REAL padding and inter-child gaps
       instead of hardcoded constants — the compact landscape rules use
       14px padding / 4px gaps, and stale 36+20 estimates under-sized
       the board by ~40px there. */
    const chrome = measureChrome();
    const pcs = getComputedStyle(panel);
    let extras = parseFloat(pcs.paddingTop) + parseFloat(pcs.paddingBottom);
    const gapPx = parseFloat(pcs.rowGap || pcs.gap || '0');
    /* Count only IN-FLOW children — absolutely-positioned overlays and
       zero-height rows do not participate in the flex gap chain. */
    let inflow = 0;
    for (const el of panel.children) {
      const cs2 = getComputedStyle(el);
      if (cs2.position === 'absolute' || cs2.position === 'fixed') continue;
      if (el.getBoundingClientRect().height <= 0) continue;
      inflow++;
    }
    extras += Math.max(0, inflow - 1) * gapPx;
    const fitH = window.innerHeight - chrome - extras - 4 /* stage margin slack */;
    availH = Math.min(stageH, Math.max(fitH, 120));
  } else {
    /* collapsed stage: estimate from viewport minus siblings' heights */
    const chrome = measureChrome();
    const pcs = panel ? getComputedStyle(panel) : null;
    let extras = 24;                                  // conservative default
    if (pcs) {
      extras = parseFloat(pcs.paddingTop) + parseFloat(pcs.paddingBottom);
      const gapPx = parseFloat(pcs.rowGap || pcs.gap || '0');
      extras += Math.max(0, panel.children.length - 1) * gapPx;
    }
    availH = Math.max(300, window.innerHeight - chrome - extras - 4);
  }
  const cssW = Math.floor(Math.min(availW, availH));
  const cssH = cssW;                                  // square board
  c.style.width = cssW + 'px';
  c.style.height = cssH + 'px';
  c.width = Math.round(cssW * dpr);
  c.height = Math.round(cssH * dpr);
  LUG.w = c.width; LUG.h = c.height;
  LUG.cell = LUG.w / 15;
  luDraw();
}

/* ---- geometry helpers ---------------------------------------------------- */
/** Centre of a grid cell in device pixels. */
function luCenter(row, col) {
  return { x: (col + 0.5) * LUG.cell, y: (row + 0.5) * LUG.cell };
}
/** Pixel centre for any logical position of player p's token i. */
function luPosCenter(p, pos, i) {
  const E = window.LudoEngine;
  if (pos === E.FINISH) return luFinishSlot(p, i || 0);
  const rc = E.cellOf(p, pos);
  return rc ? luCenter(rc[0], rc[1]) : null;
}
/**
 * Finished-token slot inside the centre pool: one 2×2 cluster per
 * player in the quadrant matching that player's yard (r TL, g TR,
 * y BR, b BL). Cluster spacing keeps neighbouring players' tokens
 * clear of each other while staying inside the 3-cell centre pool.
 */
function luFinishSlot(p, i) {
  const c = luCenter(7, 7);
  const C = LUG.cell;
  const q = C * 0.78;                       // quadrant centre offset
  const s = C * 0.38;                       // stagger inside the cluster
  const qx = [-q, q, q, -q][p];
  const qy = [-q, -q, q, q][p];
  return { x: c.x + qx + (i % 2 ? s : -s), y: c.y + qy + (i > 1 ? s : -s) };
}
/** Yard slot pixel centre for token i of player p. */
function luYardCenter(p, i) {
  const E = window.LudoEngine;
  const rc = E.YARD_SLOTS[p][i];
  return luCenter(rc[0], rc[1]);
}

/**
 * Interpolated pixel point for a GLIDING token. During the rAF walk
 * `_viz` is a fractional {from, to, t} object (render-only); the token
 * is drawn part-way between the two logical steps with an ease-in-out
 * curve so each square starts and ends at rest. Endpoints are derived
 * from `from`/`to` on EVERY frame (not cached), so a window resize
 * mid-glide re-scales correctly. Covers yard entry (from -1), the
 * shared track, home lanes and the finish pool.
 *
 * Phase 5.3: adds a restrained HOP arc — the token lifts off, peaks at
 * mid-hop and lands on the next square (parabola over the eased t).
 * The offset is purely vertical and render-only; `_viz.t` semantics,
 * engine commit and hit-test geometry are unchanged (hit-testing never
 * consults a moving token's fractional position for legality).
 */
function luGlidePoint(p, i, viz) {
  const A = viz.from === -1 ? luYardCenter(p, i) : luPosCenter(p, viz.from, i);
  const B = luPosCenter(p, viz.to, i);
  if (!A || !B) return A || B || luYardCenter(p, i);
  const t = Math.max(0, Math.min(1, viz.t));
  const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2; // easeInOutQuad
  /* hop: sin curve peaks (h=1) exactly at mid-hop, zero at both ends —
     no jump on take-off or landing, no orientation change (vertical
     lift only). Amplitude scales with the cell so it survives resizes. */
  let hop = 0;
  if (!LUG.reducedMotion) hop = Math.sin(Math.PI * e) * LUG.cell * 0.16;
  return { x: A.x + (B.x - A.x) * e, y: A.y + (B.y - A.y) * e - hop };
}

/* ---- drawing -------------------------------------------------------------- */
function luDraw() {
  const ctx = LUG.ctx; if (!ctx) return;
  const E = window.LudoEngine;
  const W = LUG.w, C = LUG.cell;
  const g = LUG.game;                 // seat activity lookups below
  ctx.clearRect(0, 0, W, W);

  /* ---- board plate (Phase 5.1 premium frame) ----
     Subtle radial vignette + rim so the board reads as a physical
     object rather than a flat dark rectangle. */
  ctx.fillStyle = LU_BOARD.plate;
  ctx.fillRect(0, 0, W, W);
  const vg = ctx.createRadialGradient(W / 2, W / 2, W * 0.18,
                                      W / 2, W / 2, W * 0.72);
  vg.addColorStop(0, 'rgba(30,48,80,.38)');
  vg.addColorStop(1, 'rgba(4,8,16,.42)');
  ctx.fillStyle = vg;
  ctx.fillRect(0, 0, W, W);
  ctx.strokeStyle = LU_BOARD.plateEdge;
  ctx.lineWidth = Math.max(2, C * 0.10);
  ctx.strokeRect(ctx.lineWidth / 2, ctx.lineWidth / 2,
                 W - ctx.lineWidth, W - ctx.lineWidth);

  /* --- yards (6×6 quadrants): colour wash + inner pedestal ring --- */
  const quad = [[0, 0], [0, 9], [9, 9], [9, 0]];
  quad.forEach((q, p) => {
    const [r0, c0] = q;
    const col = LU_COLORS[LU_P[p]];
    const on = !g || LudoEngine.isActive(g, p);
    ctx.globalAlpha = on ? 1 : 0.22;

    /* outer colour wash */
    ctx.fillStyle = col.fill + '1f';
    roundRect(ctx, c0 * C + C * 0.25, r0 * C + C * 0.25, 6 * C - C * 0.5, 6 * C - C * 0.5, C * 0.55);
    ctx.fill();
    ctx.strokeStyle = col.fill + '59';
    ctx.lineWidth = 2;
    roundRect(ctx, c0 * C + C * 0.25, r0 * C + C * 0.25, 6 * C - C * 0.5, 6 * C - C * 0.5, C * 0.55);
    ctx.stroke();

    /* inner pedestal: rounded platform the four tokens sit on */
    ctx.fillStyle = 'rgba(10,16,28,.55)';
    roundRect(ctx, c0 * C + C * 1.15, r0 * C + C * 1.15,
              6 * C - C * 2.3, 6 * C - C * 2.3, C * 0.45);
    ctx.fill();
    ctx.strokeStyle = col.fill + '40';
    ctx.lineWidth = 1.5;
    roundRect(ctx, c0 * C + C * 1.15, r0 * C + C * 1.15,
              6 * C - C * 2.3, 6 * C - C * 2.3, C * 0.45);
    ctx.stroke();

    ctx.globalAlpha = 1;
  });

  /* --- track cells --- */
  for (let t = 0; t < 52; t++) {
    const [r, c] = E.TRACK[t];
    const x = c * C, y = r * C;
    const gl = E.globalOf(0, t);           // global index of this cell

    /* cell base with slight top-light bevel */
    ctx.fillStyle = '#141e33';
    ctx.fillRect(x + 1, y + 1, C - 2, C - 2);
    ctx.fillStyle = 'rgba(255,255,255,.035)';
    ctx.fillRect(x + 1, y + 1, C - 2, (C - 2) * 0.5);   // upper half sheen
    ctx.strokeStyle = '#26354f';
    ctx.lineWidth = 1;
    ctx.strokeRect(x + 1.5, y + 1.5, C - 3, C - 3);

    /* start squares tinted with their owner colour (FIX Phase 5.1:
       this loop previously tested START_G.indexOf(g) against the GAME
       OBJECT `g` — variable shadowing meant tints never rendered). */
    const owner = E.START_G.indexOf(gl);
    if (owner >= 0) {
      ctx.fillStyle = LU_COLORS[LU_P[owner]].fill + '38';
      ctx.fillRect(x + 1, y + 1, C - 2, C - 2);
      ctx.strokeStyle = LU_COLORS[LU_P[owner]].fill + '80';
      ctx.lineWidth = 1.5;
      ctx.strokeRect(x + 1.5, y + 1.5, C - 3, C - 3);
    }

    /* stars on safe squares (same shadowing fix: SAFE_G.has(gl)) */
    if (E.SAFE_G.has(gl)) {
      luStar(x + C / 2, y + C / 2, C * 0.24, '#93a9c8');
      /* faint halo behind the star for at-a-glance recognition */
      ctx.beginPath();
      ctx.arc(x + C / 2, y + C / 2, C * 0.34, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(147,169,200,.10)';
      ctx.fill();
    }
  }

  /* --- home lanes: stronger colour + direction chevrons --- */
  for (let p = 0; p < 4; p++) {
    const col = LU_COLORS[LU_P[p]];
    /* Inactive seats keep their geometry but render faded (Phase 4.1). */
    const seatOn = !g || LudoEngine.isActive(g, p);
    ctx.globalAlpha = seatOn ? 1 : 0.22;
    E.LANES[p].forEach(([r, c]) => {
      ctx.fillStyle = col.fill + '33';
      ctx.fillRect(c * C + 1, r * C + 1, C - 2, C - 2);
      ctx.strokeStyle = col.fill + '7d';
      ctx.lineWidth = 1.5;
      ctx.strokeRect(c * C + 1.5, r * C + 1.5, C - 3, C - 3);
    });
    /* arrow head pointing into the lane from its entry edge */
    const first = E.LANES[p][0];
    luArrowHead(first[0], first[1], p, col);
    ctx.globalAlpha = 1;                       // restore for the next seat
  }

  /* --- centre finish pool: colour wedges + white core star --- */
  const cx = 7.5 * C, cy = 7.5 * C;
  const tri = [
    [cx, cy - 3 * C], [cx + 3 * C, cy], [cx, cy + 3 * C], [cx - 3 * C, cy],
  ];
  tri.forEach((pt, i) => {
    const prev = tri[(i + 3) % 4], next = tri[(i + 1) % 4];
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(prev[0], prev[1]);
    ctx.lineTo(next[0], next[1]);
    ctx.closePath();
    ctx.globalAlpha = (!g || LudoEngine.isActive(g, i)) ? 1 : 0.22;
    ctx.fillStyle = LU_COLORS[LU_P[i]].fill + '3a';
    ctx.fill();
    ctx.strokeStyle = LU_COLORS[LU_P[i]].fill + '8c';
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.globalAlpha = 1;
  });
  /* soft glow core so finished tokens read as "home" */
  const cg = ctx.createRadialGradient(cx, cy, 0, cx, cy, C * 1.5);
  cg.addColorStop(0, 'rgba(240,246,255,.22)');
  cg.addColorStop(1, 'rgba(240,246,255,0)');
  ctx.fillStyle = cg;
  ctx.beginPath();
  ctx.arc(cx, cy, C * 1.5, 0, Math.PI * 2);
  ctx.fill();

  /* --- tokens --- */
  if (g) {
    /* Stack map is computed ONCE per frame and drives both the fan-out
       below and the hit-test radius — a single source of truth for
       stacked-token geometry. */
    const stacks = luBuildStacks();

    for (let p = 0; p < 4; p++) {
      const seatOn = LudoEngine.isActive(g, p);
      for (let i = 0; i < 4; i++) {
        const tok = g.players[p].tokens[i];
        const pos = tok._viz != null ? tok._viz : tok.pos;
        const pt = luTokenPoint(p, i, stacks);
        if (!pt) continue;
        ctx.globalAlpha = seatOn ? 1 : 0.28;   // parked inactive-seat tokens

        /* radius scales with stack size so everyone fits in the cell */
        let R = C * 0.36;
        const skey = luStackKey(p, pos);
        if (skey !== null) {
          const mates = stacks.get(skey) || [];
          if (mates.length > 1) R = luStackRadius(mates.length);
        }

        /* legal-move highlight ring */
        const isLegal = LUG.phase === 'awaiting' && LUG.moves.some(
          m => m.token === i && LUG.game.turn === p
        );
        const isAnim = LUG.anim && LUG.anim.player === p && LUG.anim.token === i;
        /* illegal-click nudge flash (visual only, never state) */
        const nudged = !!(LUG.nudge && LUG.nudge.p === p && LUG.nudge.i === i &&
          performance.now() < LUG.nudge.until);
        /* Phase 5.3: hover-selected feedback — the token under the
           pointer while it is a legal pick gets a bright keyline.
           LUG.hoverToken is set by the pointermove handler and is
           purely presentational (never consulted by hit-testing). */
        const hovered = !!(isLegal && LUG.hoverToken &&
          LUG.hoverToken.p === p && LUG.hoverToken.i === i);

        luToken(pt.x, pt.y, R, LU_COLORS[LU_P[p]], isLegal, isAnim, p, nudged);
        if (hovered && !nudged) {
          ctx.beginPath();
          ctx.arc(pt.x, pt.y, R * 1.18, 0, Math.PI * 2);
          ctx.strokeStyle = '#ffffff';
          ctx.lineWidth = 2.5;
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      }
    }
  }

  /* Phase 5.3: transient overlays (capture rings, safe pulses, home
     bursts) painted last so they read above the tokens. */
  luDrawFx();
}

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function luStar(cx, cy, R, color) {
  const ctx = LUG.ctx;
  ctx.save();
  ctx.translate(cx, cy);
  ctx.beginPath();
  for (let i = 0; i < 8; i++) {
    const a = (Math.PI / 4) * i - Math.PI / 2;
    const rr = i % 2 === 0 ? R : R * 0.45;
    ctx.lineTo(Math.cos(a) * rr, Math.sin(a) * rr);
  }
  ctx.closePath();
  ctx.fillStyle = color + 'cc';
  ctx.fill();
  ctx.restore();
}

function luArrowHead(r, c, p, col) {
  /* chevron on the lane's first cell showing travel direction
     (Phase 5.1: slightly larger + outlined for visibility) */
  const ctx = LUG.ctx, C = LUG.cell;
  const cx = (c + 0.5) * C, cy = (r + 0.5) * C;
  const ang = [0, Math.PI / 2, Math.PI, -Math.PI / 2][p];  // r→ e, g→ s, y→ w, b→ n
  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(ang);
  ctx.beginPath();
  ctx.moveTo(C * 0.30, 0);
  ctx.lineTo(-C * 0.10, -C * 0.17);
  ctx.lineTo(-C * 0.02, 0);
  ctx.lineTo(-C * 0.10, C * 0.17);
  ctx.closePath();
  ctx.fillStyle = col.fill;
  ctx.fill();
  ctx.strokeStyle = 'rgba(255,255,255,.35)';
  ctx.lineWidth = 1;
  ctx.stroke();
  ctx.restore();
}

function luToken(x, y, R, col, legal, moving, p, nudged) {
  const ctx = LUG.ctx;
  const pulse = moving ? 1 + 0.12 * Math.sin(performance.now() / 90) : 1;

  if (legal) {
    /* breathing halo on movable tokens (Phase 5.1 ring, Phase 5.3:
       two-tier halo — outer soft fill + inner dashed keyline with a
       slow rotating dash offset so the "pick me" reads at a glance). */
    const breathe = 1 + 0.05 * Math.sin(performance.now() / 300);
    ctx.beginPath();
    ctx.arc(x, y, R * 1.55 * breathe, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(94,242,192,.16)';
    ctx.fill();
    ctx.beginPath();
    ctx.arc(x, y, R * 1.45 * breathe, 0, Math.PI * 2);
    ctx.strokeStyle = '#ffffffdd';
    ctx.setLineDash([4, 3]);
    ctx.lineDashOffset = -performance.now() / 140;   // gentle rotation
    ctx.lineWidth = 2;
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.lineDashOffset = 0;
  }

  /* Phase 5.3: illegal-click nudge — quick red flash ring that decays,
     distinct from the legal halo so rejection is unmistakable. */
  if (nudged) {
    const now = performance.now();
    const k = Math.max(0, Math.min(1, (LUG.nudge.until - now) / 700));
    if (k > 0) {
      ctx.globalAlpha = 0.35 + 0.65 * k;
      ctx.beginPath();
      ctx.arc(x, y, R * (1.25 + 0.35 * (1 - k)), 0, Math.PI * 2);
      ctx.strokeStyle = '#ff5d5d';
      ctx.lineWidth = 2.5;
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
  }

  /* drop shadow — two-layer for depth (soft ambient + tight contact) */
  ctx.beginPath();
  ctx.ellipse(x, y + R * 0.62, R * 0.95, R * 0.36, 0, 0, Math.PI * 2);
  ctx.fillStyle = 'rgba(0,0,0,.30)';
  ctx.fill();
  ctx.beginPath();
  ctx.ellipse(x, y + R * 0.48, R * 0.72, R * 0.26, 0, 0, Math.PI * 2);
  ctx.fillStyle = 'rgba(0,0,0,.38)';
  ctx.fill();

  /* body with rim (Phase 5.1: darker outer ring reads as a pawn edge) */
  const grd = ctx.createLinearGradient(x - R, y - R, x + R, y + R);
  grd.addColorStop(0, col.fill);
  grd.addColorStop(1, col.dark);
  ctx.beginPath();
  ctx.arc(x, y, R * pulse, 0, Math.PI * 2);
  ctx.fillStyle = grd;
  ctx.fill();
  /* rim: subtle dark edge then bright inner keyline */
  ctx.lineWidth = Math.max(1.5, R * 0.14);
  ctx.strokeStyle = 'rgba(8,12,22,.55)';
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(x, y, R * pulse * 0.88, 0, Math.PI * 2);
  ctx.strokeStyle = 'rgba(255,255,255,.34)';
  ctx.lineWidth = Math.max(1, R * 0.09);
  ctx.stroke();

  /* specular highlight — small bright arc upper-left */
  ctx.beginPath();
  ctx.arc(x, y, R * pulse * 0.70, Math.PI * 1.05, Math.PI * 1.55);
  ctx.strokeStyle = 'rgba(255,255,255,.42)';
  ctx.lineWidth = Math.max(1.2, R * 0.13);
  ctx.lineCap = 'round';
  ctx.stroke();
  ctx.lineCap = 'butt';

  /* inner dot */
  ctx.beginPath();
  ctx.arc(x, y, R * 0.32, 0, Math.PI * 2);
  ctx.fillStyle = 'rgba(255,255,255,.82)';
  ctx.fill();
}

/* ---- HUD ------------------------------------------------------------------ */
function luRenderHud() {
  const g = LUG.game; if (!g) return;

  /* ---- Phase 5.2: per-seat player panels ----
     One card per ACTIVE seat (inactive seats are not rendered — the
     board already fades their geometry). Counts are derived from token
     positions on every render, so they can never drift from engine
     state: yard = pos −1, track = 0..55 (shared + private lane),
     finished = FINISH. */
  const host = lu$('luPanels');
  if (host) {
    const E = window.LudoEngine;
    const diffLbl = LUG.aiDifficulty
      ? LUG.aiDifficulty[0].toUpperCase() + LUG.aiDifficulty.slice(1) : '';
    host.innerHTML = g.players.map((P, i) => {
      if (!LudoEngine.isActive(g, i)) return '';      // inactive → hidden
      let yard = 0, track = 0, done = 0;
      for (const t of P.tokens) {
        if (t.pos === -1) yard++;
        else if (t.pos === E.FINISH) done++;
        else track++;
      }
      const col = LU_COLORS[LU_P[i]].fill;
      const isTurn = g.winner === null && g.turn === i;
      const won = g.winner === i;
      const kind = P.kind === 'ai' ? 'AI' : (i === 0 ? 'You' : 'Human');
      const aiTag = P.kind === 'ai' && diffLbl ? ` · ${diffLbl}` : '';
      /* Phase 5.5: display name (custom or default) — engine NAMES stay
         untouched; the panel shows what the player configured. */
      const dname = luSeatName(i);
      /* Phase 5.6 fix — the die appears ONLY on the current roller's
         panel (and on a winner's panel, as a trophy). Inactive panels
         get NO die element at all: no empty dashed placeholders. The
         slot itself stays reserved via .lu-p-die-slot (fixed size,
         invisible) so every panel keeps identical width/height and the
         right-aligned columns never shift when the turn moves. */
      const showDie = isTurn || won;
      const face = isTurn && LUG.dice != null ? LUG.dice
                 : isTurn && LUG.phase === 'rolling' ? 6 : 0;
      return `<div class="lu-panel${isTurn ? ' on' : ''}${won ? ' won' : ''}"
        data-seat="${i}" style="--pc:${col}"
        aria-label="${dname} (${kind}${aiTag}): ${yard} in yard, ${track} on track, ${done} home">
        <div class="lu-p-id">
          <div class="lu-p-head">
            <span class="lu-p-dot" aria-hidden="true"></span>
            <span class="lu-p-name">${dname}</span>
          </div>
          <span class="lu-p-kind ${P.kind === 'ai' ? 'ai' : 'human'}">${kind}${aiTag}</span>
        </div>
        <div class="lu-p-row">
          <span class="lu-p-stat"><span class="k">Yd</span><span class="v">${yard}</span></span>
          <span class="lu-p-stat"><span class="k">Tr</span><span class="v">${track}</span></span>
          <span class="lu-p-stat h"><span class="k">Hm</span><span class="v">${done}</span></span>
        </div>
        ${showDie ? `<span class="lu-p-die">${luDieFaceDom(face)}</span>`
                  : `<span class="lu-p-die lu-p-die-slot" aria-hidden="true"></span>`}
        <span class="lu-p-turn"><i class="pulse" aria-hidden="true"></i>turn</span>
        <span class="lu-p-won">★ win</span>
      </div>`;
    }).join('');
  }

  /* Phase 5.2: keep every seat's in-panel die in lockstep with the
     engine (value + tumble ownership) — see luSyncSeatDice. */
  luSyncSeatDice();
}

/* ---- Phase 5.2: per-seat dice sync --------------------------------------
 * Each player panel carries its own colour-tinted die in its header
 * (built by luRenderHud). This function keeps every die in lockstep
 * with the engine on each render:
 *   • a seat's die shows LUG.dice ONLY while that seat holds the turn —
 *     a stale value can never linger on another player's die;
 *   • the `.rolling` tumble lives on the CURRENT roller only, driven by
 *     the same phase field the controller owns;
 *   • pure class/DOM sync — no timers, no listeners → nothing to leak.
 */
function luSyncSeatDice() {
  const g = LUG.game;
  if (!g) return;
  document.querySelectorAll('#luPanels .lu-panel').forEach(el => {
    const i = Number(el.dataset.seat);
    const isTurn = g.turn === i && g.winner === null;
    el.classList.toggle('rolling', isTurn && LUG.phase === 'rolling');
    /* keep the visible face honest: current roller → engine value.
       (luRenderHud builds the right face on rebuild; this catches
       mid-flight updates after a value lands without a rebuild.) */
    const die = el.querySelector('.lu-die');
    if (die && isTurn && LUG.dice != null && LUG.phase !== 'rolling') {
      luDieShow(die, LUG.dice);
    }
  });
}

/* Legacy helper kept for compatibility: paint the shared roll-button die. */
function luMsg(html) {
  const el = lu$('luMsg'); if (el) el.innerHTML = html;
}

/* Pip layout: which of the die's 9 grid dots show face n. */
const LU_PIPS = { 1: [4], 2: [0, 8], 3: [0, 4, 8], 4: [0, 2, 6, 8], 5: [0, 2, 4, 6, 8], 6: [0, 2, 3, 5, 6, 8] };

function luDieFace(n) {
  let dots = '';
  for (let i = 0; i < 9; i++) dots += `<i style="grid-area:${Math.floor(i / 3) + 1}/${(i % 3) + 1};${LU_PIPS[n].includes(i) ? '' : 'visibility:hidden'}"></i>`;
  return `<span class="lu-die" aria-hidden="true">${dots}</span>`;
}

/* ---- dice --------------------------------------------------------------- */

function luSetDice(v, rolling) {
  const btn = lu$('luRollBtn'); if (!btn) return;
  const die = btn.querySelector('.lu-die');
  if (die) {
    die.classList.toggle('rolling', !!rolling);
    if (v != null) luDieShow(die, v);
  }
  /* Phase 5.2: keep the per-seat dice in lockstep — the current
     roller's die takes the value; luSyncSeatDice flips the tumble
     class from the same `rolling` argument the controller owns. */
  LUG.phase = rolling ? 'rolling' : LUG.phase;
  luRenderHud();
  luRenderControls();
}

/** Paint face `v` onto a die element (shared by tumbling + final face). */
function luDieShow(die, v) {
  const P = LU_PIPS[v] || LU_PIPS[6];
  die.querySelectorAll('i').forEach((d, i) =>
    d.style.visibility = P.includes(i) ? 'visible' : 'hidden');
}

/**
 * Tumble animation (Phase 3 Part C): while the roll is in flight the die
 * cycles random faces so the throw reads as a real tumble. The rAF chain
 * is owned by LUG.diceRaf and cancelled by luCancelDiceSpin() — from the
 * resolve, a restart, teardown or a route change — so no stray frame can
 * ever repaint the FINAL face after the engine result has landed.
 */
function luCancelDiceSpin() {
  if (LUG.diceRaf) { cancelAnimationFrame(LUG.diceRaf); LUG.diceRaf = 0; }
}

function luDiceSpin() {
  luCancelDiceSpin();
  const dieEl = lu$('luRollBtn') && lu$('luRollBtn').querySelector('.lu-die');
  /* Phase 5.2: the CURRENT seat's in-panel die tumbles too — same rAF
     chain, same epoch guard, so it can never outlive the roll or run on
     a dead view (single owner: LUG.diceRaf). */
  const g = LUG.game;
  const seatEl = g && document.querySelector(
    `#luPanels .lu-panel[data-seat="${g.turn}"] .lu-die`);
  if (!dieEl && !seatEl) return;
  const my = LUG.clock;
  let last = 0;
  const tick = now => {
    if (my !== LUG.clock || !LUG.viewAlive()) { LUG.diceRaf = 0; return; }
    if (now - last >= 80) {                       // new face every ~80ms
      last = now;
      const v = 1 + Math.floor(Math.random() * 6);
      if (dieEl) luDieShow(dieEl, v);
      if (seatEl) luDieShow(seatEl, v);
    }
    LUG.diceRaf = requestAnimationFrame(tick);
  };
  LUG.diceRaf = requestAnimationFrame(tick);
}

function luRenderControls() {
  const g = LUG.game;
  const btn = lu$('luRollBtn');
  if (!btn || !g) return;
  if (LUG.lobby) { btn.disabled = true; btn.classList.remove('on'); return; }
  const human = g.players[g.turn].kind === 'human';
  /* Phase 5.2: the button also mirrors the CURRENT player's colour so
     it is obvious at a glance whose roll it triggers. Disabled logic is
     unchanged: only a human's idle phase may click. */
  btn.disabled = !(g.winner === null && human && LUG.phase === 'idle');
  btn.style.setProperty('--pc', LU_COLORS[LU_P[g.turn]].fill);
  btn.classList.toggle('on', !btn.disabled);
  const lbl = lu$('luRollLbl');
  if (lbl) lbl.textContent = g.winner !== null ? 'game over'
    : human ? (LUG.phase === 'idle' ? `Roll for ${luSeatName(g.turn)}` : 'pick a token')
    : `${luSeatName(g.turn)} (AI) thinking…`;
}

/* ---- overlay ---------------------------------------------------------------- */
function luOverlayShow(opts) {
  const ov = lu$('luOverlay'); if (!ov) return;
  lu$('luOvTitle').innerHTML = opts.title || '';
  lu$('luOvSub').innerHTML = opts.sub || '';
  lu$('luOvRow').innerHTML = (opts.row || []).map(b =>
    `<button type="button" class="lu-ov-btn${b.primary ? ' primary' : ''}" data-act="${b.act}">${b.label}</button>`).join('');
  const stats = lu$('luOvStats');
  if (opts.stats) {
    stats.innerHTML = opts.stats.map(s => `<div class="lu-ov-stat"><div class="k">${s.k}</div><div class="v">${s.v}</div></div>`).join('');
    stats.classList.remove('hidden');
  } else stats.classList.add('hidden');
  ov.classList.remove('hidden');
}
function luOverlayHide() {
  const ov = lu$('luOverlay'); if (ov) ov.classList.add('hidden');
}

/* =====================================================================
   TURN CONTROLLER — the single owner of the game clock.
   Every async step captures `my` epoch and re-checks it before acting,
   so restart/teardown/route-away can never leave a zombie timer alive.
   ===================================================================== */
function luStartTurn() {
  const my = ++LUG.clock;             // game-flow token for this turn
  let g = LUG.game;
  LUG.settleUntil = 0;                 // any new turn ends any pending hand-off
  if (!g || g.winner !== null) return;
  /* Defence in depth: an inactive seat must never act. If the turn ever
     lands on one (it cannot via advanceTurn), hand off to the next active. */
  if (!LudoEngine.isActive(g, g.turn)) {
    LudoEngine.advanceTurn(g);
    luRenderHud();
  }

  LUG.phase = 'idle';
  LUG.dice = null; LUG.moves = [];
  luSetDice(null, false);
  luRenderHud(); luDraw(); luRenderControls();

  const name = luSeatName(g.turn);   // Phase 5.5: display name
  const kind = g.players[g.turn].kind;
  luMusicFollow();          // Phase 5.4: a live turn → music follows
  if (kind === 'ai') {
    luMsg(`<b>${name} (AI)</b> is taking its turn…`);
    setTimeout(() => { if (my === LUG.clock && LUG.viewAlive()) luAiRoll(); }, LU_T.aiThink);
  } else {
    const you = g.turn === 0 ? 'Your move' : `${name}'s move`;
    luMsg(`${you}, <b>${name}</b>. Roll the dice.`);
  }
}

/** Is the mounted view still live (not torn down)? */
LUG.viewAlive = () => !!LUG.canvas;

/* Turn-flow timings (ms). Tests may shrink these via LUD.fast(); the
   shipped defaults match the other Fun Time games' pacing. */
const LU_T = {
  rollSpin: 480,     // dice tumble before the value lands
  aiThink: 650,      // pause before an AI rolls
  aiPick: 620,       // pause while the AI chooses a token
  stepMs: 150,       // one square of the walk animation
  resolveMs: 450,    // settle delay after a plain move
  eventMs: 850,      // longer settle when a capture/finish must be seen
};

/** Human pressed Roll (or AI auto-roll): throw the die. */
function luDoRoll() {
  const g = LUG.game;
  if (!g || g.winner !== null) return;
  if (LUG.phase !== 'idle') return;                    // one roll at a time
  LUG.phase = 'rolling';
  luSetDice(null, true);
  luDiceSpin();                                         // tumbling faces
  luSfx('roll');

  const my = LUG.clock;
  setTimeout(() => {
    if (my !== LUG.clock || !LUG.viewAlive()) { luCancelDiceSpin(); return; }  // stale game
    luCancelDiceSpin();                                 // stop BEFORE resolve
    const v = LudoEngine.rollDice();
    LUD.onRoll(v);                                      // final face = engine result
  }, LU_T.rollSpin);
}


/** Roll resolved: compute legal moves and hand control correctly. */
function luResolveRoll(v) {
  const my = LUG.clock;
  const g = LUG.game;
  if (!g || g.winner !== null) return;

  LUG.dice = v;
  luSetDice(v, false);
  LudoEngine.noteRoll(g, v);
  /* Phase 5.3: six flare — a short dashed ring on the roller's in-panel
     die marks the lucky throw (render-only; the chain logic in the
     engine is untouched). */
  if (v === 6) {
    luSfx('six');
    const dieEl = document.querySelector(
      `#luPanels .lu-panel[data-seat="${g.turn}"] .lu-die`);
    if (dieEl && LUG.canvas) {
      const r = dieEl.getBoundingClientRect();
      const cr = LUG.canvas.getBoundingClientRect();
      /* convert page coords → canvas units (canvas is square, dpr-scaled) */
      const cx = (r.left + r.width / 2 - cr.left) * (LUG.w / cr.width);
      const cy = (r.top + r.height / 2 - cr.top) * (LUG.w / cr.width);
      luFx({ kind: 'six', x: cx, y: cy,
             col: LU_COLORS[LU_P[g.turn]].fill });
    }
  }
  luRenderHud();                    // Phase 5.2: seat die shows final face

  const name = luSeatName(g.turn);     // Phase 5.5: display name
  const kind = g.players[g.turn].kind;
  const moves = LudoEngine.legalMoves(g, g.turn, v);
  LUG.moves = moves;

  if (!moves.length) {
    /* Unplayable roll — even a six never grants an extra turn here.
       Mark it in the engine so luEndRoll/grantsExtraRoll hand the
       dice to the next eligible player instead of offering a re-roll
       (regression: three tokens home + last token needing 2 while a
       six is rolled must pass the turn, not loop the roller). */
    LudoEngine.noteNoMove(g);
    luMsg(`<b>${name}</b> rolled ${v} — no legal move.`);
    /* The turn is still being handed on during this settle window even
       though phase reads 'idle' — record the deadline so LUD.force()
       cannot inject a second roll into the same hand-off. Self-expiring:
       even if this timer is killed by an external clock bump, the window
       simply lapses instead of blocking forever. */
    LUG.settleUntil = performance.now() + LU_T.eventMs;
    setTimeout(() => { if (my === LUG.clock && LUG.viewAlive()) luEndRoll(); }, LU_T.eventMs);
    return;
  }

  if (kind === 'ai') {
    LUG.phase = 'awaiting';
    luDraw();
    setTimeout(() => {
      if (my !== LUG.clock || !LUG.viewAlive()) return;
      /* Phase 4.3/4.5: difficulty-aware selection. Easy uses the per-game
         seeded RNG; 'medium'/'hard' use their weighted scorers (Hard adds
         reply-threat lookahead); the default (null) keeps the legacy
         scorer exactly as before. */
      const pick = LudoEngine.selectAIMoveForDifficulty(
        g, g.turn, moves, LUG.aiDifficulty, LUG.aiRng);
      luExecuteMove(pick);
    }, LU_T.aiPick);
  } else {
    LUG.phase = 'awaiting';
    luDraw(); luRenderControls();
    const who = g.turn === 0 ? 'You rolled' : `${luSeatName(g.turn)} rolled`;
    luMsg(`${who} <b>${v}</b>. ${v === 6 ? 'A six! ' : ''}Pick a highlighted token.`);
  }
}

/** Roll fully consumed: extra roll or next player. */
function luEndRoll() {
  const g = LUG.game;
  LUG.settleUntil = 0;                  // hand-off landed: force() may run again
  if (!g) return;
  if (g.winner !== null) { luGameOver(); return; }

  /* Bonus rolls FIRST: a capture or a token reaching its final home
     grants exactly one extra roll each, whatever the dice showed. This
     is the fix for the turn-resolution bug: previously the six rule
     alone decided, so a capture made on a 1–5 wrongly passed the dice
     (no extra turn at all), and a capture made on a six was
     indistinguishable from the six rule. grantsExtraRoll() consults
     state.bonus / state.finishBonus, which applyMove set on the event
     and noteRoll clears when this bonus roll is taken — so each bonus
     applies exactly ONCE per triggering roll. */
  if (LudoEngine.grantsExtraRoll(g, LUG.dice)) {
    const why = g.finishBonus ? 'finish'
      : g.bonus ? 'capture'
      : LUG.dice === 6 ? 'six' : '';
    if (why === 'finish') {
      luMsg(LUG.dice === 6
        ? 'Token <b>home</b> + <b>six</b> — roll again!'
        : 'Token reached <b>home</b> — bonus roll!');
    } else if (why === 'capture') {
      luMsg(LUG.dice === 6
        ? 'Capture + <b>six</b> — roll again!'
        : '<b>Capture!</b> Bonus roll.');
    } else if (why === 'six') {
      luMsg('Rolled a <b>six</b> — roll again!');
    }
    luStartTurnKeepPlayer();
    return;
  }
  /* no bonus, not a six (or an unplayable six / capped chain): pass on */
  LudoEngine.advanceTurn(g);
  luStartTurn();
}

/** Extra roll: same player, fresh roll phase (chain preserved).
 *  Phase 5.2 fix: a full luRenderHud() runs here too — the roller's
 *  in-panel die must drop its previous face while the fresh roll is
 *  awaited, exactly as it does on a turn change. */
function luStartTurnKeepPlayer() {
  const my = ++LUG.clock;
  const g = LUG.game;
  if (!g || g.winner !== null) return;
  LUG.phase = 'idle'; LUG.moves = []; LUG.dice = null;
  luSetDice(null, false);
  luRenderHud(); luRenderControls();
  if (g.players[g.turn].kind === 'ai') {
    setTimeout(() => { if (my === LUG.clock && LUG.viewAlive()) luAiRoll(); }, LU_T.aiThink);
  }
}

function luAiRoll() {
  const g = LUG.game;
  if (!g || g.winner !== null) return;
  if (g.players[g.turn].kind !== 'ai') return;         // never act for humans
  if (LUG.phase !== 'idle') return;
  luDoRoll();
}

/** Execute a chosen legal move: animate, then resolve. */
function luExecuteMove(move) {
  const g = LUG.game;
  if (!g || g.winner !== null || !move) return;
  LUG.phase = 'moving';
  LUG.moves = [];
  luRenderControls();

  const p = g.turn, t = move.token;
  const from = g.players[p].tokens[t].pos;
  const steps = [];
  if (from === -1) steps.push(0);
  else for (let s = from + 1; s <= move.to; s++) steps.push(s);

  /* One glide per hop: the token visually travels from its current
     square to the next while the engine stays at `from`. luCancelAnim()
     first: if any earlier chain is somehow still alive, it dies now —
     only one rAF walk can ever be in flight. */
  luCancelAnim();
  LUG.anim = { player: p, token: t, path: steps, i: 0, to: move.to, raf: 0 };
  luGlideStep();
}

/**
 * One square of the walk, animated with requestAnimationFrame.
 *
 *   • `_viz = {from, to, t}` is written on every frame — a purely
 *     visual hint; the ENGINE state is untouched until luMoveDone().
 *   • Every frame re-checks the game clock, so a restart, teardown or
 *     route-change mid-glide kills the chain instantly (no zombie
 *     frames can ever commit a stale move).
 *   • Only one rAF chain can be alive: luCancelAnim() runs before each
 *     new walk and from teardown/new-game, so callbacks never stack.
 *   • Endpoints are recomputed each frame (see luGlidePoint), keeping
 *     coordinates correct across a window resize mid-glide.
 */
function luCancelAnim() {
  if (LUG.anim) {
    if (LUG.anim.raf) cancelAnimationFrame(LUG.anim.raf);
    /* drop any fractional hint too — a cancelled glide must never leave
       a token frozen between squares (renderer treats _viz as truth) */
    const g = LUG.game;
    const tok = g && g.players[LUG.anim.player] && g.players[LUG.anim.player].tokens[LUG.anim.token];
    if (tok && tok._viz) delete tok._viz;
  }
  LUG.anim = null;
}

function luGlideStep() {
  const my = LUG.clock;
  const a = LUG.anim;
  if (!a) return;

  if (a.i >= a.path.length) { luMoveDone(); return; }

  const g = LUG.game;
  const tok = g.players[a.player].tokens[a.token];
  const from = a.i === 0 ? g.players[a.player].tokens[a.token].pos : a.path[a.i - 1];
  const to = a.path[a.i];
  const start = performance.now();
  const dur = Math.max(40, LU_T.stepMs);      // honour test pacing, never instant

  const tick = now => {
    if (my !== LUG.clock || !LUG.viewAlive()) {   // stale chain: die silently
      if (tok._viz) delete tok._viz;
      if (LUG.anim === a) LUG.anim = null;        // only if we still own the slot
      return;
    }
    const aa = LUG.anim;
    if (!aa || aa !== a) return;                   // replaced/cancelled: die
    const t = Math.min(1, Math.max(0, (now - start) / dur));  // clamp ≥0: the first
    tok._viz = { from, to, t };                    // frame's rAF stamp can predate
    luDraw();
    if (t < 1) {
      aa.raf = requestAnimationFrame(tick);
    } else {
      /* Square fully reached. Keep a RESTING hint ({from:to,to:to,t:1})
         until the next hop's first frame overwrites it: the engine still
         holds the walk ORIGIN here (pos commits only in luMoveDone), so
         deleting _viz now would make any draw in this window — the next
         rAF is up to a frame away — snap the token back to the origin
         square (observed live: a full-cell ~135px jump mid-walk). */
      tok._viz = { from: to, to: to, t: 1 };
      luSfx('step');
      aa.i++;
      if (aa.i >= aa.path.length) luMoveDone();
      else luGlideStep();                          // next hop (fresh closure)
    }
  };
  a.raf = requestAnimationFrame(tick);
}

/**
 * Phase 5.3 — captured-token flight home (VISUAL ONLY).
 *
 * The engine has ALREADY reset the victim to pos −1 (applyMove is the
 * sole authority and ran before this). A victim at pos −1 renders in
 * its yard instantly, which reads as a teleport; this overlays a brief
 * ghost that flies from the capture square back to the yard so the
 * send-home reads as an event.
 *
 *   • implemented as an FX record, NOT a second _viz walk: it cannot
 *     touch hit-testing, stacks or the single-glide invariant;
 *   • the pump's clock/epoch guards kill it on restart/teardown;
 *   • duration is short (450ms) and overlaps the existing eventMs
 *     settle window, so turn pacing is unchanged.
 */
function luFlyHome(p, i, pre) {
  const g = LUG.game; if (!g) return;
  const yardPt = luYardCenter(p, i);
  if (!yardPt) return;
  /* start at the capture square when known (pre-commit centre), else
     fall back to a neutral origin so the effect still reads. */
  const fromX = pre ? pre.x : yardPt.x;
  const fromY = pre ? pre.y : yardPt.y;
  luFx({ kind: 'fly', x: fromX, y: fromY, tx: yardPt.x, ty: yardPt.y,
         col: LU_COLORS[LU_P[p]].fill, dur: 450 });
}

/** Animation finished: commit via the ENGINE, resolve events. */
function luMoveDone() {
  const my = LUG.clock;
  const g = LUG.game;
  const a = LUG.anim;
  LUG.anim = null;
  if (!g || !a) return;

  const p = a.player, t = a.token;
  /* Drop the resting hint BEFORE the commit: from here on the engine's
     `pos` is the render truth (applyMove stays the sole authority that
     writes it). Keeping _viz would freeze the token at its pre-commit
     square while the state beneath it has already moved. */
  const tokDone = g.players[p] && g.players[p].tokens[t];
  if (tokDone && tokDone._viz) delete tokDone._viz;
  /* Phase 5.3: anchor square for the FX overlays, captured BEFORE
     applyMove (the effect must sit where the event happened), plus the
     victims' pre-capture centres so their flight home starts at the
     capture square rather than teleporting from the yard. */
  const E = window.LudoEngine;
  const landPt = luPosCenter(p, g.players[p].tokens[t].pos, 0);
  const victimsBefore = [];
  if (g.players[p].tokens[t].pos >= 0 &&
      g.players[p].tokens[t].pos <= E.LAST_MAIN) {
    const gl = E.globalOf(p, g.players[p].tokens[t].pos + (LUG.dice || 0));
    for (let q = 0; q < 4; q++) {
      if (q === p || !LudoEngine.isActive(g, q)) continue;
      g.players[q].tokens.forEach((vt, vi) => {
        if (vt.pos >= 0 && vt.pos <= E.LAST_MAIN &&
            E.globalOf(q, vt.pos) === gl) {
          const c = luPosCenter(q, vt.pos, vi);
          if (c) victimsBefore.push({ player: q, token: vi, x: c.x, y: c.y });
        }
      });
    }
  }
  const ev = LudoEngine.applyMove(g, p, t, LUG.dice);   // rules own the truth

  if (ev.entered) luSfx('enter');
  if (ev.captured.length) luSfx('capture');
  if (ev.finished) luSfx('finish');

  /* ---- Phase 5.3 visual feedback (all render-only) ---- */
  if (landPt) {
    const landedOn = g.players[p].tokens[t].pos;
    /* capture: impact ring at the landing square + victim flight home */
    if (ev.captured.length && landPt) {
      luFx({ kind: 'impact', x: landPt.x, y: landPt.y,
             col: LU_COLORS[LU_P[p]].fill });
      for (const v of ev.captured) {
        const pre = victimsBefore.find(b => b.player === v.player && b.token === v.token);
        luFlyHome(v.player, v.token, pre);
      }
    }
    /* safe star: gold pulse when a track landing is a star square */
    if (!ev.finished && landedOn >= 0 && landedOn <= E.LAST_MAIN &&
        E.SAFE_G.has(E.globalOf(p, landedOn))) {
      luFx({ kind: 'safe', x: landPt.x, y: landPt.y });
      luSfx('safe');                                  // Phase 5.4
    }
    /* home-lane entry: colour glow when crossing into the private lane */
    if (!ev.finished && landedOn > E.LAST_MAIN && landedOn < E.FINISH) {
      luFx({ kind: 'lane', x: landPt.x, y: landPt.y,
             col: LU_COLORS[LU_P[p]].fill });
      luSfx('lane');                                  // Phase 5.4
    }
    /* token finished: bright bloom at the centre pool slot */
    if (ev.finished) {
      const fp = luFinishSlot(p, t);
      luFx({ kind: 'finish', x: fp.x, y: fp.y });
    }
  }

  const who = luSeatName(p);          // Phase 5.5: display name
  let msg = `<b>${who}</b> moved.`;
  if (ev.entered) msg = `<b>${who}</b> entered the board.`;
  if (ev.captured.length) {
    const names = ev.captured.map(v => luSeatName(v.player)).join(', ');
    msg += ` Captured <b>${names}</b>! Token sent home.`;
  }
  if (ev.finished) msg += ` A token reached <b>home</b>!`;

  luMsg(msg);
  luRenderHud(); luDraw();
  /* Phase 5.3: the commit may have changed stack membership (arrival,
     departure, victim sent home) — start the fan-out ease pump so the
     reshuffle glides. Harmless no-op when nothing is easing. */
  luStackPump();
  luMusicFollow();          // Phase 5.4: music tracks game activity

  if (LudoEngine.checkWinner(g) !== null) {
    setTimeout(() => { if (my === LUG.clock && LUG.viewAlive()) luGameOver(); }, LU_T.eventMs);
    return;
  }

  setTimeout(() => {
    if (my === LUG.clock && LUG.viewAlive()) luEndRoll();
  }, ev.captured.length || ev.finished ? LU_T.eventMs : LU_T.resolveMs);
}

function luGameOver() {
  const g = LUG.game;
  if (!g || g.winner === null) return;
  LUG.phase = 'over';
  luRenderControls();
  const name = luSeatName(g.winner);   // Phase 5.5: display name
  const you = g.players[g.winner].kind === 'human';
  luSfx(you ? 'win' : 'over');
  luMusicStop();            // Phase 5.4: game over → music out (stinger solo)
  /* Phase 5.3: winner bloom at the board centre (render-only). */
  const c = luCenter(7, 7);
  if (c) luFx({ kind: 'win', x: c.x, y: c.y,
                col: LU_COLORS[LU_P[g.winner]].fill });
  luMsg(`<b>${name}</b> brought all four tokens home. Game over!`);
  luOverlayShow({
    title: you ? 'Victory!' : `${name} wins`,
    sub: you ? 'All four of your tokens made it home.' :
      `${name}'s AI swept the board. Try again?`,
    stats: [
      { k: 'rolls', v: g.rolls },
      { k: 'your home', v: g.players[0].home },
    ],
    row: [
      { act: 'again', label: 'Play Again', primary: true },
      { act: 'menu', label: 'Back to Games' },
    ],
  });
}

/* ---- single source of truth for token geometry ------------------------------
   Every pixel a token occupies — resting, stacked or mid-glide — is
   computed by ONE function. luDraw() and luHitTest() both call it, so
   what you see is exactly what you click (Phase 3, Part B).
   -------------------------------------------------------------------------- */

/**
 * The pixel centre of player p's token i in its CURRENT visual state.
 *
 *   • resting yard token        → yard slot
 *   • resting track/lane token  → cell centre (+ stack fan-out)
 *   • gliding token             → interpolated point between steps
 *   • finished token            → centre-pool cluster slot
 *
 * `stacks` (optional) is the shared stack map built once per frame; when
 * given, tokens sharing a square are fanned apart deterministically and
 * their radius is scaled down so up to 16 tokens fit inside one cell.
 */
function luTokenPoint(p, i, stacks) {
  const E = window.LudoEngine;
  const g = LUG.game;
  if (!g) return null;
  const tok = g.players[p].tokens[i];
  const viz = tok._viz;
  const pos = (viz !== undefined && viz !== null) ? viz : tok.pos;

  let pt;
  if (pos === -1) pt = luYardCenter(p, i);
  else if (typeof pos === 'object') pt = luGlidePoint(p, i, pos);      // mid-glide
  else if (pos === E.FINISH) pt = luFinishSlot(p, i);
  else pt = luPosCenter(p, pos, i);

  /* deterministic fan-out for stacked tokens (shared track or lane cell) */
  const skey = luStackKey(p, pos);
  if (stacks && skey !== null) {
    const mates = stacks.get(skey) || [];
    if (mates.length > 1) {
      const idx = mates.findIndex(m => m.p === p && m.i === i);
      pt = luStackOffsetSmooth(p, i, pt, idx, mates.length);
    } else {
      /* solo again: ease any remembered offset back to centre */
      pt = luStackOffsetSmooth(p, i, pt, -1, 1);
    }
  }
  return pt;
}

/**
 * Phase 5.3 — smooth stack transitions.
 *
 * luStackOffset() is fully deterministic (same inputs → same pixel),
 * which the hit-test relies on; changing that would desynchronise
 * clicks from painting. Instead this wrapper EASES toward that target:
 * each token remembers its last painted offset in a WeakMap and moves a
 * fraction of the remaining distance per frame (~120ms convergence).
 * The hit-test keeps calling luTokenPoint with the SAME easing (it goes
 * through this function too), so painted and clickable positions stay
 * identical by construction — both read the WeakMap's current value.
 * A WeakMap keyed by the token object can never leak a dead game.
 */
const LU_STACK_EASE = new WeakMap();
function luStackOffsetSmooth(p, i, pt, idx, n) {
  const target = idx >= 0 ? (luStackLayout(n)[idx] || { dx: 0, dy: 0 })
                          : { dx: 0, dy: 0 };
  const g = LUG.game;
  const tok = g && g.players[p] && g.players[p].tokens[i];
  if (!tok) return { x: pt.x + target.dx, y: pt.y + target.dy };
  let cur = LU_STACK_EASE.get(tok);
  if (!cur) { cur = { dx: target.dx, dy: target.dy }; LU_STACK_EASE.set(tok, cur); }
  const k = LUG.reducedMotion ? 1 : 0.22;         // per-frame convergence
  cur.dx += (target.dx - cur.dx) * k;
  cur.dy += (target.dy - cur.dy) * k;
  /* snap when close enough so the final position is EXACTLY the
     deterministic layout (no permanent sub-pixel drift). */
  if (Math.abs(target.dx - cur.dx) < 0.4 && Math.abs(target.dy - cur.dy) < 0.4) {
    cur.dx = target.dx; cur.dy = target.dy;
  }
  return { x: pt.x + cur.dx, y: pt.y + cur.dy };
}

/** Phase 5.3: is any token still easing toward its stack seat? Used to
 *  keep a short redraw pump alive after a stack changes (arrival or
 *  departure), so the fan-out visibly glides instead of jumping. */
function luStackEasing() {
  const g = LUG.game; if (!g) return false;
  for (let p = 0; p < 4; p++) for (let i = 0; i < 4; i++) {
    const tok = g.players[p].tokens[i];
    const cur = LU_STACK_EASE.get(tok);
    if (!cur) continue;
    const skey = luStackKey(p, tok._viz != null ? tok._viz : tok.pos);
    let target = { dx: 0, dy: 0 };
    if (skey !== null) {
      const stacks = luBuildStacks();
      const mates = stacks.get(skey) || [];
      if (mates.length > 1) {
        const idx = mates.findIndex(m => m.p === p && m.i === i);
        target = luStackLayout(mates.length)[idx] || target;
      }
    }
    if (Math.abs(target.dx - cur.dx) >= 0.4 || Math.abs(target.dy - cur.dy) >= 0.4)
      return true;
  }
  return false;
}

/** Phase 5.3: one-frame-at-a-time pump for stack easing. Same guard
 *  pattern as the FX pump (clock + epoch); stops itself the moment no
 *  token is mid-ease. Never runs during a glide (the glide's own rAF
 *  already repaints every frame). */
let luStackRaf = 0;
function luStackPump() {
  if (luStackRaf) return;
  const my = LUG.clock, ep = LUG.epoch;
  const tick = () => {
    luStackRaf = 0;
    if (my !== LUG.clock || ep !== LUG.epoch || !LUG.viewAlive()) return;
    if (LUG.anim || LUG.diceRaf || LUG.fxRaf) return;   // another loop paints
    if (luStackEasing()) {
      luDraw();
      luStackRaf = requestAnimationFrame(tick);
    } else if (LUG.game) {
      luDraw();                                          // settle frame
    }
  };
  luStackRaf = requestAnimationFrame(tick);
}

/**
 * Deterministic offset + radius for the k-th of n tokens on one square.
 * n ≤ 4: four quadrant seats around the centre (classic Ludo stacking).
 * n > 4: seats spiral outward on a small ring so 5–16 tokens stay inside
 * the cell and every one keeps a separate, clickable centre.
 */
function luStackLayout(n) {
  const C = LUG.cell;
  const seats = [];
  if (n <= 4) {
    const d = C * 0.15;
    for (let k = 0; k < n; k++) seats.push({ dx: (k % 2 ? d : -d), dy: (k > 1 ? d : -d) });
  } else {
    const ringR = C * 0.26, inner = C * 0.13;
    for (let k = 0; k < n; k++) {
      if (k < 4) { seats.push({ dx: (k % 2 ? inner : -inner), dy: (k > 1 ? inner : -inner) }); continue; }
      const a = (Math.PI * 2 * (k - 4)) / (n - 4) - Math.PI / 2;
      seats.push({ dx: Math.cos(a) * ringR, dy: Math.sin(a) * ringR });
    }
  }
  return seats;
}

/** Offset point for the idx-th member of an n-token stack. */
function luStackOffset(pt, idx, n) {
  const seat = luStackLayout(n)[idx] || { dx: 0, dy: 0 };
  return { x: pt.x + seat.dx, y: pt.y + seat.dy };
}

/** Visual radius for the members of an n-token stack (keeps them in-cell). */
function luStackRadius(n) {
  const base = LUG.cell * 0.36;
  if (n <= 1) return base;
  if (n <= 4) return base * 0.82;
  if (n <= 8) return base * 0.68;
  return base * 0.56;
}

/**
 * Stack-map key for a resting position, or null when the position has
 * its own dedicated geometry (yard, finish pool, mid-glide).
 *
 *   • shared-track squares (0..50) key by GLOBAL index — several
 *     players' tokens can share one square, so the key is player-free;
 *   • home-lane cells (51..55) are PRIVATE: only the owning player can
 *     ever stand there, so the key includes the player. Without this,
 *     two of the owner's tokens resting on the same lane cell would
 *     draw at identical centres — perfectly overlapped, the lower one
 *     invisible and unclickable (found by the Phase 3 home-lane audit);
 *   • the finish pool (56) staggers per token via luFinishSlot, and
 *     yards stagger via YARD_SLOTS — neither needs stack fan-out.
 *
 * Mid-glide positions (`_viz` objects) are excluded on purpose: a
 * walking token keeps full size while transiting (same rule as the
 * shared track).
 */
function luStackKey(p, pos) {
  const E = window.LudoEngine;
  if (typeof pos !== 'number' || pos < 0) return null;
  if (pos <= E.LAST_MAIN) return 'T' + E.globalOf(p, pos);   // shared track
  if (pos < E.FINISH) return 'L' + p + ':' + pos;            // private lane cell
  return null;                                               // finish pool
}

/**
 * Build the stack map: square → [{p, i}…] for every token whose VISUAL
 * position rests on the board (shared track or a home-lane cell — see
 * luStackKey). Built once per frame and shared by drawing and
 * hit-testing so both agree exactly.
 */
function luBuildStacks() {
  const g = LUG.game;
  const stacks = new Map();
  if (!g) return stacks;
  for (let p = 0; p < 4; p++) {
    for (let i = 0; i < 4; i++) {
      const tok = g.players[p].tokens[i];
      const viz = tok._viz;
      const pos = (viz !== undefined && viz !== null) ? viz : tok.pos;
      const key = luStackKey(p, pos);
      if (key !== null) {
        if (!stacks.has(key)) stacks.set(key, []);
        stacks.get(key).push({ p, i });
      }
    }
  }
  return stacks;
}

function luCanvasPoint(e) {
  const c = LUG.canvas;
  const r = c.getBoundingClientRect();
  const dpr = LUG.w / r.width;
  return { x: (e.clientX - r.left) * dpr, y: (e.clientY - r.top) * dpr };
}

function luHitTest(x, y) {
  const g = LUG.game; if (!g) return null;
  const E = window.LudoEngine;
  const C = LUG.cell;
  /* Same map the renderer used this frame → identical coordinates. */
  const stacks = luBuildStacks();

  const isLegalToken = (p, i) => LUG.phase === 'awaiting' &&
    LUG.moves.some(m => m.token === i && g.turn === p);

  let best = null, bestD = Infinity;
  let bestLegal = null, bestLegalD = Infinity;
  for (let p = 0; p < 4; p++) {
    for (let i = 0; i < 4; i++) {
      const tok = g.players[p].tokens[i];
      const viz = tok._viz;
      const pos = (viz !== undefined && viz !== null) ? viz : tok.pos;
      const pt = luTokenPoint(p, i, stacks);
      if (!pt) continue;

      /* clickable radius matches the drawn (stack-scaled) token */
      let R = C * 0.36;
      const skey = luStackKey(p, pos);
      if (skey !== null) {
        const mates = stacks.get(skey) || [];
        if (mates.length > 1) R = luStackRadius(mates.length);
      }
      const d = Math.hypot(pt.x - x, pt.y - y);
      if (d > R * 1.25) continue;                 // outside this token

      /* mixed-player stacks: nearest wins, but a LEGAL token always
         beats an illegal one at comparable distance so the player can
         never be locked out by an opponent sharing the square. */
      if (isLegalToken(p, i)) {
        if (d < bestLegalD) { bestLegalD = d; bestLegal = { p, i }; }
      } else if (d < bestD) { bestD = d; best = { p, i }; }
    }
  }
  return bestLegal || best;
}


function luPick(hit) {
  const g = LUG.game;
  if (!g || g.winner !== null) return;
  if (LUG.phase !== 'awaiting') return;               // only after a roll
  if (g.players[g.turn].kind !== 'human') return;     // never during AI turn
  let mv = LUG.moves.find(m => m.token === hit.i && g.turn === hit.p);
  /* Re-validate against the LIVE engine state: LUG.moves could be stale
     relative to a mutated game (test hooks / diagnostics). The engine is
     the only authority on legality — never trust a cached list. */
  if (mv) {
    const pos = g.players[hit.p].tokens[hit.i].pos;
    const fresh = window.LudoEngine.legalMoves(g, g.turn, LUG.dice)
      .find(m => m.token === hit.i);
    if (!fresh || fresh.from !== pos) mv = null;
  }
  if (!mv) { luIllegalPick(hit); return; }            // illegal — engine says no
  luSfx('pick');                                      // Phase 5.4: token selected
  luExecuteMove(mv);
}

/**
 * Subtle feedback for an illegal click (Phase 3 Part D): opponent's token,
 * own unmovable token, or a click while no roll is pending. The message is
 * short-lived and NEVER mutates game state — phase, moves and dice stay
 * exactly as they were.
 */
let luNudgeTimer = 0;
function luIllegalPick(hit) {
  const g = LUG.game; if (!g) return;
  const E = window.LudoEngine;
  const mine = hit && g.turn === hit.p;
  let why;
  if (hit && !mine) why = `That's <b>${luSeatName(hit.p)}</b>'s token — pick your own.`;
  else if (hit && mine && g.players[hit.p].tokens[hit.i].pos === E.FINISH)
    why = 'That token is already <b>home</b>.';
  else if (hit && mine && g.players[hit.p].tokens[hit.i].pos === -1)
    /* Yard token: the honest reason is the release rule, not the step
       count — "can't move 3 squares" misleads when any non-6 is rolled. */
    why = LUG.dice === 6
      ? 'That token can\'t be released right now.'
      : 'You need a <b>six</b> to release a token from the yard.';
  else if (hit && mine && typeof LUG.dice === 'number' &&
           g.players[hit.p].tokens[hit.i].pos + LUG.dice > E.FINISH)
    /* Overshoot: the token would pass home — only an exact roll finishes. */
    why = `Home needs an <b>exact roll</b> — that token is ${E.FINISH - g.players[hit.p].tokens[hit.i].pos} step${E.FINISH - g.players[hit.p].tokens[hit.i].pos === 1 ? '' : 's'} away.`;
  else if (hit && mine)
    why = `That token can't move <b>${LUG.dice != null ? LUG.dice : ''}</b> square${LUG.dice === 1 ? '' : 's'}.`;
  else why = 'Not a token — click one of your own pieces.';
  luMsg(`<span class="lu-nudge">${why}</span>`);

  /* flash the clicked token briefly (purely visual, no state change) */
  if (hit) {
    LUG.nudge = { p: hit.p, i: hit.i, until: performance.now() + 700 };
    luDraw();
  }
  luSfx('invalid');                                   // Phase 5.4: named cue

  clearTimeout(luNudgeTimer);
  luNudgeTimer = setTimeout(() => {
    if (!LUG.viewAlive()) return;
    LUG.nudge = null;
    luDraw();
    /* restore the standard awaiting prompt once the nudge fades */
    if (LUG.phase === 'awaiting' && LUG.game && LUG.game.players[LUG.game.turn].kind === 'human') {
      luMsg(`You rolled <b>${LUG.dice}</b>. ${LUG.dice === 6 ? 'A six! ' : ''}Pick a highlighted token.`);
    }
  }, 1400);
}


/** Intro/status line that reflects the active seat configuration,
 *  including the human/AI split (Phase 4.2). */
function luIntroMsg() {
  const g = LUG.game;
  const seats = g ? LudoEngine.activeSeats(g) : [];
  const humans = g ? seats.filter(s => g.players[s].kind === 'human') : [];
  const ais = g ? seats.filter(s => g.players[s].kind === 'ai') : [];
  const names = seats.map(s => luSeatName(s)).join(', ');
  const you = humans.length === 1 && humans[0] === 0
    ? 'You are <b>Red</b>. '
    : `Humans: <b>${humans.map(s => luSeatName(s)).join(', ')}</b>. `;
  const opp = ais.length === 0 ? ''
    : ais.length === 1 ? `${luSeatName(ais[0])} is AI. `
    : `AI: ${ais.map(s => luSeatName(s)).join(', ')}. `;
  return `New game — <b>${names}</b> (${seats.length} players). `
    + you + opp + 'Roll a 6 to release a token.';
}

/* ---- Phase 4.1: player-count configuration ------------------------------------
   Deterministic seat policy (mirrors LudoEngine.createGame docs):
     4 players → seats [0,1,2,3]  Red, Green, Yellow, Blue (classic default)
     3 players → seats [0,1,2]     Red, Green, Yellow
     2 players → seats [0,2]       Red, Yellow — diagonal corners, the
                                   standard two-seat arrangement (opposite
                                   starts, maximum racing distance).
   Seat 0 is always active and always the human; every other active seat is
   an AI in this phase. Inactive seats keep their colour geometry on the
   board but are dimmed and never act. */
const LU_SEATS_FOR = { 2: [0, 2], 3: [0, 1, 2], 4: [0, 1, 2, 3] };

function luSeatsForCount(n) {
  return LU_SEATS_FOR[n] || LU_SEATS_FOR[4];
}

/** Build the kinds array for createGame from the current configuration:
 *  the FIRST `humans` seats in the ACTIVE-seat order are human, every
 *  remaining active seat is AI. Inactive seats keep 'ai' (irrelevant —
 *  they never receive a turn).
 *
 *  Deterministic assignment policy (Phase 4.2, documented):
 *    3P · 2H → active [0,1,2] → humans {0,1}, AI {2}
 *    2P · 2H → active [0,2]   → humans {0,2} (both human)
 *    4P · 1H → active [0,1,2,3] → human {0}, AI {1,2,3}
 *  Seat 0 is always first in the active order → always human.
 *
 *  Invalid requests (humans < 1 or > active count) are rejected safely by
 *  clamping to the nearest legal value. */
function luKindsForConfig(n, humans) {
  const seats = luSeatsForCount(n);
  const h = Math.min(Math.max(1, Number(humans) || 1), seats.length);  // clamp
  return [0, 1, 2, 3].map((_, i) => {
    const at = seats.indexOf(i);          // position in the active order
    return at !== -1 && at < h ? 'human' : 'ai';
  });
}

/** Legacy Phase 4.1 helper: one human (seat 0), rest AI. */
function luKindsForCount(n) {
  return luKindsForConfig(n, 1);
}

/* ---- new game / restart ------------------------------------------------------- */
function luNewGame() {
  /* NOTE: the view epoch is NOT bumped here — the view stays mounted and
     its listeners must keep working across restarts (the game CLOCK bump
     below already invalidates every pending turn-flow callback). Bumping
     the epoch here would kill all board/control listeners after the very
     first restart (found by the Phase 3 Part D tests). */
  LUG.clock++;                       // kill every pending game-flow callback
  luCancelAnim();                    // + cancel any in-flight rAF glide
  luCancelDiceSpin();                // + stop any tumbling die
  luFxClear();                       // Phase 5.3: drop live FX + their pump
  if (luStackRaf) { cancelAnimationFrame(luStackRaf); luStackRaf = 0; }
  LUG.hoverToken = null;             // stale hover can't outlive a game
  LUG.game = LudoEngine.createGame(
    luKindsForConfig(LUG.seatCount, LUG.humanSeats),
    { seats: luSeatsForCount(LUG.seatCount) });
  LUG.aiRng = LudoEngine.makeRng(Date.now() & 0xffffffff);  // fresh per game
  LUG.phase = 'idle'; LUG.dice = null; LUG.moves = []; LUG.anim = null;
  LUG.settleUntil = 0;
  LUG.nudge = null; clearTimeout(luNudgeTimer);
  luOverlayHide();
  luSetDice(null, false);
  luRenderHud(); luDraw(); luRenderControls();
  luMsg(luIntroMsg());
  luMusicFollow();          // Phase 5.4: fresh game → music follows settings
  /* Phase 5.5: a fresh game created while the LOBBY is open (mount, or
     a config change outside Start) must not begin any turn flow — the
     players have not started yet. luLobbyStart() clears LUG.lobby and
     calls luNewGame again, which then reaches luStartTurn(). */
  if (!LUG.lobby) luStartTurn();
}

/* ==========================================================================
   Phase 5.5 — SETUP LOBBY LOGIC
   --------------------------------------------------------------------------
   The lobby is a pure UI state (LUG.lobby). It never touches the engine:
   configuration changes update LUG + the preview immediately, and only
   Start Game (or ✕) calls luNewGame() with the chosen settings.
   Existing behaviour preserved: the selectors keep their Phase 4 ids
   (#luSeatPick/#luHumanPick/#luDiffPick) and apply-on-restart semantics —
   they now live inside the lobby card instead of the control strip.
   ========================================================================== */

/** Rebuild the seat preview from the CURRENT configuration. Called on
 *  every config change → the preview is always live. */
function luLobRenderPrev() {
  const host = lu$('luLobPrev'); if (!host) return;
  const seats = luSeatsForCount(LUG.seatCount);
  const kinds = luKindsForConfig(LUG.seatCount, LUG.humanSeats);
  const diffLbl = LUG.aiDifficulty
    ? LUG.aiDifficulty[0].toUpperCase() + LUG.aiDifficulty.slice(1) : 'Classic';
  host.innerHTML = [0, 1, 2, 3].map(i => {
    const active = seats.includes(i);
    const kind = kinds[i];
    const col = LU_COLORS[LU_P[i]].fill;
    /* editable name input for HUMAN seats; AI seats show their fixed
       default (colour · level) — nothing to configure. */
    /* the input starts EMPTY (placeholder shows the default) unless a
       custom name exists — typing must never append to a default. */
    const cur = luCleanName(LUG.names && LUG.names[i]);
    const body = kind === 'human'
      ? `<input type="text" class="lu-lob-nm" data-nmseat="${i}"
             value="${cur}" maxlength="14"
             placeholder="${luDefaultName(i)}"
             aria-label="Name for ${luDefaultName(i)} (seat ${i + 1} of 4)">`
      : `<span class="lu-lob-sn">${luSeatName(i)}</span>`;
    return `<div class="lu-lob-seat${active ? '' : ' inactive'}"
      style="--pc:${col}" data-prevseat="${i}"
      aria-label="${luSeatNameRaw(i)}: ${active ? (kind === 'human' ? 'human' : `AI, ${diffLbl}`) : 'inactive seat'}">
      <span class="lu-p-dot" aria-hidden="true"></span>
      <span class="lu-lob-sk ${kind === 'ai' ? 'ai' : 'human'}">${active ? (kind === 'ai' ? `AI · ${diffLbl}` : 'HUMAN') : 'INACTIVE'}</span>
      ${body}
    </div>`;
  }).join('');
}

/** Push LUG config → all lobby controls (radios, mode, preview). */
function luLobSync() {
  const modeBtns = document.querySelectorAll('#luModePick .lu-sp-btn');
  modeBtns.forEach(b => {
    const on = b.dataset.mode === LUG.lobbyMode;
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on ? 'true' : 'false');
  });
  /* Quick Start pins the classic default; Custom unlocks the grid. */
  const cfg = lu$('luLobCfg');
  if (cfg) cfg.classList.toggle('off', LUG.lobbyMode === 'quick');
  /* seat / human / diff groups (Phase 4 ids preserved) */
  document.querySelectorAll('#luSeatPick .lu-sp-btn').forEach(b => {
    const on = Number(b.dataset.seats) === LUG.seatCount;
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on ? 'true' : 'false');
  });
  document.querySelectorAll('#luHumanPick .lu-sp-btn').forEach(b => {
    const n = Number(b.dataset.humans);
    const avail = n >= 1 && n <= LUG.seatCount;
    b.hidden = !avail;
    b.disabled = !avail;
    if (!avail) return;
    const on = n === LUG.humanSeats;
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on ? 'true' : 'false');
  });
  document.querySelectorAll('#luDiffPick .lu-sp-btn').forEach(b => {
    const on = b.dataset.diff === (LUG.aiDifficulty || '');
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on ? 'true' : 'false');
  });
  luLobRenderPrev();
  /* blurb mirrors the live configuration (as in Phase 4) */
  const blurb = lu$('luBlurb');
  if (blurb) {
    const seats = luSeatsForCount(LUG.seatCount);
    const kinds = luKindsForConfig(LUG.seatCount, LUG.humanSeats);
    const who = seats.map(s =>
      `${luSeatName(s)} (${kinds[s] === 'human' ? 'human' : 'AI'})`).join(', ');
    const lvl = LUG.aiDifficulty
      ? ` · AI level: ${LUG.aiDifficulty[0].toUpperCase() + LUG.aiDifficulty.slice(1)}`
      : '';
    blurb.textContent = `Classic Ludo race — ${who}${lvl}. `
      + `Six releases a token; exact count finishes it.`;
  }
}

/** Show the lobby (mount or Restart). Configuration is preserved by
 *  design — nothing resets here. The RUNNING game is frozen exactly the
 * way luNewGame freezes one (clock bump kills every pending turn-flow
 * callback; animations/FX stop), so no AI timer can fire behind the
 * setup card and Start always begins from a clean board. */
function luLobbyShow() {
  const lob = lu$('luLobby'); if (!lob) return;
  LUG.lobby = true;
  /* freeze any live game (Restart mid-turn / mid-animation) */
  LUG.clock++;                       // kill pending turn-flow callbacks
  luCancelAnim();                    // + any in-flight glide
  luCancelDiceSpin();                // + the dice tumble
  luFxClear();                       // Phase 5.3 effects
  if (luStackRaf) { cancelAnimationFrame(luStackRaf); luStackRaf = 0; }
  luMusicStop();                     // Phase 5.4: no music while setting up
  LUG.hoverToken = null;
  clearTimeout(luNudgeTimer);
  LUG.nudge = null; LUG.settleUntil = 0;
  /* reset the visible game to a pristine board for the chosen config */
  LUG.game = LudoEngine.createGame(
    luKindsForConfig(LUG.seatCount, LUG.humanSeats),
    { seats: luSeatsForCount(LUG.seatCount) });
  LUG.phase = 'idle'; LUG.dice = null; LUG.moves = []; LUG.anim = null;
  luSetDice(null, false);
  lob.classList.remove('hidden');
  luOverlayHide();
  luRenderHud(); luDraw(); luRenderControls();
  luLobSync();
  luMsg('Set up your game, then press <b>Start Game</b>.');
  /* roll button is meaningless while setting up */
  const btn = lu$('luRollBtn');
  if (btn) btn.disabled = true;
  const st = lu$('luStartBtn');
  if (st) st.focus({ preventScroll: true });       // keyboard entry point
}

/** Leave the lobby and begin play with the current configuration. */
function luLobbyStart() {
  const lob = lu$('luLobby');
  if (lob) lob.classList.add('hidden');
  LUG.lobby = false;
  luSfx('ui');
  luNewGame();                                      // applies LUG config
}

/* ---- view ------------------------------------------------------------------------
   Mounted by the console router at #funtime/lu.
   ---------------------------------------------------------------------------------- */
function renderLuView() {
  document.getElementById('viewRoot').innerHTML = `
  <div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Ludo</div>
    </div>

    <section class="ft-panel ft-game lu-wide" id="luPanel" aria-label="Ludo classic four-player game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Ludo</div>
          <div class="ft-blurb" id="luBlurb">Classic Ludo race. Six releases a token; exact count finishes it.</div>
        </div>
      </header>

      <div class="lu-panels" id="luPanels" aria-label="Player panels"></div>

      <div class="lu-stage" id="luStageWrap">
        <canvas id="luBoard" aria-label="Ludo board"
                role="img">Your browser does not support the canvas element.</canvas>
        <div class="lu-overlay hidden" id="luOverlay" role="dialog" aria-modal="false" aria-label="Game status">
          <div class="lu-ov-inner">
            <div class="lu-ov-title" id="luOvTitle">Ludo</div>
            <div class="lu-ov-sub" id="luOvSub"></div>
            <div class="lu-ov-stats hidden" id="luOvStats"></div>
            <div class="lu-ov-row" id="luOvRow"></div>
          </div>
        </div>
      </div>

      <div class="lu-controls" role="group" aria-label="Game controls">
        <!-- Phase 5.5: player/human/AI selectors moved into the setup
             lobby (#luLobby); Restart re-opens it with settings kept. -->
        <button type="button" class="lu-dice-btn" id="luRollBtn">
          ${luDieFaceDom(6)}
          <span id="luRollLbl">Roll Dice</span>
        </button>
        <button type="button" class="ft-btn" id="luRestartBtn">Restart</button>
        <button type="button" class="ft-btn si-sound" id="luSoundBtn"
                aria-pressed="true" aria-haspopup="dialog" aria-expanded="false">
          <span class="si-snd-ico" aria-hidden="true">♪</span><span id="luSoundLbl">sound on</span>
        </button>
        <button type="button" class="ft-btn" id="luFsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn si-exit" id="luExit">Back to Games</button>
      </div>

      <!-- Phase 5.4: audio settings popover (master/music/sfx toggles +
           volumes). Anchored under the controls, dismissed by outside
           click / Esc; persists via localStorage. -->
      <div class="lu-audio-pop hidden" id="luAudioPop" role="dialog"
           aria-label="Audio settings" aria-modal="false">
        <div class="lu-au-row lu-au-head">
          <span class="lu-au-title">AUDIO</span>
          <button type="button" class="lu-au-x" id="luAudioClose"
                  aria-label="Close audio settings">✕</button>
        </div>
        <div class="lu-au-row">
          <label class="lu-au-l" for="luAuMasterV">Master volume</label>
          <input type="range" id="luAuMasterV" class="lu-au-slider"
                 min="0" max="100" step="1" aria-label="Master volume">
          <span class="lu-au-val" id="luAuMasterVal"></span>
        </div>
        <div class="lu-au-row">
          <label class="lu-au-l" for="luAuMusicV">Music volume</label>
          <input type="range" id="luAuMusicV" class="lu-au-slider"
                 min="0" max="100" step="1" aria-label="Music volume">
          <span class="lu-au-val" id="luAuMusicVal"></span>
        </div>
        <div class="lu-au-row">
          <label class="lu-au-l" for="luAuSfxV">Effects volume</label>
          <input type="range" id="luAuSfxV" class="lu-au-slider"
                 min="0" max="100" step="1" aria-label="Sound effects volume">
          <span class="lu-au-val" id="luAuSfxVal"></span>
        </div>
        <div class="lu-au-row lu-au-toggles">
          <button type="button" class="lu-au-toggle" id="luAuMusicT"
                  aria-pressed="true"><span class="lu-au-dot" aria-hidden="true"></span>Music</button>
          <button type="button" class="lu-au-toggle" id="luAuSfxT"
                  aria-pressed="true"><span class="lu-au-dot" aria-hidden="true"></span>Effects</button>
        </div>
      </div>

      <div class="lu-msg" id="luMsg" aria-live="polite"></div>

      <!-- ============================================================
           Phase 5.5 — GAME SETUP LOBBY
           Shown on mount and on Restart; hidden once the game starts.
           The board stays visible behind it (dimmed) so the layout
           never reflows. All controls are real buttons/inputs with
           radiogroup semantics for keyboard + screen-reader access.
           ============================================================ -->
      <div class="lu-lobby hidden" id="luLobby" role="dialog"
           aria-label="Game setup" aria-modal="false">
        <div class="lu-lob-card" id="luLobCard">
          <div class="lu-lob-head">
            <span class="lu-lob-title">GAME SETUP</span>
            <button type="button" class="lu-au-x" id="luLobClose"
                    aria-label="Use current settings and start">✕</button>
          </div>

          <!-- 1 · mode -->
          <div class="lu-lob-sec">
            <span class="lu-lob-k" id="luLobModeK">MODE</span>
            <div class="lu-seatpick" id="luModePick" role="radiogroup"
                 aria-labelledby="luLobModeK" aria-label="Game mode">
              <button type="button" class="lu-sp-btn" data-mode="quick" role="radio"
                      aria-checked="true" title="4 players, you vs three AI, classic rules">Quick Start</button>
              <button type="button" class="lu-sp-btn" data-mode="custom" role="radio"
                      aria-checked="false" title="Choose players, humans and AI level">Custom Game</button>
            </div>
          </div>

          <!-- 2–4 · players / humans / AI level (Custom only) -->
          <div class="lu-lob-cfg" id="luLobCfg">
            <div class="lu-lob-sec">
              <span class="lu-lob-k" id="luLobSeatK">PLAYERS</span>
              <div class="lu-seatpick" id="luSeatPick" role="radiogroup"
                   aria-labelledby="luLobSeatK" aria-label="Number of players">
                <button type="button" class="lu-sp-btn" data-seats="2" role="radio" aria-checked="false">2P</button>
                <button type="button" class="lu-sp-btn" data-seats="3" role="radio" aria-checked="false">3P</button>
                <button type="button" class="lu-sp-btn" data-seats="4" role="radio" aria-checked="true">4P</button>
              </div>
            </div>
            <div class="lu-lob-sec">
              <span class="lu-lob-k" id="luLobHumK">HUMANS</span>
              <div class="lu-seatpick" id="luHumanPick" role="radiogroup"
                   aria-labelledby="luLobHumK" aria-label="How many of the active players are human">
                <button type="button" class="lu-sp-btn" data-humans="1" role="radio" aria-checked="true">1H</button>
                <button type="button" class="lu-sp-btn" data-humans="2" role="radio" aria-checked="false">2H</button>
                <button type="button" class="lu-sp-btn" data-humans="3" role="radio" aria-checked="false">3H</button>
                <button type="button" class="lu-sp-btn" data-humans="4" role="radio" aria-checked="false">4H</button>
              </div>
            </div>
            <div class="lu-lob-sec">
              <span class="lu-lob-k" id="luLobDiffK">AI LEVEL</span>
              <div class="lu-seatpick" id="luDiffPick" role="radiogroup"
                   aria-labelledby="luLobDiffK" aria-label="AI difficulty">
                <button type="button" class="lu-sp-btn" data-diff="" role="radio" aria-checked="true">Classic</button>
                <button type="button" class="lu-sp-btn" data-diff="easy" role="radio" aria-checked="false">Easy</button>
                <button type="button" class="lu-sp-btn" data-diff="medium" role="radio" aria-checked="false">Medium</button>
                <button type="button" class="lu-sp-btn" data-diff="hard" role="radio" aria-checked="false">Hard</button>
              </div>
            </div>
          </div>

          <!-- 5+6 · seat preview with editable names -->
          <div class="lu-lob-sec lu-lob-prev-sec">
            <span class="lu-lob-k" id="luLobPrevK">SEATS</span>
            <div class="lu-lob-prev" id="luLobPrev"
                 aria-labelledby="luLobPrevK" aria-label="Seat preview"></div>
          </div>

          <!-- 7 · start -->
          <div class="lu-lob-actions">
            <button type="button" class="lu-ov-btn primary lu-lob-start" id="luStartBtn">
              Start Game
            </button>
          </div>
        </div>
      </div>

      <div class="lu-help">
        <span>click a glowing token to move</span><span>stars are safe</span><span>6 = extra roll</span><span>F fullscreen</span>
      </div>
    </section>
  </div></div>`;

  LUG.epoch++;                       // fresh view epoch for these handlers
  const myEpoch = LUG.epoch;
  LUG.clock++;                       // also invalidate any pre-mount game clock
  LUG.cleanup = [];
  LUG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const luOnLocal = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    LUG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };
  LUG._on = luOnLocal;

  LUG.canvas = lu$('luBoard');
  LUG.ctx = LUG.canvas.getContext('2d');

  /* ---- controls ---- */
  luOnLocal(lu$('luRollBtn'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    const g = LUG.game;
    if (!g || g.winner !== null) return;
    if (g.players[g.turn].kind !== 'human') return;
    if (LUG.phase !== 'idle') return;
    if (LUA.ctx && LUA.ctx.state === 'suspended') LUA.ctx.resume().catch(() => {});
    luDoRoll();
  });

  luOnLocal(lu$('luRestartBtn'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    luSfx('ui');
    /* Phase 5.5: Restart opens the setup lobby with the configuration
       (players / humans / AI level / names) preserved; Start applies it
       to a fresh game. */
    luLobbyShow();
  });

  /* ---- Phase 5.4: audio settings popover --------------------------------
     The sound button is now a master mute + the entry point to the full
     settings panel. State lives in LUG.audio (persisted); the UI is a
     plain read/write of that object, so refresh/restart both restore
     exactly what was saved. */
  const auPop = lu$('luAudioPop');
  const auSyncUi = () => {
    const a = LUG.audio;
    const on = a.m > 0 && (a.sx || a.mu);
    LUG.soundOn = on;                                // legacy gate
    const b = lu$('luSoundBtn');
    if (b) {
      b.setAttribute('aria-pressed', String(on));
      b.classList.toggle('on', on);
      const lbl = lu$('luSoundLbl');
      if (lbl) lbl.textContent = on ? 'sound on' : 'muted';
    }
    const set = (id, v) => { const el = lu$(id); if (el) el.value = String(Math.round(v * 100)); };
    const txt = (id, v) => { const el = lu$(id); if (el) el.textContent = Math.round(v * 100) + '%'; };
    set('luAuMasterV', a.m);  txt('luAuMasterVal', a.m);
    set('luAuMusicV', a.mv);  txt('luAuMusicVal', a.mv);
    set('luAuSfxV', a.sv);    txt('luAuSfxVal', a.sv);
    const tg = (id, onn) => { const el = lu$(id); if (el) el.setAttribute('aria-pressed', String(!!onn)); };
    tg('luAuMusicT', a.mu); tg('luAuSfxT', a.sx);
  };
  const auOpen = () => {
    if (!auPop) return;
    auPop.classList.remove('hidden');
    lu$('luSoundBtn').setAttribute('aria-expanded', 'true');
    luSfx('ui');
  };
  const auClose = () => {
    if (!auPop) return;
    auPop.classList.add('hidden');
    const b = lu$('luSoundBtn');
    if (b) b.setAttribute('aria-expanded', 'false');
  };
  const auToggleOpen = () => auPop && auPop.classList.contains('hidden') ? auOpen() : auClose();

  luOnLocal(lu$('luSoundBtn'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    /* First click on a MUTED state unmutes; otherwise it opens the
       settings popover (mute itself is the master slider at 0). */
    if (!LUG.soundOn && LUG.audio.m === 0) {
      LUG.audio.m = 0.7;
      luAudioApplyVolumes(); luAudioSave(); auSyncUi();
      luSfx('ui');                                    // audible confirmation
      return;
    }
    auToggleOpen();
  });
  luOnLocal(lu$('luAudioClose'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    auClose();
  });

  /* volume sliders — apply live, persist debounced-free (cheap writes) */
  const auSlider = (inputId, key, valId) => {
    const el = lu$(inputId); if (!el) return;
    luOnLocal(el, 'input', () => {
      LUG.audio[key] = Math.max(0, Math.min(1, Number(el.value) / 100));
      luAudioApplyVolumes(); luAudioSave(); auSyncUi();
      luMusicFollow();                               // music may start/stop
    });
  };
  auSlider('luAuMasterV', 'm', 'luAuMasterVal');
  auSlider('luAuMusicV', 'mv', 'luAuMusicVal');
  auSlider('luAuSfxV', 'sv', 'luAuSfxVal');

  /* toggles — instant mute via the gain graph (no clicks, no restart) */
  luOnLocal(lu$('luAuMusicT'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    LUG.audio.mu = LUG.audio.mu ? 0 : 1;
    luAudioApplyVolumes(); luAudioSave(); auSyncUi();
    luMusicFollow();
    luSfx('ui');
  });
  luOnLocal(lu$('luAuSfxT'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    LUG.audio.sx = LUG.audio.sx ? 0 : 1;
    luAudioApplyVolumes(); luAudioSave(); auSyncUi();
    luSfx('ui');                                      // plays only if now on
  });

  /* dismiss: outside click (Escape stays owned by the shared session
     controller — it exits the game view for every Fun Time game, and
     hijacking it here would break that global contract). */
  luOnLocal(document, 'pointerdown', e => {
    if (myEpoch !== LUG.epoch) return;
    if (!auPop || auPop.classList.contains('hidden')) return;
    if (auPop.contains(e.target) || lu$('luSoundBtn').contains(e.target)) return;
    auClose();
  });

  auSyncUi();                                        // paint persisted state

  luOnLocal(lu$('luFsBtn'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    if (typeof ftToggleFullscreen === 'function') ftToggleFullscreen('lu', 'lu-fs', null);
  });
  /* Phase 5.6: reflect genuine fullscreen state onto the body class and
     the button label. Without this, the BUTTON and F-KEY paths entered
     native fullscreen but never applied the .lu-fs layout (board stayed
     left-aligned in a narrow inline panel) and the button kept saying
     "Fullscreen" with aria-pressed="false" while actually full screen.
     Same convention as Snakes & Ladders; ftFsReflect also clears every
     other game's fs class defensively. */
  const luFsChanged = () => {
    if (myEpoch !== LUG.epoch) return;
    if (typeof ftFsReflect === 'function') {
      ftFsReflect('lu', 'lu-fs', '#luFsBtn');
    }
    luResize();                       // refit the board to the new viewport
  };
  luOnLocal(document, 'fullscreenchange', luFsChanged);
  luOnLocal(document, 'webkitfullscreenchange', luFsChanged);
  if (typeof ftFsSupported === 'function' && !ftFsSupported()) {
    const b = lu$('luFsBtn'); if (b) b.style.display = 'none';
  }

  luOnLocal(lu$('luExit'), 'click', () => { location.hash = '#funtime'; });

  /* ---- Phase 4.1: player-count selector ----
     Selecting a count marks it in the UI and applies it by starting a fresh
     game immediately (the only way a count change can take effect — mid-game
     seat changes would corrupt the running game). Restart keeps the choice. */
  /* Phase 5.5: the selectors live in the lobby now; this hook simply
     re-syncs the whole lobby (radios + preview + blurb). Kept for the
     non-lobby apply path above. */
  const luSyncSeatPick = () => luLobSync();
  luOnLocal(lu$('luSeatPick'), 'click', e => {
    if (myEpoch !== LUG.epoch) return;
    const b = e.target.closest('.lu-sp-btn');
    if (!b || b.disabled) return;
    const n = Number(b.dataset.seats);
    if (n !== 2 && n !== 3 && n !== 4) return;
    if (n === LUG.seatCount) return;                 // no change → nothing
    LUG.seatCount = n;
    /* Keep the human count inside the new range (deterministic clamp). */
    LUG.humanSeats = Math.min(LUG.humanSeats, n);
    /* Phase 5.5: in the lobby only the PREVIEW updates — the engine
       starts fresh on Start Game. Outside the lobby (should not happen
       now, kept for safety) the old apply-immediately behaviour runs. */
    if (LUG.lobby) { luLobSync(); luSfx('ui'); }
    else { luNewGame(); luSyncSeatPick(); }
  });
  /* Phase 4.2: human-seat count. Same apply semantics as the player count:
     takes effect via a fresh game; clicking the already-selected option is
     a no-op so it can never restart a running game. */
  luOnLocal(lu$('luHumanPick'), 'click', e => {
    if (myEpoch !== LUG.epoch) return;
    const b = e.target.closest('.lu-sp-btn');
    if (!b || b.disabled) return;
    const n = Number(b.dataset.humans);
    if (!(n >= 1 && n <= 4)) return;
    if (n === LUG.humanSeats) return;                // no change → nothing
    LUG.humanSeats = Math.min(n, LUG.seatCount);     // reject invalid safely
    if (LUG.lobby) { luLobSync(); luSfx('ui'); }
    else { luNewGame(); luSyncSeatPick(); }
  });
  /* Phase 4.3/4.5: AI difficulty. '' (Classic), 'easy', 'medium' and
     'hard' are selectable. Applies via fresh game, same as the other
     selectors; re-clicking the current choice is a no-op. Human-only
     games ignore this entirely (no AI seats exist). */
  luOnLocal(lu$('luDiffPick'), 'click', e => {
    if (myEpoch !== LUG.epoch) return;
    const b = e.target.closest('.lu-sp-btn');
    if (!b || b.disabled) return;
    const d = b.dataset.diff;
    if (d !== '' && d !== 'easy' && d !== 'medium' && d !== 'hard') return;   // implemented levels only
    if (d === (LUG.aiDifficulty || '')) return;      // no change → nothing
    LUG.aiDifficulty = d === '' ? null : d;
    if (LUG.lobby) { luLobSync(); luSfx('ui'); }
    else { luNewGame(); luSyncSeatPick(); }
  });

  /* ---- Phase 5.5: lobby wiring ---- */
  luOnLocal(lu$('luModePick'), 'click', e => {
    if (myEpoch !== LUG.epoch) return;
    const b = e.target.closest('.lu-sp-btn');
    if (!b || b.disabled) return;
    const m = b.dataset.mode;
    if (m !== 'quick' && m !== 'custom') return;
    if (m === LUG.lobbyMode) return;
    LUG.lobbyMode = m;
    if (m === 'quick') {
      /* Quick Start pins the classic default configuration. */
      LUG.seatCount = 4; LUG.humanSeats = 1; LUG.aiDifficulty = null;
    }
    luLobSync(); luSfx('ui');
  });

  /* name inputs: live-clean + persist in LUG.names (survives restart) */
  luOnLocal(lu$('luLobPrev'), 'input', e => {
    if (myEpoch !== LUG.epoch) return;
    const inp = e.target.closest('.lu-lob-nm');
    if (!inp) return;
    const seat = Number(inp.dataset.nmseat);
    if (!(seat >= 0 && seat <= 3)) return;
    /* store the CLEANED value; the input itself keeps what the user
       typed so editing never fights them (validation happens on blur
       and at resolve time). */
    LUG.names[seat] = luCleanName(inp.value);
  });
  /* blur: normalise the field to the cleaned value (or clear to the
     placeholder default when empty/too long) */
  luOnLocal(lu$('luLobPrev'), 'focusout', e => {
    if (myEpoch !== LUG.epoch) return;
    const inp = e.target.closest('.lu-lob-nm');
    if (!inp) return;
    const seat = Number(inp.dataset.nmseat);
    const clean = luCleanName(inp.value);
    LUG.names[seat] = clean;
    inp.value = clean || luDefaultName(seat);
  });

  luOnLocal(lu$('luStartBtn'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    luLobbyStart();
  });
  /* ✕ = start with current settings (same as Start, one less click) */
  luOnLocal(lu$('luLobClose'), 'click', () => {
    if (myEpoch !== LUG.epoch) return;
    luLobbyStart();
  });

  luSyncSeatPick();

  /* ---- overlay actions ---- */
  luOnLocal(lu$('luOvRow'), 'click', e => {
    if (myEpoch !== LUG.epoch) return;
    const b = e.target.closest('.lu-ov-btn');
    if (!b) return;
    if (b.dataset.act === 'again') luNewGame();
    else if (b.dataset.act === 'menu') location.hash = '#funtime';
  });

  /* ---- board picking ---- */
  luOnLocal(LUG.canvas, 'pointerdown', e => {
    if (myEpoch !== LUG.epoch) return;
    const pt = luCanvasPoint(e);
    const hit = luHitTest(pt.x, pt.y);
    if (hit) luPick(hit);
  });

  luOnLocal(LUG.canvas, 'pointermove', e => {
    if (myEpoch !== LUG.epoch) return;
    const pt = luCanvasPoint(e);
    const hit = luHitTest(pt.x, pt.y);
    const pickable = hit && LUG.phase === 'awaiting' &&
      LUG.game.players[LUG.game.turn].kind === 'human' &&
      LUG.moves.some(m => m.token === hit.i && LUG.game.turn === hit.p);
    LUG.canvas.style.cursor = pickable ? 'pointer' : 'default';
    /* Phase 5.3: remember the hovered legal token for the selection
       keyline. Only a CHANGE redraws — moving within the same token
       must not spam frames (the board is otherwise static here). */
    const nextHover = pickable ? { p: hit.p, i: hit.i } : null;
    const changed = !!nextHover !== !!LUG.hoverToken ||
      (nextHover && LUG.hoverToken &&
        (nextHover.p !== LUG.hoverToken.p || nextHover.i !== LUG.hoverToken.i));
    if (changed) {
      LUG.hoverToken = nextHover;
      if (!LUG.anim && !LUG.fx.length) luDraw();
    }
  });

  /* ---- keyboard ----
     Phase 5.5: while the setup lobby is open, gameplay keys are inert
     (only fullscreen still works); typing names must never trigger them. */
  luOnLocal(window, 'keydown', e => {
    if (myEpoch !== LUG.epoch) return;
    if (e.target && (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA')) return;
    if (e.key === 'f' || e.key === 'F') {
      if (typeof ftToggleFullscreen === 'function') ftToggleFullscreen('lu', 'lu-fs', null);
    } else if (!LUG.lobby) {
      if (e.key === 'r' || e.key === 'R') {
        luLobbyShow();                 // Restart → setup (config kept)
      } else if (e.key === ' ' && LUG.game && LUG.game.winner === null) {
        const g = LUG.game;
        if (g.players[g.turn].kind === 'human' && LUG.phase === 'idle') {
          e.preventDefault();
          luDoRoll();
        }
      }
    }
  });

  /* ---- resize ---- */
  luOnLocal(window, 'resize', () => { if (myEpoch === LUG.epoch) luResize(); });

  /* ---- boot ----
     Phase 5.5: mount into the SETUP LOBBY. A game object still exists
     (the board renders behind the card and panels need state), but no
     turn flow starts until Start Game. Configuration persists across
     remounts because it lives on LUG (module scope). */
  luResize();
  LUG.game = LudoEngine.createGame(
    luKindsForConfig(LUG.seatCount, LUG.humanSeats),
    { seats: luSeatsForCount(LUG.seatCount) });
  LUG.aiRng = LudoEngine.makeRng(Date.now() & 0xffffffff);
  LUG.phase = 'idle'; LUG.dice = null; LUG.moves = []; LUG.anim = null;
  luRenderHud(); luRenderControls();
  luMsg('Set up your game, then press <b>Start Game</b>.');
  luLobbyShow();

  /* Shared session lifecycle: automatic fullscreen from the card click,
     Esc → back to Fun Time, dashboard chrome hidden while playing. */
  if (typeof ftSessionAfterMount === 'function') ftSessionAfterMount('lu', 'lu-fs');
  if (typeof ftSessionBind === 'function') ftSessionBind(luOnLocal, () => luResize());
  /* The session classes above are what give #luPanel its viewport height;
     refit the board now that they have landed. */
  luResize();
}

/** Static die markup for the roll button (initial face). */
function luDieFaceDom(n) {
  /* n=0 renders a blank die (no pips) — used for non-current seats. */
  const on = LU_PIPS[n] || [];
  let dots = '';
  for (let i = 0; i < 9; i++) dots += `<i style="grid-area:${Math.floor(i / 3) + 1}/${(i % 3) + 1};${on.includes(i) ? '' : 'visibility:hidden'}"></i>`;
  return `<span class="lu-die" aria-hidden="true">${dots}</span>`;
}

/* ---- diagnostics hook (browser tests drive the game through it) -------------- */
const LUD = {
  /** Called with each dice value; tests override to force rolls. */
  onRoll(v) { luResolveRoll(v); },
  /**
   * Force a specific dice value for the CURRENT roll (test hook).
   *
   * State-machine protections are NEVER bypassed (Phase 3 Part D):
   *   • rejected while a roll is in flight (`rolling`) — the pending
   *     tumble owns this turn's result;
   *   • rejected while a move animates (`moving`) — the engine commit
   *     at luMoveDone() is the only authority;
   *   • rejected once the game is over or the view is dead.
   * Legitimate testing use (forcing a value from `idle`) is preserved.
   */
  force(v) {
    const g = LUG.game;
    if (!g || g.winner !== null) return false;
    if (!LUG.viewAlive()) return false;
    /* Only genuine die faces 1..6 — anything else (0, 7, -1, null, NaN,
       strings, non-integers) would flow into legalMoves/applyMove as a
       bogus step count: dice 0 produced a "move" from 5 to 5 that still
       captured an opponent standing there (found by the Phase 3 Part F
       audit). The engine must never see a forged value. */
    if (!Number.isInteger(v) || v < 1 || v > 6) return false;
    if (LUG.phase === 'rolling') return false;   // a roll is already in flight
    if (LUG.phase === 'moving') return false;    // a move is animating
    if (LUG.phase === 'over') return false;
    if (LUG.phase !== 'idle') return false;      // awaiting: pick, don't re-roll
    /* Only the HUMAN player's turn may be forced. Without this, a forced
       value resolved a roll on Green's AI turn — the hook must never act
       for an AI (its roll belongs to luAiRoll/the engine pipeline). */
    if (g.players[g.turn].kind !== 'human') return false;
    /* A no-move resolve (luResolveRoll's empty-moves path) leaves phase
       'idle' during its LU_T.eventMs settle window, so rapid force()
       calls used to inject several rolls at once (each consumed, each
       scheduling its own overlapping settle timer). The settle records
       its deadline; a forced roll is refused only while that window is
       genuinely open (self-expiring, so an externally killed timer can
       never leave a stale block behind). */
    if (LUG.settleUntil > performance.now()) return false;
    LUD.onRoll(v);
    return true;
  },
  /** Shrink all turn-flow timings so full games finish in seconds (tests). */
  fast() {
    LU_T.rollSpin = 30; LU_T.aiThink = 40; LU_T.aiPick = 30;
    LU_T.stepMs = 12; LU_T.resolveMs = 20; LU_T.eventMs = 25;
  },
};


/* ---- teardown -------------------------------------------------------------------- */
function luTeardown() {
  LUG.epoch++;                      // view gone: UI handlers self-invalidate
  LUG.clock++;                      // game clock: pending timers self-kill
  luCancelAnim();                   // cancel any pending animation frame
  luCancelDiceSpin();               // + the dice tumble rAF chain
  luFxClear();                      // Phase 5.3: FX pump + records
  if (luStackRaf) { cancelAnimationFrame(luStackRaf); luStackRaf = 0; }
  for (const off of LUG.cleanup) { try { off(); } catch { /* gone */ } }
  LUG.cleanup = [];
  LUG.game = null; LUG.anim = null; LUG.nudge = null;
  LUG.hoverToken = null;
  clearTimeout(luNudgeTimer);
  LUG.settleUntil = 0;
  LUG.phase = 'idle'; LUG.dice = null; LUG.moves = [];
  LUG.ctx = null; LUG.canvas = null;
  luAudioTeardown();                // Phase 5.4: music + context, once
  if (typeof ftExitFullscreenQuiet === 'function') ftExitFullscreenQuiet();
  document.body.classList.remove('lu-fs');
  if (typeof ftSessionRestore === 'function') ftSessionRestore();
}

/* Expose for the console shell (router + tests). Mirrors chess-game.js. */
window.renderLuView = renderLuView;
window.luTeardown = luTeardown;
window.LUG = LUG;
window.LUD = LUD;
window.LUA = LUA;                    // Phase 5.4: audio state (read-only tests)
/* Phase 5.3 test hooks: the stack-ease map and pump id are exposed for
   the animation verification script (read-only inspection). */
window.LU_STACK_EASE = LU_STACK_EASE;
Object.defineProperty(window, 'luStackRaf', { get: () => luStackRaf });
/* Geometry test hooks (Phase 3): the same single-source-of-truth helpers
   the renderer and hit-tester use, so browser tests can verify that
   drawing coordinates and clicking coordinates agree exactly. */
window.__luTokenPoint = luTokenPoint;
window.__luHitTest = luHitTest;
window.__luStackRadius = luStackRadius;
window.__luBuildStacks = luBuildStacks;
window.__luDraw = luDraw;
window.__luIllegalPick = luIllegalPick;
