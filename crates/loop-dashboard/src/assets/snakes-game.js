/* ============================================================
   FUN TIME · GAME 10 — SNAKES & LADDERS · UI LAYER
   Route: #funtime/sn · Namespace: SN / sn* functions / .sn- CSS.

   Renders the SnakesEngine state and owns NOTHING about the rules:
   every move, snake slide and ladder climb flows through the engine.
   The UI may never bypass it.

   Turn lifecycle (single controller, epoch-guarded):
     START TURN → ROLL (animated dice) → STEP ANIMATION →
     SNAKE SLIDE / LADDER CLIMB → CHECK WINNER → NEXT PLAYER

   One `SNG.epoch` integer invalidates every stale async continuation:
   restart, teardown or route-change bumps it, so no old timer can roll
   a dice, move a token or hand the turn on for a dead game.

   Mirrors the other Fun Time games exactly (ludo/fruitninja/timberman):
   namespaced CSS, snOn() listener registry, ftSessionAfterMount /
   ftSessionBind shared fullscreen+Esc lifecycle, zero DOM leakage.

   Rendering: pure canvas (no external assets). The board, snakes and
   ladders are drawn procedurally with the dashboard's design tokens.
   ============================================================ */

/* ---- Injected styles (namespaced; no global selectors touched) ---- */
(function snInjectCss() {
  if (document.getElementById('sn-style')) return;
  const st = document.createElement('style');
  st.id = 'sn-style';
  st.textContent = `
/* ============================================================
   PREMIUM LAYOUT — one compact command bar on top, then a
   full-height play area: board (primary focus) + info sidebar.
   Every section keeps its original ID so tests/UI stay stable.
   ============================================================ */
.sn-wide{max-width:1180px}

/* ---- command bar: status chips + setup + controls in ONE row ---- */
.sn-topbar{
  display:flex;flex-direction:column;gap:10px;padding:2px 0 0;
}
.sn-bar-row{display:flex;align-items:center;gap:10px;flex-wrap:wrap;min-width:0}
.sn-hud{display:flex;gap:8px;flex-wrap:wrap;align-items:center;padding:0}
.sn-stat{display:inline-flex;align-items:center;gap:8px;border:1px solid var(--line2);border-radius:10px;
  padding:6px 12px;background:rgba(0,0,0,.32);font:600 11px Sora;color:#aab6c8}
.sn-stat .k{text-transform:uppercase;letter-spacing:.12em;font-size:9.5px;color:#7f8ca1}
.sn-stat .v{font:800 15px "JetBrains Mono",monospace;color:#eaf2ff;min-width:2ch;text-align:right}
#snTurnChip{display:inline-flex;align-items:center;gap:8px;border:1px solid var(--line2);border-radius:10px;
  padding:6px 12px;background:rgba(0,0,0,.32);font:600 11px Sora;color:#aab6c8;
  border-color:rgba(94,242,192,.28);background:rgba(94,242,192,.06)}
#snTurnDot{width:10px;height:10px;border-radius:50%;background:#ff5d5d;box-shadow:0 0 10px currentColor;flex:none}

/* ---- layout: board left (primary), side rail right ---- */
.sn-layout{display:grid;grid-template-columns:minmax(0,1fr) 320px;gap:16px;padding:4px 0 0;align-items:stretch;min-height:0}
@media(max-width:1020px){.sn-layout{grid-template-columns:1fr}}
.sn-stage{position:relative;border:1px solid var(--line2);border-radius:16px;overflow:hidden;
  background:
    radial-gradient(120% 90% at 20% 0%,rgba(110,168,255,.07),transparent 55%),
    radial-gradient(100% 80% at 85% 100%,rgba(94,242,192,.05),transparent 50%),
    linear-gradient(180deg,#0b1322 0%,#0e1a2e 60%,#12233b 100%);
  user-select:none;-webkit-user-select:none;touch-action:manipulation;
  display:flex;align-items:center;justify-content:center;min-height:0}
#snBoard{display:block;width:auto;height:auto;max-width:100%;max-height:100%}
/* Inline play while the session hides dashboard chrome: the panel owns the
   viewport height, so the layout STRETCHES to fill it. This gives the stage
   a definite height for snResize()'s square fit — without it the board would
   size from its own (circular) content height and stay tiny. Same trick as
   Ludo's #luStageWrap rule (applies in EVERY playing state). */
body.ft-playing #snPanel{height:100vh;box-sizing:border-box;justify-content:flex-start}
body.ft-playing #snLayout{
  flex:1;min-height:0;display:flex;flex-direction:row;align-items:stretch;gap:16px;
}
body.ft-playing .sn-stage{
  flex:1 1 auto;min-width:0;min-height:0;
  display:flex;align-items:center;justify-content:center;
}
body.ft-playing .sn-side{flex:0 0 320px;max-width:320px;min-height:0;overflow-y:auto}
@media(max-width:1020px){
  body.ft-playing #snLayout{flex-direction:column;overflow-y:auto;align-items:stretch}
  body.ft-playing .sn-stage{flex:0 0 auto}
  body.ft-playing .sn-side{flex:0 0 auto;max-width:none;overflow:visible}
  /* Mobile: cap the stage so the dice + players remain reachable without
     scrolling far past the board. The board still fills the width. */
  @media(max-width:560px){
    body.ft-playing .sn-stage{max-height:56vh}
  }
}
.sn-overlay{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;
  background:radial-gradient(ellipse at center,rgba(6,10,18,.62) 0%,rgba(6,10,18,.9) 100%);
  backdrop-filter:blur(3px);z-index:5;text-align:center;padding:18px}
.sn-overlay.hidden{display:none}
.sn-ov-inner{max-width:560px;width:100%}
.sn-ov-title{font:800 clamp(26px,4vw,40px) Sora;letter-spacing:-.02em;color:#f2f7ff;margin-bottom:8px;
  text-shadow:0 0 30px rgba(94,242,192,.25)}
.sn-ov-sub{font:500 14px Sora;color:#9fb0c7;line-height:1.65;margin-bottom:18px}
.sn-ov-sub b{color:#e5edf9}
.sn-ov-stats{display:flex;gap:14px;justify-content:center;flex-wrap:wrap;margin:0 0 16px}
.sn-ov-stat{border:1px solid var(--line2);border-radius:10px;padding:8px 14px;background:rgba(0,0,0,.35)}
.sn-ov-stat .k{font:600 10px Sora;letter-spacing:.12em;text-transform:uppercase;color:#7f8ca1}
.sn-ov-stat .v{font:800 20px "JetBrains Mono",monospace;color:#eaf2ff}

/* ---- winner celebration (Task 3) ---- */
.sn-ov-winner{display:flex;align-items:center;justify-content:center;gap:12px;margin:2px 0 6px}
.sn-ov-crown{width:44px;height:44px;flex:none;filter:drop-shadow(0 0 14px rgba(255,209,102,.55))}
.sn-ov-name{font:800 clamp(30px,5vw,52px) Sora;letter-spacing:-.02em;color:#ffd166;
  text-shadow:0 0 34px rgba(255,209,102,.45)}
.sn-ov-name.humanWin{color:#5ef2c0;text-shadow:0 0 34px rgba(94,242,192,.45)}
@keyframes snPopIn{0%{transform:scale(.4);opacity:0}60%{transform:scale(1.08);opacity:1}100%{transform:scale(1)}}
.sn-ov-winner.pop{animation:snPopIn .55s cubic-bezier(.2,.9,.3,1.2) both}

/* match summary table (Task 4) */
.sn-ov-table{margin:4px auto 14px;border:1px solid var(--line2);border-radius:12px;overflow:hidden;
  background:rgba(0,0,0,.35);font:500 12.5px Sora;color:#c7d2e2;max-width:520px;width:100%}
.sn-ov-table table{border-collapse:collapse;width:100%}
.sn-ov-table th{font:700 9.5px Sora;letter-spacing:.1em;text-transform:uppercase;color:#7f8ca1;
  padding:8px 10px;text-align:right;background:rgba(0,0,0,.25)}
.sn-ov-table th:first-child{text-align:left}
.sn-ov-table td{padding:7px 10px;text-align:right;font:700 13px "JetBrains Mono",monospace;color:#eaf2ff;
  border-top:1px solid var(--line)}
.sn-ov-table td:first-child{text-align:left;font:700 12.5px Sora;color:#dbe4f0}
.sn-ov-table tr.winner-row td{background:rgba(255,209,102,.08)}
.sn-ov-table .tdot{display:inline-block;width:9px;height:9px;border-radius:50%;margin-right:7px;vertical-align:1px}
@media(prefers-reduced-motion:reduce){.sn-ov-winner.pop{animation:none}}

/* confetti canvas sits INSIDE the overlay so it never blocks clicks */
#snConfetti{position:absolute;inset:0;pointer-events:none;z-index:1}
.sn-ov-inner{position:relative;z-index:2}
.sn-ov-row{display:flex;gap:10px;justify-content:center;flex-wrap:wrap}
.sn-ov-btn{border:1px solid var(--line2);border-bottom-width:3px;border-radius:10px;padding:11px 22px;
  background:var(--panel2);color:var(--ink);font:700 13px Sora;cursor:pointer;transition:transform .12s,border-color .12s}
.sn-ov-btn:hover{transform:translateY(-1px);border-color:var(--acc-dim)}
.sn-ov-btn.primary{background:var(--acc);border-color:var(--acc-dim);color:#06231a}

/* ---- side rail: dice + players + log ---- */
.sn-side{display:flex;flex-direction:column;gap:12px;min-width:0}
.sn-card{border:1px solid var(--line2);border-radius:14px;background:
  linear-gradient(180deg,rgba(255,255,255,.02),rgba(255,255,255,0)),
  rgba(13,18,30,.55);padding:14px 16px;box-shadow:0 8px 24px rgba(0,0,0,.25)}
.sn-card h4{margin:0 0 11px;font:700 9.5px Sora;letter-spacing:.16em;text-transform:uppercase;color:#7f8ca1;
  display:flex;align-items:center;gap:8px}
.sn-card h4::after{content:'';flex:1;height:1px;background:var(--line2)}

/* ---- dice panel ---- */
.sn-dice-row{display:flex;align-items:center;gap:18px}
.sn-die{width:74px;height:74px;border-radius:16px;background:
    linear-gradient(145deg,#ffffff 0%,#eef2fa 45%,#d7dfef 100%);
  display:grid;grid-template-columns:repeat(3,1fr);grid-template-rows:repeat(3,1fr);
  padding:12px;gap:2px;position:relative;flex:none;
  box-shadow:0 10px 22px rgba(0,0,0,.5),inset 0 -4px 8px rgba(96,112,148,.28),inset 0 3px 6px rgba(255,255,255,.9)}
.sn-die::after{content:'';position:absolute;inset:0;border-radius:16px;
  background:linear-gradient(115deg,rgba(255,255,255,.85) 0%,transparent 34%);pointer-events:none}
.sn-die i{border-radius:50%;background:radial-gradient(circle at 35% 30%,#2c3444 0%,#10141d 60%);place-self:center;width:10px;height:10px;visibility:hidden;
  box-shadow:inset 0 -1px 2px rgba(255,255,255,.25),0 1px 1px rgba(255,255,255,.35)}
.sn-die.rolling{animation:snshake .42s ease-in-out infinite}
@keyframes snshake{0%,100%{transform:rotate(0) translateY(0)}25%{transform:rotate(-16deg) translateY(-4px)}
  75%{transform:rotate(16deg) translateY(-4px)}}
/* landing pop when a value arrives */
.sn-die.landed{animation:snland .34s cubic-bezier(.2,.9,.3,1.4)}
@keyframes snland{0%{transform:scale(.82)}62%{transform:scale(1.09)}100%{transform:scale(1)}}
@media(prefers-reduced-motion:reduce){.sn-die.rolling{animation:none}.sn-die.landed{animation:none}}
.sn-dice-info{min-width:0;flex:1}
.sn-dice-val{font:800 30px "JetBrains Mono",monospace;color:#eaf2ff;line-height:1;
  text-shadow:0 0 18px rgba(94,242,192,.25)}
.sn-dice-lbl{font:600 9.5px Sora;letter-spacing:.14em;text-transform:uppercase;color:#7f8ca1;margin-top:5px}
.sn-roll-btn{display:inline-flex;align-items:center;justify-content:center;gap:10px;width:100%;margin-top:14px;
  border:1px solid var(--acc-dim);border-bottom-width:3px;border-radius:12px;padding:13px 18px;
  background:linear-gradient(180deg,rgba(94,242,192,.22),rgba(94,242,192,.10));
  color:var(--acc);font:800 14.5px Sora;cursor:pointer;transition:transform .12s,box-shadow .12s,filter .12s;
  box-shadow:0 6px 18px rgba(94,242,192,.14)}
.sn-roll-btn:hover:not(:disabled){transform:translateY(-1px);filter:brightness(1.12);
  box-shadow:0 8px 22px rgba(94,242,192,.22)}
.sn-roll-btn:active:not(:disabled){transform:translateY(0)}
.sn-roll-btn:disabled{opacity:.38;cursor:not-allowed;box-shadow:none}
.sn-roll-btn .hint{font:600 9.5px Sora;color:#7f8ca1;letter-spacing:.06em;
  border:1px solid var(--line2);border-radius:5px;padding:1px 5px}

/* player chips */
.sn-players{display:flex;flex-direction:column;gap:7px}
.sn-pl{display:flex;align-items:center;gap:10px;border:1px solid var(--line2);border-radius:11px;
  padding:9px 11px;background:rgba(0,0,0,.22);transition:border-color .18s,background .18s,box-shadow .18s}
.sn-pl .dot{width:15px;height:15px;border-radius:50%;flex:none;border:2px solid rgba(255,255,255,.55);
  box-shadow:0 0 10px currentColor;background:
    radial-gradient(circle at 35% 30%,rgba(255,255,255,.65),transparent 55%),currentColor}
.sn-pl .nm{font:700 12.5px Sora;color:#dbe4f0;flex:1;min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;
  display:flex;align-items:center;gap:6px}
.sn-pl .nm .tag{font:600 8.5px Sora;letter-spacing:.1em;color:#7f8ca1;margin-left:2px;
  border:1px solid var(--line2);border-radius:4px;padding:1px 4px}
.sn-pl .pos{font:800 14px "JetBrains Mono",monospace;color:#eaf2ff;min-width:3ch;text-align:right}
/* mini per-player stats (rolls·snakes·ladders) + square number */
.sn-pl .stats{font:600 10px "JetBrains Mono",monospace;color:#7f8ca1;min-width:7ch;text-align:right;letter-spacing:.02em}
.sn-pl .sq{font:800 14px "JetBrains Mono",monospace;color:#eaf2ff;min-width:3ch;text-align:right}
.sn-pl.turn{border-color:rgba(94,242,192,.45);background:rgba(94,242,192,.09);
  box-shadow:0 0 0 1px rgba(94,242,192,.18),0 4px 14px rgba(94,242,192,.08)}
.sn-pl.turn .nm{color:#eafff6}
.sn-pl.winner{border-color:#ffd166;background:rgba(255,209,102,.1);
  box-shadow:0 0 0 1px rgba(255,209,102,.25)}

/* event log: newest first (code prepends); timeline rail on each entry */
.sn-log{display:flex;flex-direction:column;gap:7px;max-height:200px;overflow-y:auto;
  font:500 11.5px Sora;color:#9fb0c7;padding-right:4px;scrollbar-width:thin}
.sn-log div{display:flex;gap:8px;align-items:baseline;line-height:1.5;flex-wrap:wrap;
  padding-left:9px;border-left:2px solid var(--line2)}
.sn-log div:first-child{border-left-color:var(--acc)}
.sn-log .who{font-weight:700;flex:none}
.sn-log .arrow{color:#5ef2c0;flex:none}
.sn-log .bad{color:#ff8fa3}
.sn-log .good{color:#5ef2c0}
.sn-log .win{color:#ffd166;font-weight:700}

/* controls + help */
.sn-ctl-row{align-items:center}
.sn-controls{display:flex;gap:8px;flex-wrap:wrap;padding:0}

/* ---- audio settings popover (same surface language as .lu-audio-pop) --
   Anchored to the top-right of the controls row; mobile keeps it inside
   the panel via right-anchoring. #snPanel scopes the positioning context
   so the popover attaches to THIS game only.                              */
#snPanel{position:relative}
.sn-audio-pop{position:absolute;top:64px;right:18px;z-index:6;width:290px;
  padding:12px 14px;display:flex;flex-direction:column;gap:9px;
  background:rgba(10,16,28,.96);border:1px solid var(--line2);border-radius:12px;
  box-shadow:0 12px 34px rgba(0,0,0,.45);backdrop-filter:blur(4px)}
.sn-audio-pop.hidden{display:none}
.sn-au-row{display:flex;align-items:center;gap:10px}
.sn-au-head{justify-content:space-between;margin-bottom:2px}
.sn-au-title{font:700 11px Sora;letter-spacing:.14em;color:#aab6c8}
.sn-au-x{border:none;background:transparent;color:#7f8ca1;cursor:pointer;
  font-size:13px;line-height:1;padding:4px 6px;border-radius:6px}
.sn-au-x:hover{color:var(--ink);background:rgba(255,255,255,.06)}
.sn-au-l{flex:0 0 104px;font:600 11.5px Sora;color:#9fb0c7}
.sn-au-slider{flex:1;min-width:0;accent-color:var(--acc);height:18px}
.sn-au-val{flex:0 0 38px;text-align:right;font:700 11px "JetBrains Mono",monospace;color:#e5edf9}
.sn-au-toggles{gap:8px;margin-top:2px}
.sn-au-toggle{flex:1;display:inline-flex;align-items:center;justify-content:center;gap:7px;
  border:1px solid var(--line2);border-radius:9px;padding:7px 10px;background:transparent;
  color:#9fb0c7;font:700 11.5px Sora;cursor:pointer;
  transition:color .12s,border-color .12s,background .12s}
.sn-au-toggle:hover{color:var(--ink);border-color:var(--acc)}
.sn-au-dot{width:8px;height:8px;border-radius:50%;background:#4a5568;transition:background .12s}
.sn-au-toggle[aria-pressed="true"]{color:var(--acc);
  background:rgba(94,242,192,.09);border-color:rgba(94,242,192,.45)}
.sn-au-toggle[aria-pressed="true"] .sn-au-dot{background:var(--acc)}
@media (max-width:520px){
  .sn-audio-pop{top:auto;bottom:14px;right:12px;left:12px;width:auto}
}
/* in-game control buttons share one premium style (namespaced so the
   shared .ft-btn look is preserved elsewhere) */
#snPanel .ft-btn{border-radius:10px;padding:8px 14px;font:700 12px Sora;
  background:rgba(13,18,30,.65);border:1px solid var(--line2);color:#c7d2e2;
  transition:border-color .15s,color .15s,background .15s,transform .12s}
#snPanel .ft-btn:hover{border-color:var(--acc-dim);color:#eafff6;background:rgba(94,242,192,.07);transform:translateY(-1px)}
#snPanel .ft-btn.primary{background:linear-gradient(180deg,rgba(94,242,192,.9),rgba(64,208,168,.92));
  border-color:rgba(94,242,192,.55);color:#06231a;font-weight:800;
  box-shadow:0 4px 14px rgba(94,242,192,.22)}
#snPanel .ft-btn.primary:hover{filter:brightness(1.08)}
.sn-msg{padding:0;font:500 12.5px Sora;color:#9fb0c7;min-height:18px;flex:1;min-width:200px;
  text-align:right;align-self:center}
.sn-msg b{color:#e5edf9}
.sn-help{display:flex;gap:14px;flex-wrap:wrap;padding:2px 0 0;font:500 11.5px Sora;color:#7f8ca1}
.sn-help span::before{content:'·';margin-right:6px;color:#5ef2c0}

/* ---- setup: compact inline toolbar (mode + names + start) ---- */
.sn-setup{padding:0;border-bottom:none}
.sn-setup-grid{display:flex;flex-wrap:wrap;gap:8px 14px;align-items:center}
.sn-field{min-width:0;display:flex;align-items:center;gap:8px}
.sn-field>label{font:700 9.5px Sora;letter-spacing:.14em;text-transform:uppercase;color:#7f8ca1;white-space:nowrap}
.sn-seg{display:inline-flex;border:1px solid var(--line2);border-radius:10px;overflow:hidden;background:rgba(0,0,0,.28);flex-wrap:wrap}
.sn-seg button{appearance:none;border:0;border-right:1px solid var(--line);background:transparent;color:#aab6c8;
  padding:7px 12px;font:600 12px Sora;cursor:pointer;transition:color .15s,background .15s}
.sn-seg button:last-child{border-right:0}
.sn-seg button:hover{color:#fff;background:rgba(94,242,192,.08)}
.sn-seg button.on{background:var(--acc);color:#06231b}
.sn-setup-actions{display:flex;gap:10px;align-items:center;flex-wrap:wrap}
.sn-note{font:400 11.5px Sora;color:#8b98ad}

/* player-name rows: inline chips, wrap when crowded */
.sn-names{display:flex;flex-direction:row;flex-wrap:wrap;gap:6px}
.sn-name-row{display:flex;align-items:center;gap:7px}
.sn-name-row .dot{width:11px;height:11px;border-radius:50%;flex:none;border:2px solid rgba(255,255,255,.55);
  box-shadow:0 0 8px currentColor}
.sn-name-input{width:150px;max-width:44vw;background:var(--bg2,#0b0e14);border:1px solid var(--line2,#2a3347);
  color:var(--ink,#e8ecf4);border-radius:8px;padding:7px 10px;font:600 12.5px Sora;transition:border-color .15s}
.sn-name-input:focus{outline:none;border-color:var(--acc-dim,#5ef2c0)}
.sn-name-input::placeholder{color:var(--ink-faint,#5d687e)}
/* duplicate-name flag (handled gracefully downstream via "· 2" suffix) */
.sn-name-input.dup{border-color:#ff8fa3;box-shadow:0 0 0 2px rgba(255,143,163,.18)}
.sn-name-static{font:600 12.5px Sora;color:var(--ink-dim,#9aa5b8)}
.sn-name-row .ai-tag{font:600 8.5px Sora;letter-spacing:.1em;text-transform:uppercase;color:#7f8ca1}
.sn-name-row.ai{opacity:.75}

/* compact setup on narrow screens */
@media(max-width:1020px){
  .sn-setup-grid{gap:6px 10px}
  .sn-field>label{display:none}          /* labels are obvious in context */
  .sn-seg button{padding:5px 9px;font-size:11px}
  .sn-name-input{width:118px;padding:6px 8px;font-size:11.5px}
  .sn-note{display:none}
  .sn-ctl-row{flex-wrap:wrap}
  .sn-msg{text-align:left;min-width:160px}
  .sn-topbar{gap:7px}
  #snPanel .ft-btn{padding:7px 11px;font-size:11.5px}
}
/* phones: reclaim vertical space so the dice panel is reachable without
   scrolling — tighter chips, smaller controls, capped stage height, and
   the redundant panel header folds away (the game keeps its Back button) */
@media(max-width:560px){
  body.ft-playing #snPanel .ft-panel-h{display:none}
  .sn-hud{gap:5px}
  .sn-stat,#snTurnChip{padding:4px 8px;gap:5px;font-size:10px}
  .sn-stat .v{font-size:13px}
  .sn-stat .k{font-size:8.5px}
  .sn-bar-row{gap:6px}
  .sn-controls{gap:5px}
  #snPanel .ft-btn{padding:6px 9px;font-size:11px}
  .sn-seg{max-width:100%;overflow-x:auto;scrollbar-width:none;-webkit-overflow-scrolling:touch}
  .sn-seg::-webkit-scrollbar{display:none}
  .sn-seg button{padding:4px 8px;font-size:10.5px;white-space:nowrap;flex:none}
  .sn-name-input{width:92px;padding:5px 7px;font-size:11px}
  .sn-names{gap:4px}
  .sn-name-row{gap:5px}
  /* phone: mode · Start scroll as ONE compact row; the name inputs (only
     present for local multiplayer) wrap onto their own tight rows below */
  .sn-setup-grid{flex-wrap:wrap;align-items:center;gap:6px}
  .sn-field{flex:none}
  #snNamesField{flex:1 1 100%;min-width:0}
  .sn-help{display:none}                 /* hints live in the desktop rail */
  body.ft-playing .sn-stage{max-height:42vh}
  .sn-card{padding:11px 12px}
  .sn-die{width:58px;height:58px;padding:9px;border-radius:13px}
  .sn-die i{width:8px;height:8px}
  .sn-dice-val{font-size:24px}
  .sn-roll-btn{margin-top:10px;padding:11px 14px;font-size:13px}
  .sn-pl{padding:7px 9px;gap:8px}
  .sn-players{gap:5px}
}

/* ---- Fullscreen / immersive layout (same convention as lu/fn) -------
   The shared session controller locks page scroll while a game is
   playing, so the square board MUST fit the viewport: the panel gets a
   definite height, the layout flexes, and snResize() fits the board. */
body.sn-fs #snPanel,body.ft-playing.ft-immersive #snPanel{
  max-width:none;width:100vw;height:100vh;margin:0;border-radius:0;border:none;overflow:hidden;
  padding:clamp(10px,2vh,20px) clamp(12px,2.5vw,28px);box-sizing:border-box;justify-content:flex-start;
  background:var(--bg,#0a0d13)}
body.sn-fs #snPanel .ft-panel-h,body.ft-playing.ft-immersive #snPanel .ft-panel-h{display:none}
/* The mode picker is a pre-game control; in fullscreen keep it VISIBLE but
   compact — hiding it (as before) made the player-count selector unreachable
   on the card-click launch path, which always starts fullscreen. */
body.sn-fs .sn-setup,body.ft-playing.ft-immersive .sn-setup{padding:0}
body.sn-fs .sn-field>label,body.ft-playing.ft-immersive .sn-field>label{display:none}
body.sn-fs .sn-seg button,body.ft-playing.ft-immersive .sn-seg button{padding:6px 11px;font-size:12px}
body.sn-fs .sn-name-input,body.ft-playing.ft-immersive .sn-name-input{width:140px;padding:6px 9px;font-size:12px}
body.sn-fs .sn-note,body.ft-playing.ft-immersive .sn-note{display:none}
body.sn-fs #snLayout,body.ft-playing.ft-immersive #snLayout{
  flex:1;display:flex;flex-direction:row;justify-content:center;align-items:stretch;min-height:0;gap:16px}
body.sn-fs .sn-stage,body.ft-playing.ft-immersive .sn-stage{
  flex:1 1 auto;min-width:0;min-height:0;max-height:100%;display:flex;align-items:center;justify-content:center}
body.sn-fs #snBoard,body.ft-playing.ft-immersive #snBoard{height:auto;max-height:100%;width:auto;max-width:100%}
body.sn-fs .sn-side,body.ft-playing.ft-immersive .sn-side{flex:0 0 320px;max-width:320px;overflow-y:auto}
body.sn-fs .sn-help,body.ft-playing.ft-immersive .sn-help{display:none}
body.sn-fs .sn-layout,body.ft-playing.ft-immersive .sn-layout{padding:4px 0 0}
@media(max-width:1020px){
  body.sn-fs #snLayout,body.ft-playing.ft-immersive #snLayout{flex-direction:column;overflow-y:auto}
  body.sn-fs .sn-side,body.ft-playing.ft-immersive .sn-side{flex:0 0 auto;max-width:none}
  body.sn-fs .sn-stage,body.ft-playing.ft-immersive .sn-stage{flex:0 0 auto}
}

/* ---- Fun Time selector-card art (matches fn/tm/lu card conventions) ---- */
.ft-card-art.sn{
  background:
    radial-gradient(circle at 30% 20%,rgba(94,242,192,.12),transparent 55%),
    linear-gradient(180deg,rgba(110,168,255,.10),rgba(7,9,13,.6));
}
.sn-card-scene{position:relative;width:min(250px,78%);height:clamp(104px,15vw,140px);display:block}
.sn-card-grid{position:absolute;inset:0;display:grid;grid-template-columns:repeat(5,1fr);grid-template-rows:repeat(4,1fr);
  gap:4px}
.sn-card-grid i{border-radius:4px;background:rgba(21,26,37,.9);border:1px solid #263149}
.sn-card-grid i:nth-child(odd){background:rgba(94,242,192,.07)}
.sn-card-ladder{position:absolute;left:24%;top:12%;width:10px;height:62%;
  background:repeating-linear-gradient(180deg,#ffc857 0 7px,transparent 7px 14px);
  transform:rotate(9deg);border-radius:2px;box-shadow:0 0 12px rgba(255,200,87,.35)}
.sn-card-ladder::after{content:'';position:absolute;left:-3px;top:0;bottom:0;width:2px;background:#ffc85788}
.sn-card-ladder::before{content:'';position:absolute;right:-3px;top:0;bottom:0;width:2px;background:#ffc85788}
.sn-card-snake{position:absolute;right:18%;top:20%;width:44px;height:70%;
  background:repeating-radial-gradient(circle at 50% 100%,#c9304a 0 3px,#ff8fa3 3px 6px);
  clip-path:polygon(50% 0,72% 12%,60% 30%,80% 46%,62% 62%,78% 78%,58% 92%,50% 100%,34% 90%,44% 74%,26% 60%,42% 44%,24% 28%,36% 12%);
  filter:drop-shadow(0 0 8px rgba(255,107,129,.4))}
.sn-card-pawn{position:absolute;width:15px;height:15px;border-radius:50%;border:2px solid rgba(255,255,255,.6);
  box-shadow:0 0 12px currentColor;animation:sncardp 2.6s ease-in-out infinite}
.sn-card-pawn.r{background:#ff5d5d;color:#ff5d5d;left:14%;bottom:16%}
.sn-card-pawn.g{background:#5ef2c0;color:#5ef2c0;left:26%;bottom:30%;animation-delay:.4s}
@keyframes sncardp{0%,100%{transform:translate(0,0)}50%{transform:translate(4px,-8px)}}
@media (prefers-reduced-motion:reduce){.sn-card-pawn{animation:none}}
`;
  document.head.appendChild(st);
})();

/* ---- state ------------------------------------------------------------ */
const SNG = {
  epoch: 0,            // VIEW epoch: bumped on mount/teardown only.
  clock: 0,            // GAME clock: bumped by new-game/teardown. Every
                       // async flow callback captures and re-checks it.
  game: null,          // SnakesEngine game state
  phase: 'idle',       // idle | rolling | moving | over
  dice: null,          // last rolled value
  anim: null,          // {player, path:[sq…], i, raf, mv} while walking
  viz: null,           // {player, from, to, t, kind} fractional hint
  cleanup: [],
  canvas: null, ctx: null,
  w: 0, h: 0, cell: 0,
  soundOn: true,
  reducedMotion: false,
  mode: 'pvc',         // pvc | 2p | 3p | 4p | 2vc  (setup selection)
  names: [],           // display names per player index; [] = defaults
  stats: [],           // per-player match stats; rebuilt by snResetStats()
};
SNG.viewAlive = () => !!SNG.canvas;

/* ---- audio -------------------------------------------------------------
   PROFESSIONAL AUDIO SYSTEM (same architecture as ludo-game.js Phase 5.4):

     SNA.ctx            ONE AudioContext for the whole view lifecycle.
                        Created only after a real user gesture (autoplay
                        policy); every entry point funnels through
                        snAudioReady() so init can never race or double.
     SNA.master         master GainNode — every SFX + music bus feeds it.
     SNA.sfxBus         GainNode for sound effects (own volume).
     SNA.musicBus       GainNode for background music (own volume).
     SNA.lastPlay       per-sound-kind timestamp map → identical sounds
                        closer than SN_SFX_GAP ms apart are dropped, so
                        rAF-driven triggers (steps) can never stack into
                        a buzz and two events in the same frame play once.
     SNA.music          music state {step, nextT, duckUntil}.

   The "music" is a generative cheerful arcade loop (Web Audio
   oscillators scheduled ahead on a lookahead timer — no assets, no
   dependencies, seamless by construction). Ducking during victory
   stingers keeps them from competing.

   Settings persist under one localStorage key ('snakes-audio-v1') and
   are validated on load; failures fall back to defaults silently.

   Cleanup: snTeardown() closes the context exactly once and clears the
   music timer; route-away/restart can never leak a context.          */
const SNA = {
  ctx: null, master: null, sfxBus: null, musicBus: null,
  inited: false,        // init ran (success OR failure) — never retry-spam
  lastPlay: {},         // kind → last start time (dedup guard)
  music: null,          // active music state (null = stopped)
  musicTimer: 0,        // scheduler timer id (0 = none)
};

/* persisted settings shape: {m:0..1 master, mu:0|1 music on, sx:0|1 sfx
   on, mv:0..1 music vol, sv:0..1 sfx vol} — compact keys, one blob. */
const SN_AUDIO_KEY = 'snakes-audio-v1';
const SN_SFX_GAP = 70;           // ms: identical sounds closer dedupe
const SN_MUSIC_LOOKAHEAD = 0.9;  // s of notes scheduled ahead

function snAudioDefaults() {
  return { m: 0.7, mu: 1, sx: 1, mv: 0.35, sv: 0.9 };
}

/** Load + validate persisted settings (defensive: any bad value falls
 *  back to the default for that field). Returns a fresh object. */
function snAudioLoad() {
  const d = snAudioDefaults();
  try {
    const raw = localStorage.getItem(SN_AUDIO_KEY);
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
      if (o.sx === 0 || o.sx === 1) d.sx = o.sx;
    }
  } catch { /* corrupted blob → defaults */ }
  return d;
}

/** Persist current settings (best effort; private mode may refuse). */
function snAudioSave() {
  try {
    localStorage.setItem(SN_AUDIO_KEY, JSON.stringify({
      m: SNG.audio.m, mu: SNG.audio.mu, mv: SNG.audio.mv,
      sv: SNG.audio.sv, sx: SNG.audio.sx,
    }));
  } catch { /* storage unavailable — settings stay session-only */ }
}

/** Live settings object (loaded once at script parse; safe pre-DOM). */
SNG.audio = snAudioLoad();

/**
 * Ensure the audio graph exists AND is running. Returns the context or
 * null. Safe to call from every trigger — the `inited` flag means the
 * constructor runs at most once per view mount, so a failing environment
 * (no Web Audio / blocked) costs one attempt, not one per frame.
 *
 * Autoplay policy: the context starts 'suspended' until a gesture. All
 * callers run inside click/keydown handlers; for programmatic triggers
 * (AI turns) the context is already unlocked by the human's first
 * interaction with the page.
 */
function snAudioReady() {
  if (SNA.inited) {
    if (SNA.ctx && SNA.ctx.state === 'suspended')
      SNA.ctx.resume().catch(() => {});
    return SNA.ctx;
  }
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) { SNA.inited = true; return null; }      // unsupported: silent
  try {
    SNA.ctx = new AC();
    SNA.master = SNA.ctx.createGain();
    SNA.sfxBus = SNA.ctx.createGain();
    SNA.musicBus = SNA.ctx.createGain();
    SNA.master.connect(SNA.ctx.destination);
    SNA.sfxBus.connect(SNA.master);
    SNA.musicBus.connect(SNA.master);
    snAudioApplyVolumes();
    SNA.inited = true;
    if (SNA.ctx.state === 'suspended')
      SNA.ctx.resume().catch(() => {});             // gesture-gated
    return SNA.ctx;
  } catch {
    SNA.ctx = null; SNA.inited = true; return null;
  }
}

/** Push settings into the gain graph (immediate, click-free via ramp). */
function snAudioApplyVolumes() {
  if (!SNA.ctx || !SNA.master) return;
  const t = SNA.ctx.currentTime;
  const set = (node, v) => {
    if (!node) return;
    node.gain.cancelScheduledValues(t);
    node.gain.setTargetAtTime(Math.max(0.0001, v), t, 0.02);
  };
  set(SNA.master, SNG.audio.m);
  set(SNA.sfxBus, SNG.audio.sx ? SNG.audio.sv : 0);
  set(SNA.musicBus, SNG.audio.mu ? SNG.audio.mv : 0);
}

/** One synthesized tone on the SFX bus. Volume is relative (0..1) — the
 *  bus/master gains do the scaling, so global changes apply mid-sound. */
function snTone(freq, dur, vol, type, slideTo, when) {
  const ctx = snAudioReady();
  /* Autoplay-safe: never schedule into a suspended/frozen clock — notes
     booked there would all fire at once on the first resume. */
  if (!ctx || ctx.state !== 'running' || !SNG.soundOn || !freq) return;
  if (!SNG.audio.sx) return;                        // SFX muted
  const t = (when != null && when > ctx.currentTime) ? when : ctx.currentTime;
  const osc = ctx.createOscillator(), gain = ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(Math.max(20, slideTo), t + dur);
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(Math.max(0.001, vol), t + 0.008);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(SNA.sfxBus);
  osc.start(t); osc.stop(t + dur + 0.02);
}

/** Soft percussive noise burst (dice clack) — generated buffer, created
 *  ONCE and reused (never per-frame allocation). */
let SN_NOISE_BUF = null;
function snNoiseBuf(ctx) {
  if (SN_NOISE_BUF) return SN_NOISE_BUF;
  const n = Math.floor(ctx.sampleRate * 0.25);
  const buf = ctx.createBuffer(1, n, ctx.sampleRate);
  const d = buf.getChannelData(0);
  for (let i = 0; i < n; i++) d[i] = Math.random() * 2 - 1;
  SN_NOISE_BUF = buf;
  return buf;
}

function snNoise(vol, dur, freq, when) {
  const ctx = snAudioReady();
  if (!ctx || ctx.state !== 'running' || !SNG.soundOn || !SNG.audio.sx) return;
  const t = (when != null && when > ctx.currentTime) ? when : ctx.currentTime;
  const src = ctx.createBufferSource();
  src.buffer = snNoiseBuf(ctx);
  const f = ctx.createBiquadFilter();
  f.type = 'bandpass'; f.frequency.value = freq || 900; f.Q.value = 1.1;
  const g = ctx.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(Math.max(0.001, vol), t + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(f); f.connect(g); g.connect(SNA.sfxBus);
  src.start(t); src.stop(t + dur + 0.02);
}

/**
 * Play a named sound effect. Deduplicates identical kinds inside
 * SN_SFX_GAP ms (rapid steps / double-triggered events collapse to one)
 * and composes each cue from a few scheduled tones/noise hits so every
 * game event has its own recognisable voice. Short and quiet by design.
 */
function snSfx(kind) {
  if (!SNG.soundOn || !SNG.audio.sx) return;
  const now = performance.now();
  if (SNA.lastPlay[kind] && now - SNA.lastPlay[kind] < SN_SFX_GAP) return;
  SNA.lastPlay[kind] = now;
  /* Autoplay policy: programmatic triggers (AI turns, timers) may only
     USE an already-running context; they must never create one. */
  if (!SNA.ctx || SNA.ctx.state !== 'running') return;
  const ctx = SNA.ctx;
  const T = ctx.currentTime;

  switch (kind) {
    /* dice tumble: wooden clacks rising in pitch + a settle tick */
    case 'roll':
      [0, 90, 185].forEach((ms, i) =>
        snNoise(0.10, 0.05, 700 + i * 260, T + ms / 1000));
      snTone(320, 0.03, 0.05, 'triangle', 240, T + 0.24);
      break;
    /* dice result landed: bright two-note reveal */
    case 'dice':
      snTone(587, 0.07, 0.09, 'triangle');
      snTone(784, 0.09, 0.09, 'triangle', null, T + 0.07);
      break;
    /* one square hop: soft woody tick (quiet — fires every hop) */
    case 'step':
      snTone(292, 0.035, 0.05, 'sine', 250);
      break;
    /* token entering the board from start */
    case 'enter':
      snTone(392, 0.08, 0.10, 'triangle');
      snTone(523, 0.09, 0.10, 'triangle', null, T + 0.08);
      break;
    /* ladder climb: bright ascending arpeggio (special rise) */
    case 'ladder':
      [392, 494, 587, 698, 880].forEach((f, i) =>
        snTone(f, 0.10, 0.11, 'triangle', null, T + i * 0.085));
      break;
    /* snake slide: dramatic descending slide + low hiss thud */
    case 'snake':
      snTone(520, 0.55, 0.15, 'sawtooth', 80);
      snNoise(0.10, 0.30, 300, T + 0.10);
      snTone(110, 0.18, 0.12, 'square', 60, T + 0.42);
      break;
    /* overshoot / stuck: muted negative blip */
    case 'stuck':
      snTone(220, 0.14, 0.10, 'square', 150);
      break;
    /* game start: upbeat launch fanfare */
    case 'start':
      snMusicDuck(1.6);
      [523, 659, 784].forEach((f, i) =>
        snTone(f, 0.11, 0.11, 'triangle', null, T + i * 0.09));
      snTone(1047, 0.16, 0.10, 'triangle', null, T + 0.30);
      break;
    /* victory celebration: major arpeggio + high bell */
    case 'win':
      snMusicDuck(2.4);
      [523, 659, 784, 1047].forEach((f, i) =>
        snTone(f, 0.12, 0.12, 'triangle', null, T + i * 0.10));
      snTone(1319, 0.32, 0.10, 'sine', null, T + 0.44);
      break;
    /* generic UI button press */
    case 'ui':
      snTone(880, 0.03, 0.05, 'sine', 780);
      break;
  }
}

/* ---- background music (generative cheerful arcade loop) -------------------
   A bouncy two-chord verse/chorus progression with a walking bass and a
   sparkle melody, scheduled on a lookahead timer (the standard Web Audio
   scheduling pattern — the timer only BOOKS notes ahead; nothing runs
   per animation frame). Seamless by construction: the progression loops
   forever with no seam to crossfade. Volume lives on SNA.musicBus;
   toggling mutes instantly via the graph.                                  */

const SN_MUSIC_CHORDS = [
  [261.63, 329.63, 392.00],   // C major
  [261.63, 329.63, 392.00],   // C
  [220.00, 277.18, 329.63],   // A minor-ish
  [196.00, 246.94, 293.66],   // G
];
const SN_MUSIC_BASS = [130.81, 130.81, 110.00, 98.00];   // C C A G
const SN_MUSIC_BELLS = [783.99, 659.25, 587.33, 659.25];

/** Start the music scheduler if (and only if) settings allow. Idempotent:
 *  a second call while running is a no-op, so restart churn can never
 *  stack timers. */
function snMusicStart() {
  if (SNA.musicTimer) return;                       // already playing
  if (!SNG.audio.mu) return;                        // music muted
  /* Autoplay policy: music must never CREATE the context — only a real
     user gesture may. If the context is missing or still suspended,
     skip; the next gesture-owned start picks music up. */
  if (!SNA.ctx || SNA.ctx.state !== 'running') return;
  const ctx = SNA.ctx;
  SNA.music = { step: 0, nextT: ctx.currentTime + 0.1, duckUntil: 0 };
  const tick = () => {
    SNA.musicTimer = 0;                             // own the slot again
    if (!SNA.music || !SNG.audio.mu) { SNA.music = null; return; }
    if (!SNA.ctx || SNA.ctx.state !== 'running') { SNA.music = null; return; }
    const m = SNA.music, ct = ctx.currentTime;
    while (m.nextT < ct + SN_MUSIC_LOOKAHEAD) {
      const ci = m.step % SN_MUSIC_CHORDS.length;
      const chord = SN_MUSIC_CHORDS[ci];
      const barDur = 1.9;                           // seconds per chord
      /* pad: three detuned sines per chord, slow attack/release */
      chord.forEach((f, k) => {
        snPadNote(f, m.nextT, barDur * 1.05);
        if (k === 0) snPadNote(f * 0.5, m.nextT, barDur * 1.05); // root down
      });
      /* walking bass on the beat */
      snBassNote(SN_MUSIC_BASS[ci], m.nextT);
      snBassNote(SN_MUSIC_BASS[ci], m.nextT + barDur * 0.5);
      /* sparkle bells on beats 1 and 3 */
      snBell(SN_MUSIC_BELLS[m.step % SN_MUSIC_BELLS.length], m.nextT + 0.35);
      snBell(SN_MUSIC_BELLS[(m.step + 2) % SN_MUSIC_BELLS.length], m.nextT + 1.15);
      m.step++;
      m.nextT += barDur;
    }
    SNA.musicTimer = setTimeout(tick, 350);         // re-arm scheduler
  };
  SNA.musicTimer = setTimeout(tick, 50);
}

/** Stop the music and drop every pending booking. */
function snMusicStop() {
  if (SNA.musicTimer) { clearTimeout(SNA.musicTimer); SNA.musicTimer = 0; }
  SNA.music = null;
}

/** Temporarily lower music while a stinger plays (win/start). */
function snMusicDuck(sec) {
  if (!SNA.musicBus || !SNA.ctx) return;
  const t = SNA.ctx.currentTime;
  SNA.musicBus.gain.cancelScheduledValues(t);
  SNA.musicBus.gain.setTargetAtTime(SNG.audio.mu ? SNG.audio.mv * 0.18 : 0, t, 0.05);
  SNA.musicBus.gain.setTargetAtTime(SNG.audio.mu ? SNG.audio.mv : 0, t + sec, 0.4);
}

/** One sustained pad note on the music bus (long attack = unobtrusive). */
function snPadNote(freq, when, dur) {
  const ctx = SNA.ctx; if (!ctx || !SNA.musicBus) return;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = 'sine';
  osc.frequency.value = freq * (1 + (Math.random() - 0.5) * 0.004); // chorus detune
  g.gain.setValueAtTime(0.0001, when);
  g.gain.linearRampToValueAtTime(0.042, when + dur * 0.35);
  g.gain.linearRampToValueAtTime(0.0001, when + dur);
  osc.connect(g); g.connect(SNA.musicBus);
  osc.start(when); osc.stop(when + dur + 0.05);
}

/** One short plucked bass note on the music bus. */
function snBassNote(freq, when) {
  const ctx = SNA.ctx; if (!ctx || !SNA.musicBus) return;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = 'triangle';
  osc.frequency.value = freq;
  g.gain.setValueAtTime(0.0001, when);
  g.gain.exponentialRampToValueAtTime(0.055, when + 0.02);
  g.gain.exponentialRampToValueAtTime(0.0001, when + 0.45);
  osc.connect(g); g.connect(SNA.musicBus);
  osc.start(when); osc.stop(when + 0.55);
}

/** One soft bell pluck on the music bus. */
function snBell(freq, when) {
  const ctx = SNA.ctx; if (!ctx || !SNA.musicBus) return;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = 'triangle';
  osc.frequency.value = freq;
  g.gain.setValueAtTime(0.0001, when);
  g.gain.exponentialRampToValueAtTime(0.048, when + 0.02);
  g.gain.exponentialRampToValueAtTime(0.0001, when + 1.0);
  osc.connect(g); g.connect(SNA.musicBus);
  osc.start(when); osc.stop(when + 1.1);
}

/** Music follows game life: start on game activity, stop when over or
 *  the view goes away. Called from the same places that already manage
 *  the turn flow, so it inherits their guards. */
function snMusicFollow() {
  const active = SNG.game && SNG.game.winner === null &&
    ['idle', 'rolling', 'moving'].includes(SNG.phase);
  if (active && SNG.viewAlive()) snMusicStart();
  else snMusicStop();
}

/** Full audio teardown: stop music, close the context ONCE. */
function snAudioTeardown() {
  snMusicStop();
  if (SNA.ctx) {
    try { SNA.ctx.close(); } catch { /* already closed */ }
  }
  SNA.ctx = null; SNA.master = null; SNA.sfxBus = null; SNA.musicBus = null;
  SNA.inited = false; SNA.lastPlay = {};
  SN_NOISE_BUF = null;                              // buffer belonged to ctx
}

const sn$ = id => document.getElementById(id);
function snOn(target, ev, fn, opts) {
  target.addEventListener(ev, fn, opts);
  SNG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
}

/* ---- palette ----------------------------------------------------------- */
const SN_COLORS = {
  red:    { fill: '#ff5d5d', dark: '#c93030' },
  green:  { fill: '#5ef2c0', dark: '#2ea886' },
  yellow: { fill: '#ffd166', dark: '#c99a2e' },
  blue:   { fill: '#6ea8ff', dark: '#3671c9' },
};
const SN_P = ['red', 'green', 'yellow', 'blue'];

/* ---- turn-flow timings (ms). Tests shrink via SND.fast(). ------------- */
const SN_T = {
  rollSpin: 520,      // dice tumble before the value lands
  aiThink: 700,       // pause before an AI rolls
  stepMs: 130,        // one square of the walk animation
  jumpMs: 900,        // snake slide / ladder climb duration
  settleMs: 320,      // pause after a plain move
  eventMs: 700,       // longer pause when a snake/ladder must be seen
};

/* ---- canvas sizing ------------------------------------------------------ */
function snResize() {
  const c = SNG.canvas; if (!c) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  /* Square board: fit BOTH the stage's width and its height (the playing
     session locks page scroll, so a width-only fit would clip). Measured
     from the STAGE, never the canvas itself — the canvas's own width is
     whatever the last fit produced, which would freeze the size forever.
     A rAF-deferred second pass catches late flex/grid settles (e.g. right
     after a viewport change while the topbar is still re-wrapping). */
  const fit = () => {
    if (!SNG.canvas) return;
    const wrap = c.parentElement;                        // .sn-stage
    const availW = Math.max(280, (wrap && wrap.clientWidth) || 620);
    const availH = Math.max(280, (wrap && wrap.clientHeight) || availW);
    const cssW = Math.floor(Math.min(availW, availH));
    const cssH = cssW;
    if (cssW === SNG._lastCssW && c.style.width === cssW + 'px') return;   // no-op guard
    SNG._lastCssW = cssW;
    c.style.width = cssW + 'px';
    c.style.height = cssH + 'px';
    c.width = Math.round(cssW * dpr);
    c.height = Math.round(cssH * dpr);
    SNG.w = c.width; SNG.h = c.height;
    SNG.cell = SNG.w / 10;
    snDraw();
  };
  fit();
  requestAnimationFrame(() => { if (SNG.viewAlive()) fit(); });
}

/* ---- geometry helpers ---------------------------------------------------- */
/** Centre of square n (1..100) in device pixels; null when off board. */
function snCenter(n) {
  const E = window.SnakesEngine;
  if (n < 1 || n > 100) return null;
  const rc = E.CELL[n];
  /* row 0 is the BOTTOM row: flip for canvas y */
  return { x: (rc[1] + 0.5) * SNG.cell, y: ((9 - rc[0]) + 0.5) * SNG.cell };
}

/* ---- drawing -------------------------------------------------------------- */
function snRoundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/** Draw the whole board: plate, squares, ladders, snakes, tokens. */
function snDraw() {
  const ctx = SNG.ctx; if (!ctx) return;
  const E = window.SnakesEngine;
  const W = SNG.w, C = SNG.cell;
  ctx.clearRect(0, 0, W, W);

  /* ---- premium plate: deep vignette + felt texture ---- */
  const plate = ctx.createLinearGradient(0, 0, W, W);
  plate.addColorStop(0, '#101a2c');
  plate.addColorStop(0.5, '#0c1424');
  plate.addColorStop(1, '#0a1120');
  ctx.fillStyle = plate;
  ctx.fillRect(0, 0, W, W);
  /* soft inner light so the centre reads as the play surface */
  const glow = ctx.createRadialGradient(W / 2, W * 0.42, W * 0.05, W / 2, W / 2, W * 0.75);
  glow.addColorStop(0, 'rgba(110,168,255,.07)');
  glow.addColorStop(1, 'rgba(110,168,255,0)');
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, W, W);

  /* --- numbered squares (zigzag order is implicit in CELL) --- */
  for (let n = 1; n <= 100; n++) {
    const pt = snCenter(n);
    const x = pt.x - C / 2, y = pt.y - C / 2;
    const even = (E.CELL[n][0] + E.CELL[n][1]) % 2 === 0;
    const r = Math.max(2.5, C * 0.12);
    let c0, c1, edge;

    if (n === 100) {            // FINISH: emerald jewel cell
      c0 = '#37d9a4'; c1 = '#178a63'; edge = 'rgba(234,255,246,.55)';
    } else if (n === 1) {       // START: slate welcome cell
      c0 = '#2c3a58'; c1 = '#1c2740'; edge = 'rgba(159,176,199,.5)';
    } else if (even) {
      c0 = '#1a2439'; c1 = '#141c2e'; edge = 'rgba(255,255,255,.045)';
    } else {
      c0 = '#151e31'; c1 = '#101728'; edge = 'rgba(255,255,255,.03)';
    }

    /* cell body: diagonal light → dark, like a lacquered tile */
    const g = ctx.createLinearGradient(x, y, x + C, y + C);
    g.addColorStop(0, c0); g.addColorStop(1, c1);
    ctx.fillStyle = g;
    snRoundRect(ctx, x + 1.5, y + 1.5, C - 3, C - 3, r);
    ctx.fill();

    /* top-edge sheen (skipped on tiny cells where it would look noisy) */
    if (C > 34 && n !== 100) {
      const sheen = ctx.createLinearGradient(0, y + 2, 0, y + C * 0.45);
      sheen.addColorStop(0, 'rgba(255,255,255,.055)');
      sheen.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = sheen;
      snRoundRect(ctx, x + 2.5, y + 2.5, C - 5, C * 0.42, r * 0.8);
      ctx.fill();
    }

    /* hairline border */
    ctx.lineWidth = 1;
    ctx.strokeStyle = edge;
    snRoundRect(ctx, x + 1.5, y + 1.5, C - 3, C - 3, r);
    ctx.stroke();

    /* subtle tint on odd (reversed) rows so the zigzag reads clearly */
    if (E.CELL[n][0] % 2 === 1 && n !== 1 && n !== 100) {
      ctx.fillStyle = 'rgba(110,168,255,.035)';
      snRoundRect(ctx, x + 1.5, y + 1.5, C - 3, C - 3, r);
      ctx.fill();
    }

    /* square number is painted in the overlay pass AFTER snakes/ladders
       so artwork can never obscure it. Only START/FINISH label here. */
    const fs = Math.max(8, Math.min(C * 0.235, 14));
    if (n === 1 || n === 100) {
      ctx.font = `800 ${Math.max(6.5, fs * 0.68)}px Sora, sans-serif`;
      ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
      ctx.fillStyle = n === 100 ? '#06231a' : '#aab6c8';
      ctx.fillText(n === 1 ? 'START' : 'FINISH', pt.x, y + C * 0.71);
    }
  }

  /* --- ladders (behind tokens): two rails + rungs, warm wood --- */  for (const [b, t] of E.LADDERS) {
    const A = snCenter(b), B = snCenter(t);
    const dx = B.x - A.x, dy = B.y - A.y;
    const len = Math.hypot(dx, dy);
    const ux = dx / len, uy = dy / len;          // unit along ladder
    const px = -uy, py = ux;                     // unit perpendicular
    const half = C * 0.17;                       // rail separation (half-width)
    const railW = Math.max(1.8, C * 0.052);      // rail thickness
    const rungW = Math.max(1.3, C * 0.036);      // rung thickness
    ctx.save();
    ctx.lineCap = 'round';

    /* soft drop shadow under the whole ladder */
    ctx.strokeStyle = 'rgba(0,0,0,.38)';
    ctx.lineWidth = railW * 2.1;
    ctx.beginPath();
    ctx.moveTo(A.x + px * half + C * 0.02, A.y + py * half + C * 0.03);
    ctx.lineTo(B.x + px * half + C * 0.02, B.y + py * half + C * 0.03);
    ctx.moveTo(A.x - px * half + C * 0.02, A.y - py * half + C * 0.03);
    ctx.lineTo(B.x - px * half + C * 0.02, B.y - py * half + C * 0.03);
    ctx.stroke();

    /* rails: warm oak gradient along the ladder axis */
    const wood = ctx.createLinearGradient(A.x, A.y, B.x, B.y);
    wood.addColorStop(0, '#f0b95c');
    wood.addColorStop(0.5, '#d99b3d');
    wood.addColorStop(1, '#b97f2c');
    ctx.strokeStyle = wood;
    ctx.lineWidth = railW;
    for (const s of [-1, 1]) {
      ctx.beginPath();
      ctx.moveTo(A.x + px * half * s, A.y + py * half * s);
      ctx.lineTo(B.x + px * half * s, B.y + py * half * s);
      ctx.stroke();
    }
    /* rungs: lighter, evenly spaced, inset from the rail ends */
    ctx.strokeStyle = 'rgba(240,197,120,.92)';
    ctx.lineWidth = rungW;
    const rungGap = C * 0.46;
    for (let d = rungGap * 0.55; d < len - rungGap * 0.35; d += rungGap) {
      ctx.beginPath();
      ctx.moveTo(A.x + ux * d + px * half, A.y + uy * d + py * half);
      ctx.lineTo(A.x + ux * d - px * half, A.y + uy * d - py * half);
      ctx.stroke();
    }
    ctx.restore();
  }

  /* --- snakes (behind tokens) --- */
  for (const [head, tail] of E.SNAKES) snDrawSnake(head, tail);

  /* --- number overlay pass: repaint the numbers ABOVE the snakes and
         ladders so every square stays identifiable at a glance. A soft
         rounded backing keeps them legible over any artwork. --- */
  {
    const fs = Math.max(8, Math.min(C * 0.235, 14));
    for (let n = 1; n <= 100; n++) {
      const pt = snCenter(n);
      const x = pt.x - C / 2, y = pt.y - C / 2;
      const special = n === 1 || n === 100;
      if (!special) {
        /* translucent chip behind the numeral */
        const bw = C * 0.34, bh = C * 0.26;
        ctx.fillStyle = 'rgba(9,13,22,.62)';
        snRoundRect(ctx, x + C * 0.075, y + C * 0.055, bw, bh, Math.max(2, bh * 0.35));
        ctx.fill();
      }
      ctx.font = `700 ${fs}px "JetBrains Mono", monospace`;
      ctx.textAlign = 'left'; ctx.textBaseline = 'top';
      /* FINISH keeps a dark numeral only where the emerald is bright;
         use light text everywhere else (incl. 100) for contrast. */
      ctx.fillStyle = n === 1 ? 'rgba(214,225,240,.95)' : 'rgba(234,255,246,.95)';
      ctx.fillText(String(n), x + C * 0.11, y + C * 0.085);
    }
  }

  /* --- tokens --- */
  snDrawTokens();
}

/**
 * One snake, drawn as a filled outline (not stroked segments): sample a
 * cubic bezier spine, then build left/right edge points at a smoothly
 * tapering half-width and fill the resulting ribbon. This gives constant,
 * controlled thickness (no lumpy overlaps between stroked segments) plus
 * a proper highlight and belly stripe, at any board size.
 */
function snDrawSnake(head, tail) {
  const ctx = SNG.ctx, C = SNG.cell;
  const H = snCenter(head), T = snCenter(tail);
  const dx = T.x - H.x, dy = T.y - H.y;
  const len = Math.hypot(dx, dy) || 1;
  const ux = dx / len, uy = dy / len;             // head -> tail
  const px = -uy, py = ux;                        // perpendicular
  const bow = Math.min(len * 0.20, C * 1.25);     // lateral bend amount

  /* control points offset perpendicular for the S-curve */
  const m1x = H.x + dx * 0.32 + px * bow, m1y = H.y + dy * 0.32 + py * bow;
  const m2x = H.x + dx * 0.68 - px * bow, m2y = H.y + dy * 0.68 - py * bow;

  /* sample the spine */
  const STEPS = 40;
  const pts = [];
  for (let i = 0; i <= STEPS; i++) {
    const t = i / STEPS;
    const it = 1 - t;
    const x = it * it * it * H.x + 3 * it * it * t * m1x + 3 * it * t * t * m2x + t * t * t * T.x;
    const y = it * it * it * H.y + 3 * it * it * t * m1y + 3 * it * t * t * m2y + t * t * t * T.y;
    pts.push({ x, y, t });
  }
  /* tangent at each sample (for the ribbon edges) */
  const tang = pts.map((p, i) => {
    const a = pts[Math.max(0, i - 1)], b = pts[Math.min(STEPS, i + 1)];
    const l = Math.hypot(b.x - a.x, b.y - a.y) || 1;
    return { x: (b.x - a.x) / l, y: (b.y - a.y) / l };
  });

  /* half-width profile: slim neck → full mid-body → thin tail tip.
     Deliberately modest (max ~C*0.13) so snakes never swamp the board. */
  const halfW = t => {
    const body = C * 0.125;
    const neck = 0.62 + 0.38 * Math.min(1, t / 0.16);           // ease out of the head
    const taper = t > 0.72 ? Math.max(0.12, 1 - (t - 0.72) / 0.28) : 1;   // tail point
    return body * neck * taper;
  };

  /* ribbon outline: head side → tail side, reversed */
  const left = [], right = [];
  for (let i = 0; i <= STEPS; i++) {
    const p = pts[i], tn = tang[i];
    const w = halfW(p.t);
    const ox = -tn.y * w, oy = tn.x * w;
    left.push({ x: p.x + ox, y: p.y + oy });
    right.push({ x: p.x - ox, y: p.y - oy });
  }

  ctx.save();

  /* soft ground shadow (slightly offset, blurred by low alpha layers) */
  ctx.beginPath();
  ctx.moveTo(left[0].x + C * 0.03, left[0].y + C * 0.045);
  for (const p of left) ctx.lineTo(p.x + C * 0.03, p.y + C * 0.045);
  for (let i = right.length - 1; i >= 0; i--) ctx.lineTo(right[i].x + C * 0.03, right[i].y + C * 0.045);
  ctx.closePath();
  ctx.fillStyle = 'rgba(0,0,0,.30)';
  ctx.fill();

  /* body fill: crimson → deep rose along the spine */
  const grad = ctx.createLinearGradient(H.x, H.y, T.x, T.y);
  grad.addColorStop(0, '#e8506b');
  grad.addColorStop(0.5, '#c93a52');
  grad.addColorStop(1, '#8f2740');
  ctx.beginPath();
  ctx.moveTo(left[0].x, left[0].y);
  for (const p of left) ctx.lineTo(p.x, p.y);
  for (let i = right.length - 1; i >= 0; i--) ctx.lineTo(right[i].x, right[i].y);
  ctx.closePath();
  ctx.fillStyle = grad;
  ctx.fill();

  /* top highlight: a thinner bright ribbon along the upper edge */
  ctx.beginPath();
  for (let i = 0; i <= STEPS; i++) {
    const p = pts[i], tn = tang[i];
    const w = halfW(p.t) * 0.42;
    const hx = p.x - tn.y * w * 0.55, hy = p.y + tn.x * w * 0.55;
    if (i === 0) ctx.moveTo(hx, hy); else ctx.lineTo(hx, hy);
  }
  ctx.strokeStyle = 'rgba(255,175,190,.34)';
  ctx.lineWidth = Math.max(1, C * 0.02);
  ctx.lineCap = 'round';
  ctx.stroke();

  /* scale pattern: soft diamond dashes down the spine centre */
  ctx.setLineDash([C * 0.11, C * 0.155]);
  ctx.strokeStyle = 'rgba(70,10,25,.35)';
  ctx.lineWidth = Math.max(1, C * 0.016);
  ctx.beginPath();
  ctx.moveTo(pts[0].x, pts[0].y);
  for (const p of pts) ctx.lineTo(p.x, p.y);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.restore();

  /* --- head: rounded wedge with eyes + forked tongue, facing AWAY from
         the body so "snake head square" is visually obvious --- */
  const hr = C * 0.155;                            // head radius
  const ang = Math.atan2(-uy, -ux);                // away from tail
  ctx.save();
  ctx.translate(H.x, H.y);
  ctx.rotate(ang);

  /* head shape: teardrop — wide at the back, rounded snout forward */
  ctx.beginPath();
  ctx.moveTo(-hr * 1.05, 0);
  ctx.bezierCurveTo(-hr * 0.9, -hr * 0.92, hr * 0.35, -hr * 0.95, hr * 1.15, -hr * 0.28);
  ctx.bezierCurveTo(hr * 1.45, 0, hr * 1.45, 0, hr * 1.15, hr * 0.28);
  ctx.bezierCurveTo(hr * 0.35, hr * 0.95, -hr * 0.9, hr * 0.92, -hr * 1.05, 0);
  ctx.closePath();
  const hg = ctx.createRadialGradient(-hr * 0.15, -hr * 0.35, hr * 0.15, 0, 0, hr * 1.5);
  hg.addColorStop(0, '#ff96aa');
  hg.addColorStop(0.7, '#e04a63');
  hg.addColorStop(1, '#b23250');
  ctx.fillStyle = hg;
  ctx.fill();
  ctx.lineWidth = Math.max(1, C * 0.012);
  ctx.strokeStyle = 'rgba(60,8,22,.55)';
  ctx.stroke();

  /* eyes: white sclera + dark pupil, on top of the head */
  for (const s of [-1, 1]) {
    ctx.beginPath();
    ctx.arc(hr * 0.28, s * hr * 0.48, hr * 0.26, 0, Math.PI * 2);
    ctx.fillStyle = '#fdf6f8';
    ctx.fill();
    ctx.beginPath();
    ctx.arc(hr * 0.36, s * hr * 0.48, hr * 0.13, 0, Math.PI * 2);
    ctx.fillStyle = '#1a0d13';
    ctx.fill();
  }

  /* forked tongue flicking forward */
  ctx.strokeStyle = '#ff4d6d';
  ctx.lineWidth = Math.max(1.1, C * 0.024);
  ctx.lineCap = 'round';
  ctx.beginPath();
  ctx.moveTo(hr * 1.35, 0);
  ctx.lineTo(hr * 1.78, 0);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(hr * 1.78, 0); ctx.lineTo(hr * 2.08, -hr * 0.24);
  ctx.moveTo(hr * 1.78, 0); ctx.lineTo(hr * 2.08, hr * 0.24);
  ctx.stroke();
  ctx.restore();

  /* --- tail tip: small tapered rattle --- */
  ctx.save();
  ctx.translate(T.x, T.y);
  ctx.rotate(Math.atan2(uy, ux));
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.quadraticCurveTo(C * 0.14, -C * 0.05, C * 0.24, 0);
  ctx.quadraticCurveTo(C * 0.14, C * 0.05, 0, 0);
  ctx.fillStyle = '#8f2740';
  ctx.fill();
  ctx.restore();
}

/** Tokens for every live player, stacked/fanned when sharing a square. */
function snDrawTokens() {
  const g = SNG.game; if (!g) return;
  const C = SNG.cell;

  /* visual position per player: honours the animation hint */
  const posOf = p => {
    const v = SNG.viz;
    if (v && v.player === p) {
      const A = snCenter(v.from), B = snCenter(v.to);
      const t = Math.max(0, Math.min(1, v.t));
      let e;
      if (v.kind === 'jump') e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;   // easeInOutQuad
      else e = t * t * (3 - 2 * t);                                                       // smoothstep
      let x = A.x + (B.x - A.x) * e;
      let y = A.y + (B.y - A.y) * e;
      /* jumps follow a gentle ARC so climbs feel like climbing and slides
         like falling: lift toward the board centre on ladders, dip outward
         on snakes. Magnitude is proportional to the jump length. */
      if (v.kind === 'jump') {
        const len = Math.hypot(B.x - A.x, B.y - A.y);
        const arc = Math.min(len * 0.16, C * 0.9) * Math.sin(Math.PI * t);
        const dir = v.to > v.from ? -1 : 1;               // climb up / slide down
        y += arc * dir;
      }
      return { __pt: { x, y }, __square: v.to };
    }
    return { __square: g.players[p].pos };
  };

  /* count tokens per square for fan-out (uses visual position) */
  const stack = new Map();
  const vis = [];
  for (let p = 0; p < g.players.length; p++) {
    const v = posOf(p);
    vis.push(v);
    if (v.__square >= 1) {
      if (!stack.has(v.__square)) stack.set(v.__square, []);
      stack.get(v.__square).push(p);
    }
  }

  for (let p = 0; p < g.players.length; p++) {
    const v = vis[p];
    const col = SN_COLORS[SN_P[p]];
    let pt, sq;
    if (v.__pt) { pt = v.__pt; sq = v.__square; }
    else if (v.__square === 0) { pt = snStartSlot(p); sq = 0; }
    else { pt = snCenter(v.__square); sq = v.__square; }
    if (!pt) continue;

    /* fan out stacked tokens */
    const mates = stack.get(sq) || [p];
    if (mates.length > 1 && sq >= 1) {
      const idx = mates.indexOf(p);
      const spread = C * 0.17;
      pt = { x: pt.x + (idx % 2 ? spread : -spread), y: pt.y + (idx > 1 ? spread : -spread) };
    }

    const moving = SNG.anim && SNG.anim.player === p;
    snToken(pt.x, pt.y, C * 0.30, col, g.turn === p && g.winner === null, moving, p);
  }
}

/** Off-board start slot for player p (below square 1, fanned). */
function snStartSlot(p) {
  const C = SNG.cell;
  const one = snCenter(1);
  const off = [-1.5, -0.5, 0.5, 1.5][p] * C * 0.42;
  return { x: one.x + off, y: one.y + C * 0.95 };
}

/** One token: soft shadow, radial-gradient sphere with specular highlight,
    crisp ring, and a smooth halo for the active player. */
function snToken(x, y, R, col, isTurn, moving, p) {
  const ctx = SNG.ctx;
  const pulse = moving ? 1 + 0.08 * Math.sin(performance.now() / 90) : 1;
  const r = R * pulse;

  /* active-player halo: soft double glow ring (replaces old dashed debug ring) */
  if (isTurn) {
    for (const [mult, alpha] of [[1.62, .10], [1.34, .22]]) {
      ctx.beginPath();
      ctx.arc(x, y, R * mult, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(255,255,255,${alpha})`;
      ctx.lineWidth = 1.5;
      ctx.stroke();
    }
  }

  /* grounded drop shadow */
  ctx.beginPath();
  ctx.ellipse(x, y + r * 0.72, r * 0.86, r * 0.36, 0, 0, Math.PI * 2);
  ctx.fillStyle = 'rgba(0,0,0,.42)';
  ctx.fill();

  /* body: radial gradient lit from the upper-left, like a lacquered pawn */
  const grd = ctx.createRadialGradient(x - r * 0.38, y - r * 0.42, r * 0.12, x, y, r * 1.05);
  grd.addColorStop(0, '#ffffff');
  grd.addColorStop(0.18, col.fill);
  grd.addColorStop(0.75, col.fill);
  grd.addColorStop(1, col.dark);
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fillStyle = grd;
  ctx.fill();

  /* rim: light edge top, dark edge bottom (3D read) */
  ctx.lineWidth = Math.max(1.2, r * 0.13);
  ctx.strokeStyle = 'rgba(255,255,255,.5)';
  ctx.beginPath();
  ctx.arc(x, y, r - ctx.lineWidth * 0.35, Math.PI * 1.05, Math.PI * 1.95);
  ctx.stroke();
  ctx.strokeStyle = 'rgba(0,0,0,.28)';
  ctx.beginPath();
  ctx.arc(x, y, r - ctx.lineWidth * 0.35, Math.PI * 0.05, Math.PI * 0.95);
  ctx.stroke();

  /* specular dot */
  ctx.beginPath();
  ctx.ellipse(x - r * 0.34, y - r * 0.4, r * 0.2, r * 0.13, -0.6, 0, Math.PI * 2);
  ctx.fillStyle = 'rgba(255,255,255,.75)';
  ctx.fill();

  /* player initial */
  ctx.font = `800 ${Math.max(8, R * 0.9)}px Sora, sans-serif`;
  ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  ctx.fillStyle = 'rgba(10,14,20,.88)';
  ctx.fillText(['R', 'G', 'Y', 'B'][p], x, y + R * 0.05);
}

/* ---- confetti (Task 3) ------------------------------------------------------ */
const SNCF = { raf: 0, parts: [], w: 0, h: 0, canvas: null };

/** Launch a gentle confetti burst behind the winner overlay. Self-terminating
    and clock-guarded: any restart/teardown kills it instantly. */
function snConfettiStop() {
  if (SNCF.raf) cancelAnimationFrame(SNCF.raf);
  SNCF.raf = 0; SNCF.parts = [];
  if (SNCF.canvas) {
    const ctx = SNCF.canvas.getContext('2d');
    if (ctx) ctx.clearRect(0, 0, SNCF.w, SNCF.h);
  }
}
function snConfettiStart() {
  snConfettiStop();
  const cv = sn$('snConfetti'); if (!cv) return;
  if (SNG.reducedMotion) return;                       // respect users' setting
  SNCF.canvas = cv;
  const rect = cv.getBoundingClientRect();
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  SNCF.w = Math.max(1, Math.round(rect.width * dpr));
  SNCF.h = Math.max(1, Math.round(rect.height * dpr));
  cv.width = SNCF.w; cv.height = SNCF.h;
  const ctx = cv.getContext('2d');

  /* palette drawn from the game's own tokens + gold */
  const cols = ['#ff5d5d', '#5ef2c0', '#ffd166', '#6ea8ff', '#eaf2ff'];
  const N = 90;
  for (let i = 0; i < N; i++) {
    SNCF.parts.push({
      x: SNCF.w * (0.1 + Math.random() * 0.8),
      y: -SNCF.h * (0.05 + Math.random() * 0.5),
      vx: (Math.random() - 0.5) * 1.4,
      vy: 1.1 + Math.random() * 1.8,
      w: (5 + Math.random() * 5) * dpr,
      h: (8 + Math.random() * 7) * dpr,
      rot: Math.random() * Math.PI * 2,
      vr: (Math.random() - 0.5) * 0.22,
      col: cols[i % cols.length],
      sway: Math.random() * Math.PI * 2,
    });
  }

  const my = SNG.clock;
  const t0 = performance.now();
  const tick = now => {
    if (my !== SNG.clock || !SNG.viewAlive()) { snConfettiStop(); return; }
    const el = now - t0;
    ctx.clearRect(0, 0, SNCF.w, SNCF.h);
    let alive = 0;
    for (const p of SNCF.parts) {
      p.x += p.vx + Math.sin(el / 420 + p.sway) * 0.5;
      p.y += p.vy;
      p.rot += p.vr;
      if (p.y < SNCF.h + 20 && el < 5200) alive++;
      if (p.y > SNCF.h + 30) continue;
      ctx.save();
      ctx.translate(p.x, p.y);
      ctx.rotate(p.rot);
      ctx.globalAlpha = el > 4200 ? Math.max(0, 1 - (el - 4200) / 1000) : 0.95;
      ctx.fillStyle = p.col;
      ctx.fillRect(-p.w / 2, -p.h / 2, p.w, p.h);
      ctx.restore();
    }
    if (alive > 0) SNCF.raf = requestAnimationFrame(tick);
    else snConfettiStop();
  };
  SNCF.raf = requestAnimationFrame(tick);
}

/* ---- match statistics ------------------------------------------------------ */
/** Fresh per-player stat blocks for the current mode. */
function snResetStats() {
  const kinds = (SN_MODES[SNG.mode] || SN_MODES.pvc).kinds;
  SNG.stats = kinds.map(kind => ({
    kind,
    rolls: 0, moves: 0, snakes: 0, ladders: 0, sixes: 0,
  }));
}

/** Record one resolved roll for player p. `mv` is the engine's move result
    (overshoot counts as a roll but not a move; snake/ladder both count). */
function snStatRecord(p, dice, mv) {
  const s = SNG.stats && SNG.stats[p];
  if (!s) return;
  s.rolls++;
  if (dice === 6) s.sixes++;
  if (mv && !mv.overshoot) s.moves++;
  if (mv && mv.kind === 'snake') s.snakes++;
  if (mv && mv.kind === 'ladder') s.ladders++;
}

/* ---- HUD ------------------------------------------------------------------ */
function snRenderHud() {
  const g = SNG.game; if (!g) return;
  const chip = sn$('snTurnChip');
  if (chip) {
    const nm = snPlayerName(g.turn);
    /* possessive form for the chip: "Himanshu's Turn" (names ending in s
       get the bare apostrophe: "James' Turn") */
    const poss = /s$/i.test(nm) ? `${nm}'` : `${nm}'s`;
    chip.querySelector('.v').textContent = `${poss} Turn`;
    const dot = sn$('snTurnDot');
    if (dot) {
      const c = SN_COLORS[SN_P[g.turn]];
      dot.style.background = c.fill;
      dot.style.color = c.fill;
    }
  }
  const el = sn$('snRollCount');
  if (el) el.textContent = String(g.rolls);
  snRenderPlayers();
}

/** Sensible, distinct default names for AI opponents (never collides with
    the colour defaults humans fall back to). */
const SN_AI_NAMES = ['Nova', 'Orion', 'Vega', 'Pixel'];

/** The names actually in play for the CURRENT mode — the single source of
    truth every display reads from. Applies validation + duplicate handling
    once, in one place:
      • empty / whitespace-only → colour default (human) / AI default (AI)
      • duplicates → later entries get "· 2", "· 3"… so every player stays
        uniquely identifiable on screen and in the log. */
function snActiveNames() {
  const kinds = (SN_MODES[SNG.mode] || SN_MODES.pvc).kinds;
  const seen = new Map();
  return kinds.map((kind, i) => {
    const typed = (SNG.names[i] || '').trim().slice(0, 12);
    let nm = kind === 'ai' ? (SN_AI_NAMES[i] || ('Computer ' + (i + 1)))
                           : (typed || window.SnakesEngine.NAMES[i] || ('P' + (i + 1)));
    if (seen.has(nm)) { const n = seen.get(nm) + 1; seen.set(nm, n); nm = `${nm} · ${n}`; }
    else seen.set(nm, 1);
    return nm;
  });
}

/** Display name for player index (kind-aware). Reads snActiveNames() so
    turn chip, chips, log and overlay always agree, duplicates included. */
function snPlayerName(p) {
  const g = SNG.game;
  if (!g || !g.players[p]) return ('P' + (p + 1));
  return snActiveNames()[p] || ('P' + (p + 1));
}

/** Default display name for player p when no custom name is set. */
function snDefaultName(p) {
  const kinds = (SN_MODES[SNG.mode] || SN_MODES.pvc).kinds;
  if (kinds[p] === 'ai') return SN_AI_NAMES[p] || ('Computer ' + (p + 1));
  return window.SnakesEngine.NAMES[p] || ('P' + (p + 1));
}

/** Side-rail player chips with live positions + per-player mini-stats. */
function snRenderPlayers() {
  const g = SNG.game; if (!g) return;
  const host = sn$('snPlayers');
  if (!host) return;
  const names = snActiveNames();
  host.innerHTML = g.players.map((P, i) => {
    const c = SN_COLORS[SN_P[i]];
    const cls = g.winner === i ? 'winner'
      : (g.turn === i && g.winner === null ? 'turn' : '');
    const tag = P.kind === 'ai' ? '<span class="tag">AI</span>' : '';
    const pos = P.pos === 0 ? '—' : String(P.pos);
    const s = (SNG.stats && SNG.stats[i]) || { rolls: 0, snakes: 0, ladders: 0 };
    return `<div class="sn-pl ${cls}" data-p="${i}">
      <span class="dot" style="background:${c.fill};color:${c.fill}"></span>
      <span class="nm"><span class="nm-t">${names[i]}</span>${tag}</span>
      <span class="stats" title="rolls · snakes · ladders">${s.rolls}·${s.snakes}·${s.ladders}</span>
      <span class="sq">${pos}</span>
    </div>`;
  }).join('');
}

function snMsg(html) {
  const el = sn$('snMsg'); if (el) el.innerHTML = html;
}

/** Push one line onto the scrolling event log. */
function snLogLine(html) {
  const el = sn$('snLog'); if (!el) return;
  const d = document.createElement('div');
  d.innerHTML = html;
  el.prepend(d);
  while (el.children.length > 40) el.removeChild(el.lastChild);
}

/* ---- dice widget --------------------------------------------------------- */
const SN_PIPS = { 1: [4], 2: [0, 8], 3: [0, 4, 8], 4: [0, 2, 6, 8], 5: [0, 2, 4, 6, 8], 6: [0, 2, 3, 5, 6, 8] };

function snSetDice(v, rolling) {
  const die = document.querySelector('#snPanel .sn-die');
  if (die) {
    die.classList.toggle('rolling', !!rolling);
    if (v != null) {
      const dots = die.querySelectorAll('i');
      dots.forEach((d, i) => d.style.visibility = SN_PIPS[v].includes(i) ? 'visible' : 'hidden');
      /* landing pop: retrigger the keyframe on every new value */
      die.classList.remove('landed');
      void die.offsetWidth;                              // restart animation
      die.classList.add('landed');
    } else {
      die.querySelectorAll('i').forEach(d => d.style.visibility = 'hidden');
    }
  }
  const val = sn$('snDiceVal');
  if (val) val.textContent = rolling ? '·' : (v == null ? '—' : String(v));
  snRenderControls();
}

function snRenderControls() {
  const g = SNG.game;
  const btn = sn$('snRollBtn');
  if (!btn || !g) return;
  const human = g.players[g.turn].kind === 'human';
  const enabled = g.winner === null && human && SNG.phase === 'idle';
  btn.disabled = !enabled;
  const lbl = sn$('snRollLbl');
  if (lbl) lbl.textContent = g.winner !== null ? 'game over'
    : !human ? 'AI is thinking…'
    : SNG.phase === 'idle' ? 'Roll Dice'
    : 'moving…';
}

/* ---- overlay ---------------------------------------------------------------- */
function snOverlayShow(opts) {
  const ov = sn$('snOverlay'); if (!ov) return;
  sn$('snOvTitle').innerHTML = opts.title || '';
  sn$('snOvSub').innerHTML = opts.sub || '';
  sn$('snOvRow').innerHTML = (opts.row || []).map(b =>
    `<button type="button" class="sn-ov-btn${b.primary ? ' primary' : ''}" data-act="${b.act}">${b.label}</button>`).join('');
  /* optional per-player match-summary table (Task 4) */
  const tbl = sn$('snOvTable');
  if (tbl) {
    if (opts.table) { tbl.innerHTML = opts.table; tbl.classList.remove('hidden'); }
    else { tbl.innerHTML = ''; tbl.classList.add('hidden'); }
  }
  const stats = sn$('snOvStats');
  if (opts.stats) {
    stats.innerHTML = opts.stats.map(s => `<div class="sn-ov-stat"><div class="k">${s.k}</div><div class="v">${s.v}</div></div>`).join('');
    stats.classList.remove('hidden');
  } else stats.classList.add('hidden');
  ov.classList.remove('hidden');
}
function snOverlayHide() {
  const ov = sn$('snOverlay'); if (ov) ov.classList.add('hidden');
  snConfettiStop();
}

/* =====================================================================
   TURN CONTROLLER — the single owner of the game clock.
   Every async step captures `my` epoch and re-checks it before acting,
   so restart/teardown/route-away can never leave a zombie timer alive.
   ===================================================================== */
function snStartTurn() {
  const my = ++SNG.clock;
  const g = SNG.game;
  if (!g || g.winner !== null) return;

  SNG.phase = 'idle';
  SNG.dice = null;
  SNG.viz = null;
  snSetDice(null, false);
  snRenderHud(); snDraw(); snRenderControls();

  const nm = snPlayerName(g.turn);
  if (g.players[g.turn].kind === 'ai') {
    snMsg(`<b>${nm}</b> is taking its turn…`);
    setTimeout(() => { if (my === SNG.clock && SNG.viewAlive()) snAiRoll(); }, SN_T.aiThink);
  } else {
    snMsg(`<b>${nm}'s</b> turn — roll the dice.`);
  }
}

/** Human pressed Roll (or AI auto-roll): throw the die. */
function snDoRoll() {
  const g = SNG.game;
  if (!g || g.winner !== null) return;
  if (SNG.phase !== 'idle') return;                    // one roll at a time
  SNG.phase = 'rolling';
  snSetDice(null, true);
  snSfx('roll');
  const my = SNG.clock;
  setTimeout(() => {
    if (my !== SNG.clock || !SNG.viewAlive()) return;   // stale game
    const v = window.SnakesEngine.rollDice();
    SND.onRoll(v);
  }, SN_T.rollSpin);
}

/** Roll resolved: animate the walk, then the snake/ladder, then resolve. */
function snResolveRoll(v) {
  const my = SNG.clock;
  const g = SNG.game;
  if (!g || g.winner !== null) return;

  SNG.dice = v;
  snSetDice(v, false);
  snSfx('dice');                                   // result reveal tone

  const mv = window.SnakesEngine.previewMove(g, g.turn, v);
  if (!mv) { snStartTurn(); return; }                   // defensive

  const p = g.turn;
  const nm = snPlayerName(p);
  const col = SN_COLORS[SN_P[p]].fill;

  /* overshoot: token does not move at all */
  if (mv.overshoot) {
    const need = 100 - mv.from;
    window.SnakesEngine.applyRoll(g, v);                // commits + advances turn
    snStatRecord(p, v, mv);
    snSfx('stuck');
    snMsg(`<b>${nm}</b> rolled <b>${v}</b> but needs exactly <b>${need}</b> to finish — overshoots, stays on <b>${mv.from}</b>.`);
    snLogLine(`<span class="who" style="color:${col}">${nm}</span> rolled <b>${v}</b> <span class="bad">overshoots</span> · stuck at ${mv.from}`);
    snRenderHud(); snDraw();
    setTimeout(() => { if (my === SNG.clock && SNG.viewAlive()) snAfterMove(); }, SN_T.eventMs);
    return;
  }

  /* build the square-by-square walking path */
  const path = [];
  for (let s = mv.from + 1; s <= mv.to; s++) path.push(s);
  if (path.length === 0) path.push(mv.to);              // safety

  SNG.phase = 'moving';
  snRenderControls();
  if (mv.from === 0) snSfx('enter');

  SNG.anim = { player: p, path, i: 0, raf: 0 };
  snWalkStep();
}

/** One square of the walk, animated with requestAnimationFrame. */
function snCancelAnim() {
  if (SNG.anim && SNG.anim.raf) cancelAnimationFrame(SNG.anim.raf);
  SNG.anim = null;
  SNG.viz = null;
}

function snWalkStep() {
  const my = SNG.clock;
  const a = SNG.anim;
  if (!a) return;

  if (a.i >= a.path.length) { snWalkDone(); return; }

  const g = SNG.game;
  const p = a.player;
  const from = a.i === 0 ? g.players[p].pos : a.path[a.i - 1];
  const to = a.path[a.i];
  const start = performance.now();
  const dur = Math.max(30, SN_T.stepMs);

  const tick = now => {
    if (my !== SNG.clock || !SNG.viewAlive()) { SNG.anim = null; SNG.viz = null; return; }
    const aa = SNG.anim;
    if (!aa || aa !== a) return;                        // replaced/cancelled
    const t = Math.min(1, (now - start) / dur);
    SNG.viz = { player: p, from: from || 1, to, t, kind: 'walk' };
    snDraw();
    if (t < 1) {
      aa.raf = requestAnimationFrame(tick);
    } else {
      SNG.viz = null;
      snSfx('step');
      aa.i++;
      if (aa.i >= a.path.length) snWalkDone();
      else snWalkStep();                               // next hop (fresh closure)
    }
  };
  a.raf = requestAnimationFrame(tick);
}

/** Walk finished: apply the roll through the ENGINE, then animate jumps. */
function snWalkDone() {
  const my = SNG.clock;
  const g = SNG.game;
  const a = SNG.anim;
  if (!g || !a) { SNG.anim = null; return; }
  SNG.anim = null;
  SNG.viz = null;

  const p = a.player;
  const mv = window.SnakesEngine.applyRoll(g, SNG.dice);   // rules own the truth
  if (!mv) { snStartTurn(); return; }
  snStatRecord(p, SNG.dice, mv);

  const nm = snPlayerName(p);
  const col = SN_COLORS[SN_P[p]].fill;

  /* win: celebrate immediately */
  if (mv.won) {
    snRenderHud(); snDraw();
    snSfx('win');
    snMusicFollow();                               // winner → music stops
    snMsg(`<b>${nm}</b> landed exactly on <b>100</b>. Game over!`);
    snLogLine(`<span class="who" style="color:${col}">${nm}</span> rolled <b>${SNG.dice}</b> → <b>100</b> <span class="win">WINNER!</span>`);
    setTimeout(() => { if (my === SNG.clock && SNG.viewAlive()) snGameOver(); }, SN_T.eventMs);
    return;
  }

  /* ladder climb / snake slide animation */
  if (mv.climb || mv.slide) {
    const isLadder = !!mv.climb;
    const [from, to] = isLadder ? mv.climb : mv.slide;
    snSfx(isLadder ? 'ladder' : 'snake');
    snMsg(isLadder
      ? `<b>${nm}</b> climbed a ladder from <b>${from}</b> up to <b>${to}</b>!`
      : `<b>${nm}</b> got bitten by a snake — down from <b>${from}</b> to <b>${to}</b>.`);
    snLogLine(`<span class="who" style="color:${col}">${nm}</span> ${isLadder ? 'climbed a ladder' : 'bitten by a snake'} ${from} <span class="${isLadder ? 'good' : 'bad'}">→ ${to}</span>`);
    snAnimateJump(p, from, to);
    return;
  }

  /* plain move */
  snMsg(`<b>${nm}</b> rolled <b>${SNG.dice}</b> and moved to <b>${mv.final}</b>.`);
  snLogLine(`<span class="who" style="color:${col}">${nm}</span> rolled <b>${SNG.dice}</b> <span class="arrow">→</span> ${mv.final}`);
  snRenderHud(); snDraw();
  setTimeout(() => { if (my === SNG.clock && SNG.viewAlive()) snAfterMove(); }, SN_T.settleMs);
}

/** Animate a snake slide or ladder climb between two squares. */
function snAnimateJump(p, from, to) {
  const my = SNG.clock;
  SNG.phase = 'moving';
  const start = performance.now();
  const dur = Math.max(60, SN_T.jumpMs);

  const tick = now => {
    if (my !== SNG.clock || !SNG.viewAlive()) { SNG.viz = null; return; }
    const t = Math.min(1, (now - start) / dur);
    SNG.viz = { player: p, from, to, t, kind: 'jump' };
    snDraw();
    if (t < 1) requestAnimationFrame(tick);
    else {
      SNG.viz = null;
      snRenderHud(); snDraw();
      setTimeout(() => { if (my === SNG.clock && SNG.viewAlive()) snAfterMove(); }, SN_T.eventMs);
    }
  };
  requestAnimationFrame(tick);
}

/** Move fully consumed: winner check already done; hand the dice on. */
function snAfterMove() {
  const g = SNG.game;
  if (!g) return;
  if (g.winner !== null) { snGameOver(); return; }
  snStartTurn();
}

function snAiRoll() {
  const g = SNG.game;
  if (!g || g.winner !== null) return;
  if (!window.SnakesEngine.aiShouldRoll(g)) return;    // never act for humans
  if (SNG.phase !== 'idle') return;
  snDoRoll();
}

function snGameOver() {
  const g = SNG.game;
  if (!g || g.winner === null) return;
  SNG.phase = 'over';
  snRenderControls();
  const w = g.winner;
  const nm = snPlayerName(w);
  const soloHumanWin = g.players[w].kind === 'human' && g.players.filter(P => P.kind === 'human').length === 1;
  const humans = g.players.filter(P => P.kind === 'human').length;

  /* per-player stats (Task 4): totals + per-player breakdown */
  const st = SNG.stats;
  const tot = (k) => st.reduce((a, s) => a + (s[k] || 0), 0);
  const winSt = st[w] || { rolls: 0, snakes: 0, ladders: 0 };

  snSfx(soloHumanWin ? 'win' : 'win');
  snMusicFollow();                                 // game over → music stops
  snMsg(`<b>${nm}</b> reached square 100 exactly. Game over!`);
  snRenderHud(); snDraw();

  /* match summary table: one row per player, winner highlighted */
  const names = snActiveNames();
  const tableRows = g.players.map((P, i) => {
    const c = SN_COLORS[SN_P[i]];
    const s = st[i] || { rolls: 0, moves: 0, snakes: 0, ladders: 0, sixes: 0 };
    return `<tr class="${i === w ? 'winner-row' : ''}">
      <td><span class="tdot" style="background:${c.fill}"></span>${names[i]}${P.kind === 'ai' ? ' <span class="tag">AI</span>' : ''}</td>
      <td>${s.rolls}</td><td>${s.moves}</td><td>${s.snakes}</td><td>${s.ladders}</td><td>${s.sixes}</td>
    </tr>`;
  }).join('');

  const crown = `<svg class="sn-ov-crown" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M3 8.5l4.2 3L12 4.5l4.8 7 4.2-3-1.6 9a2 2 0 0 1-2 1.7H6.6a2 2 0 0 1-2-1.7L3 8.5z" fill="#ffd166"/>
  </svg>`;

  snOverlayShow({
    title: `${crown}<div class="sn-ov-winner pop"><span class="sn-ov-name ${soloHumanWin ? 'humanWin' : ''}">${nm}</span></div>`,
    sub: soloHumanWin
      ? `${nm} landed exactly on square 100 — a flawless race to the top!`
      : humans > 1
        ? `${nm} landed exactly on square 100 first. Well played everyone — rematch?`
        : `${nm} reached square 100 first. Try again?`,
    stats: [
      { k: 'total rolls', v: g.rolls },
      { k: 'snakes hit', v: tot('snakes') },
      { k: 'ladders climbed', v: tot('ladders') },
    ],
    table: `<div class="sn-ov-table"><table>
      <thead><tr><th>player</th><th>rolls</th><th>moves</th><th>snakes</th><th>ladders</th><th>sixes</th></tr></thead>
      <tbody>${tableRows}</tbody></table></div>`,
    row: [
      { act: 'again', label: 'Play Again', primary: true },
      { act: 'menu', label: 'Back to Games' },
    ],
  });
  snConfettiStart();
}

/* ---- new game / restart ------------------------------------------------------- */
/** Mode -> player kinds (the deterministic mode registry). */
const SN_MODES = {
  pvc:  { label: 'vs Computer',       short: 'P vs C', kinds: ['human', 'ai'] },
  '2p': { label: '2 Players',         short: '2P',     kinds: ['human', 'human'] },
  '3p': { label: '3 Players',         short: '3P',     kinds: ['human', 'human', 'human'] },
  '4p': { label: '4 Players',         short: '4P',     kinds: ['human', 'human', 'human', 'human'] },
  '2vc':{ label: '2 Players vs Computer', short: '2P vs C', kinds: ['human', 'human', 'ai'] },
};

function snNewGame() {
  /* NOTE: the view epoch is NOT bumped here — the view stays mounted and
     its listeners must keep working across restarts (the game CLOCK bump
     below already invalidates every pending turn-flow callback). Bumping
     the epoch here would kill all control listeners after the very first
     restart. Mirrors luNewGame's documented behaviour.
     REMATCH semantics (Task 5): SNG.names is intentionally untouched, so
     names + colours carry over; positions/dice/turn/stats reset below. */
  SNG.clock++;                       // kill every pending flow callback
  snCancelAnim();
  snConfettiStop();
  const kinds = (SN_MODES[SNG.mode] || SN_MODES.pvc).kinds;
  SNG.game = window.SnakesEngine.createGame(kinds);
  snResetStats();                    // fresh per-player stat blocks
  SNG.phase = 'idle'; SNG.dice = null;
  snOverlayHide();
  snSetDice(null, false);
  const log = sn$('snLog'); if (log) log.innerHTML = '';
  snRenderHud(); snDraw(); snRenderControls();
  const who = snActiveNames().join(', ');
  snMsg(`New game: <b>${who}</b>. First to land exactly on 100 wins.`);
  snSfx('start');                                  // game-start fanfare
  snMusicFollow();                                 // music follows new game
  snStartTurn();
}

/* ---- view ------------------------------------------------------------------------
   Mounted by the console router at #funtime/sn.
   ---------------------------------------------------------------------------------- */
function renderSnView() {
  document.getElementById('viewRoot').innerHTML = `
  <div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Snakes &amp; Ladders</div>
    </div>

    <section class="ft-panel ft-game sn-wide" id="snPanel" aria-label="Snakes and Ladders game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Snakes &amp; Ladders</div>
          <div class="ft-blurb">The classic 100-square race. Climb the ladders, dodge the snakes, land exactly on 100 to win.</div>
        </div>
      </header>

      <div class="sn-topbar">
        <div class="sn-bar-row">
          <div class="sn-hud" role="status" aria-live="polite">
            <span id="snTurnChip"><span id="snTurnDot" aria-hidden="true"></span><span class="k">TURN</span> <span class="v">Red's Turn</span></span>
            <span class="sn-stat"><span class="k">ROLLS</span><span class="v" id="snRollCount">0</span></span>
            <span class="sn-stat"><span class="k">MODE</span><span class="v" id="snModeChip">P vs C</span></span>
          </div>
          <div class="sn-setup" style="margin-left:auto">
            <div class="sn-setup-grid">
              <div class="sn-field">
                <label>Mode</label>
                <div class="sn-seg" role="group" aria-label="Game mode" id="snModeSeg">
                  <button type="button" data-v="pvc">vs Computer</button>
                  <button type="button" data-v="2p">2 Players</button>
                  <button type="button" data-v="3p">3 Players</button>
                  <button type="button" data-v="4p">4 Players</button>
                  <button type="button" data-v="2vc">2P vs Computer</button>
                </div>
              </div>
              <div class="sn-field" id="snNamesField">
                <label>Players</label>
                <div class="sn-names" id="snNames"></div>
              </div>
              <div class="sn-setup-actions">
                <button type="button" class="ft-btn primary" id="snStart">Start Game</button>
              </div>
            </div>
          </div>
        </div>
        <div class="sn-bar-row sn-ctl-row">
          <div class="sn-controls" role="group" aria-label="Game controls">
            <button type="button" class="ft-btn" id="snRestartBtn">Restart</button>
            <button type="button" class="ft-btn si-sound" id="snSoundBtn" aria-pressed="true"
                    aria-haspopup="dialog" aria-expanded="false">
              <span class="si-snd-ico" aria-hidden="true">♪</span><span id="snSoundLbl">sound on</span>
            </button>
            <button type="button" class="ft-btn" id="snFsBtn" aria-pressed="false">Fullscreen</button>
            <button type="button" class="ft-btn si-exit" id="snExit">Back to Games</button>
          </div>
          <!-- audio settings popover: master/music/sfx toggles + volumes.
               Anchored under the controls, dismissed by outside click;
               persists via localStorage ('snakes-audio-v1'). -->
          <div class="sn-audio-pop hidden" id="snAudioPop" role="dialog"
               aria-label="Audio settings" aria-modal="false">
            <div class="sn-au-row sn-au-head">
              <span class="sn-au-title">AUDIO</span>
              <button type="button" class="sn-au-x" id="snAudioClose"
                      aria-label="Close audio settings">✕</button>
            </div>
            <div class="sn-au-row">
              <label class="sn-au-l" for="snAuMasterV">Master volume</label>
              <input type="range" id="snAuMasterV" class="sn-au-slider"
                     min="0" max="100" step="1" aria-label="Master volume">
              <span class="sn-au-val" id="snAuMasterVal"></span>
            </div>
            <div class="sn-au-row">
              <label class="sn-au-l" for="snAuMusicV">Music volume</label>
              <input type="range" id="snAuMusicV" class="sn-au-slider"
                     min="0" max="100" step="1" aria-label="Music volume">
              <span class="sn-au-val" id="snAuMusicVal"></span>
            </div>
            <div class="sn-au-row">
              <label class="sn-au-l" for="snAuSfxV">Effects volume</label>
              <input type="range" id="snAuSfxV" class="sn-au-slider"
                     min="0" max="100" step="1" aria-label="Sound effects volume">
              <span class="sn-au-val" id="snAuSfxVal"></span>
            </div>
            <div class="sn-au-row sn-au-toggles">
              <button type="button" class="sn-au-toggle" id="snAuMusicT"
                      aria-pressed="true"><span class="sn-au-dot" aria-hidden="true"></span>Music</button>
              <button type="button" class="sn-au-toggle" id="snAuSfxT"
                      aria-pressed="true"><span class="sn-au-dot" aria-hidden="true"></span>Effects</button>
            </div>
          </div>
          <div class="sn-msg" id="snMsg" aria-live="polite"></div>
        </div>
      </div>

      <div class="sn-layout" id="snLayout">
        <div class="sn-stage" id="snStageWrap">
          <canvas id="snBoard" aria-label="Snakes and Ladders board, 100 numbered squares"
                  role="img">Your browser does not support the canvas element.</canvas>
          <div class="sn-overlay hidden" id="snOverlay" role="dialog" aria-modal="false" aria-label="Game status">
            <canvas id="snConfetti" aria-hidden="true"></canvas>
            <div class="sn-ov-inner">
              <div class="sn-ov-title" id="snOvTitle">Snakes &amp; Ladders</div>
              <div class="sn-ov-sub" id="snOvSub"></div>
              <div class="sn-ov-stats hidden" id="snOvStats"></div>
              <div id="snOvTable" class="hidden"></div>
              <div class="sn-ov-row" id="snOvRow"></div>
            </div>
          </div>
        </div>

        <aside class="sn-side">
          <div class="sn-card">
            <h4>Dice</h4>
            <div class="sn-dice-row">
              <span class="sn-die" aria-hidden="true">${snDieFaceDom(6)}</span>
              <div class="sn-dice-info">
                <div class="sn-dice-val" id="snDiceVal">—</div>
                <div class="sn-dice-lbl" id="snDiceLbl">last roll</div>
              </div>
            </div>
            <button type="button" class="sn-roll-btn" id="snRollBtn">
              <span id="snRollLbl">Roll Dice</span>
              <span class="hint">space</span>
            </button>
          </div>

          <div class="sn-card">
            <h4>Players</h4>
            <div class="sn-players" id="snPlayers"></div>
          </div>

          <div class="sn-card">
            <h4>Moves</h4>
            <div class="sn-log" id="snLog" aria-live="polite"></div>
          </div>
        </aside>
      </div>

      <div class="sn-help">
        <span>land on a ladder bottom to climb</span><span>land on a snake head to slide down</span>
        <span>exact roll needed for 100</span><span>space rolls</span><span>F fullscreen</span>
      </div>
    </section>
  </div></div>`;

  SNG.epoch++;
  const myEpoch = SNG.epoch;
  SNG.clock++;
  SNG.cleanup = [];
  SNG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const snOnLocal = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    SNG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };
  SNG._on = snOnLocal;

  SNG.canvas = sn$('snBoard');
  SNG.ctx = SNG.canvas.getContext('2d');

  /* Any real user gesture inside the game may unlock audio (autoplay
     policy): one delegated capture-phase pointerdown does it for every
     button/slider/click without touching each handler. */
  snOnLocal(SNG.canvas.ownerDocument.body, 'pointerdown', () => {
    if (myEpoch !== SNG.epoch) return;
    snAudioReady();
    if (SNA.ctx && SNA.ctx.state === 'running') snMusicFollow();
  }, true);

  /* ---- mode selector + player names (local multiplayer) ---- */
  const syncSeg = () => {
    document.querySelectorAll('#snModeSeg button').forEach(b => {
      const on = b.dataset.v === SNG.mode;
      b.classList.toggle('on', on);
      b.setAttribute('aria-pressed', String(on));
    });
    const chip = sn$('snModeChip');
    if (chip) chip.textContent = (SN_MODES[SNG.mode] || SN_MODES.pvc).short;
  };

  /** Rebuild the per-player name rows for the selected mode.
      Custom names live in SNG.names and survive restarts / mode switches. */
  const snRenderNames = () => {
    const host = sn$('snNames'); if (!host) return;
    const kinds = (SN_MODES[SNG.mode] || SN_MODES.pvc).kinds;
    host.innerHTML = kinds.map((kind, i) => {
      const c = SN_COLORS[SN_P[i]];
      if (kind === 'ai') {
        return `<div class="sn-name-row ai" data-p="${i}">
          <span class="dot" style="background:${c.fill};color:${c.fill}"></span>
          <span class="sn-name-static">${SN_AI_NAMES[i] || ('Computer ' + (i + 1))}</span>
          <span class="ai-tag">AI</span>
        </div>`;
      }
      return `<div class="sn-name-row" data-p="${i}">
        <span class="dot" style="background:${c.fill};color:${c.fill}"></span>
        <input type="text" class="sn-name-input" id="snName${i}" data-p="${i}"
               maxlength="12" autocomplete="off" spellcheck="false"
               placeholder="${snDefaultName(i)}" value="${(SNG.names[i] || '').replace(/"/g, '&quot;')}">
      </div>`;
    }).join('');
    /* keep SNG.names in sync while typing (no restart needed mid-game:
       the chips/log/turn indicator re-render on the next HUD paint).
       Duplicates are handled downstream by snActiveNames() ("· 2" suffix),
       but flag them visually here so players notice immediately. */
    host.querySelectorAll('.sn-name-input').forEach(inp => {
      snOnLocal(inp, 'input', () => {
        const p = Number(inp.dataset.p);
        const v = inp.value.trim().slice(0, 12);
        SNG.names[p] = v;                       // '' falls back to default
        /* duplicate check against other HUMAN inputs of this mode */
        const others = [...host.querySelectorAll('.sn-name-input')]
          .filter(o => o !== inp)
          .map(o => o.value.trim().toLowerCase());
        const dup = v && others.includes(v.toLowerCase());
        inp.classList.toggle('dup', dup);
        inp.title = dup ? 'another player is using this name' : '';
        if (SNG.game && SNG.game.players[p]) snRenderPlayers();
      });
    });
  };
  SNG._renderNames = snRenderNames;

  syncSeg();
  snRenderNames();
  document.querySelectorAll('#snModeSeg button').forEach(b => {
    snOnLocal(b, 'click', () => {
      if (myEpoch !== SNG.epoch) return;
      SNG.mode = b.dataset.v;
      syncSeg();
      snRenderNames();
      snSfx('ui');
      snNewGame();
    });
  });

  /* ---- controls ---- */
  snOnLocal(sn$('snStart'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    snSfx('ui');
    snNewGame();
  });

  snOnLocal(sn$('snRollBtn'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    const g = SNG.game;
    if (!g || g.winner !== null) return;
    if (g.players[g.turn].kind !== 'human') return;
    if (SNG.phase !== 'idle') return;
    snDoRoll();          // audio already unlocked by the delegated gesture
  });

  snOnLocal(sn$('snRestartBtn'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    snSfx('ui');
    snNewGame();
  });

  /* ---- audio settings popover ------------------------------------------
     The sound button is a master mute + the entry point to the full
     settings panel. State lives in SNG.audio (persisted); the UI is a
     plain read/write of that object, so refresh/restart both restore
     exactly what was saved. */
  const auPop = sn$('snAudioPop');
  const auSyncUi = () => {
    const a = SNG.audio;
    const on = a.m > 0 && (a.sx || a.mu);
    SNG.soundOn = on;                                // legacy gate
    const b = sn$('snSoundBtn');
    if (b) {
      b.setAttribute('aria-pressed', String(on));
      b.classList.toggle('on', on);
      const lbl = sn$('snSoundLbl');
      if (lbl) lbl.textContent = on ? 'sound on' : 'muted';
    }
    const set = (id, v) => { const el = sn$(id); if (el) el.value = String(Math.round(v * 100)); };
    const txt = (id, v) => { const el = sn$(id); if (el) el.textContent = Math.round(v * 100) + '%'; };
    set('snAuMasterV', a.m);  txt('snAuMasterVal', a.m);
    set('snAuMusicV', a.mv);  txt('snAuMusicVal', a.mv);
    set('snAuSfxV', a.sv);    txt('snAuSfxVal', a.sv);
    const tg = (id, onn) => { const el = sn$(id); if (el) el.setAttribute('aria-pressed', String(!!onn)); };
    tg('snAuMusicT', a.mu); tg('snAuSfxT', a.sx);
  };
  const auOpen = () => {
    if (!auPop) return;
    auPop.classList.remove('hidden');
    sn$('snSoundBtn').setAttribute('aria-expanded', 'true');
    snSfx('ui');
  };
  const auClose = () => {
    if (!auPop) return;
    auPop.classList.add('hidden');
    const b = sn$('snSoundBtn');
    if (b) b.setAttribute('aria-expanded', 'false');
  };
  const auToggleOpen = () => auPop && auPop.classList.contains('hidden') ? auOpen() : auClose();

  snOnLocal(sn$('snSoundBtn'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    /* First click on a MUTED state unmutes; otherwise it opens the
       settings popover (mute itself is the master slider at 0). */
    if (!SNG.soundOn && SNG.audio.m === 0) {
      SNG.audio.m = 0.7;
      snAudioApplyVolumes(); snAudioSave(); auSyncUi();
      snMusicFollow();                               // music may resume
      snSfx('ui');                                   // audible confirmation
      return;
    }
    auToggleOpen();
  });
  snOnLocal(sn$('snAudioClose'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    auClose();
  });

  /* volume sliders — apply live, persist on every change (cheap writes) */
  const auSlider = (inputId, key, valId) => {
    const el = sn$(inputId); if (!el) return;
    snOnLocal(el, 'input', () => {
      SNG.audio[key] = Math.max(0, Math.min(1, Number(el.value) / 100));
      snAudioApplyVolumes(); snAudioSave(); auSyncUi();
      snMusicFollow();                               // music may start/stop
    });
  };
  auSlider('snAuMasterV', 'm', 'snAuMasterVal');
  auSlider('snAuMusicV', 'mv', 'snAuMusicVal');
  auSlider('snAuSfxV', 'sv', 'snAuSfxVal');

  /* toggles — instant mute via the gain graph (no clicks, no restart) */
  snOnLocal(sn$('snAuMusicT'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    SNG.audio.mu = SNG.audio.mu ? 0 : 1;
    snAudioApplyVolumes(); snAudioSave(); auSyncUi();
    snMusicFollow();
    snSfx('ui');
  });
  snOnLocal(sn$('snAuSfxT'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    SNG.audio.sx = SNG.audio.sx ? 0 : 1;
    snAudioApplyVolumes(); snAudioSave(); auSyncUi();
    snSfx('ui');                                      // plays only if now on
  });

  /* dismiss: outside click (Escape stays owned by the shared session
     controller — it exits the game view for every Fun Time game, and
     hijacking it here would break that global contract). */
  snOnLocal(document, 'pointerdown', e => {
    if (myEpoch !== SNG.epoch) return;
    if (!auPop || auPop.classList.contains('hidden')) return;
    if (auPop.contains(e.target) || sn$('snSoundBtn').contains(e.target)) return;
    auClose();
  });

  auSyncUi();                                        // paint persisted state

  snOnLocal(sn$('snFsBtn'), 'click', () => {
    if (myEpoch !== SNG.epoch) return;
    if (typeof ftToggleFullscreen === 'function') ftToggleFullscreen('sn', 'sn-fs', null);
  });
  if (typeof ftFsSupported === 'function' && !ftFsSupported()) {
    const b = sn$('snFsBtn'); if (b) b.style.display = 'none';
  }
  /* Reflect genuine fullscreen state onto the body class + button label.
     Same convention as Space Invaders: this is what makes the Fullscreen
     BUTTON path (not just the card-click launch) apply the .sn-fs layout,
     and it clears every other game's fs class defensively. */
  const snFsReflect = () => {
    if (typeof ftFsReflect === 'function') ftFsReflect('sn', 'sn-fs', '#snFsBtn');
    snResize();
  };
  snOnLocal(document, 'fullscreenchange', snFsReflect);
  snOnLocal(document, 'webkitfullscreenchange', snFsReflect);

  snOnLocal(sn$('snExit'), 'click', () => { location.hash = '#funtime'; });

  /* ---- overlay actions ---- */
  snOnLocal(sn$('snOvRow'), 'click', e => {
    if (myEpoch !== SNG.epoch) return;
    const b = e.target.closest('.sn-ov-btn');
    if (!b) return;
    if (b.dataset.act === 'again') snNewGame();
    else if (b.dataset.act === 'menu') location.hash = '#funtime';
  });

  /* ---- keyboard ---- */
  snOnLocal(window, 'keydown', e => {
    if (myEpoch !== SNG.epoch) return;
    if (e.target && (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA')) return;
    if (e.key === 'f' || e.key === 'F') {
      if (typeof ftToggleFullscreen === 'function') ftToggleFullscreen('sn', 'sn-fs', null);
    } else if (e.key === 'r' || e.key === 'R') {
      snNewGame();
    } else if (e.key === ' ' && SNG.game && SNG.game.winner === null) {
      const g = SNG.game;
      if (g.players[g.turn].kind === 'human' && SNG.phase === 'idle') {
        e.preventDefault();
        snDoRoll();
      }
    }
  });

  /* ---- resize ---- */
  snOnLocal(window, 'resize', () => { if (myEpoch === SNG.epoch) snResize(); });

  /* ---- boot ---- */
  snResize();
  snNewGame();

  /* Shared session lifecycle: automatic fullscreen from the card click,
     Esc → back to Fun Time, dashboard chrome hidden while playing. */
  if (typeof ftSessionAfterMount === 'function') ftSessionAfterMount('sn', 'sn-fs');
  if (typeof ftSessionBind === 'function') ftSessionBind(snOnLocal, () => snResize());
  /* The session classes are what give #snPanel its viewport height;
     refit the board now that they have landed. */
  snResize();
}

/** Static die markup for the dice card (initial face). */
function snDieFaceDom(n) {
  let dots = '';
  for (let i = 0; i < 9; i++) dots += `<i style="grid-area:${Math.floor(i / 3) + 1}/${(i % 3) + 1};${SN_PIPS[n].includes(i) ? '' : 'visibility:hidden'}"></i>`;
  return dots;
}

/* ---- diagnostics hook (browser tests drive the game through it) -------------- */
const SND = {
  /** Called with each dice value; tests override to force rolls. */
  onRoll(v) { snResolveRoll(v); },
  /** Force a specific dice value for the CURRENT roll (test hook).
      Defensive: refuses while another roll is animating — exactly like the
      real Roll button — so a test can never interleave two rolls and
      corrupt engine state. Returns true when the roll was accepted. */
  force(v) {
    const g = SNG.game;
    if (!g || g.winner !== null) return false;
    if (SNG.phase !== 'idle') return false;
    SNG.phase = 'idle'; SND.onRoll(v);
    return true;
  },
  /** Shrink all turn-flow timings so full games finish in seconds (tests). */
  fast() {
    SN_T.rollSpin = 30; SN_T.aiThink = 40;
    SN_T.stepMs = 10; SN_T.jumpMs = 40;
    SN_T.settleMs = 20; SN_T.eventMs = 25;
  },
};

/* ---- teardown -------------------------------------------------------------------- */
function snTeardown() {
  SNG.epoch++;                      // view gone: UI handlers self-invalidate
  SNG.clock++;                      // game clock: pending timers self-kill
  snCancelAnim();                   // cancel any pending animation frame
  snConfettiStop();                 // + any confetti burst
  for (const off of SNG.cleanup) { try { off(); } catch { /* gone */ } }
  SNG.cleanup = [];
  SNG.game = null; SNG.anim = null; SNG.viz = null;
  SNG.phase = 'idle'; SNG.dice = null;
  SNG.ctx = null; SNG.canvas = null;
  snAudioTeardown();          // stop music timer, close the context ONCE
  if (typeof ftExitFullscreenQuiet === 'function') ftExitFullscreenQuiet();
  document.body.classList.remove('sn-fs');
  if (typeof ftSessionRestore === 'function') ftSessionRestore();
}

/* Expose for the console shell (router + tests). Mirrors ludo-game.js. */
window.renderSnView = renderSnView;
window.snTeardown = snTeardown;
window.SNG = SNG;
window.SNCF = SNCF;
window.SND = SND;
window.SN_MODES = SN_MODES;
