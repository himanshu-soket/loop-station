/* ============================================================
   FUN TIME · GAME 5 — CHESS
   Route: #funtime/ch · Namespace: CH / ch* functions / .ch- CSS.
   Rules: the vendored chess.js library (assets/chess.js, exposed as
   the global `Chess`) — legal moves, castling, en passant, promotion,
   check / checkmate / stalemate / draw detection are all delegated to
   it, never hand-rolled.

   AI: a genuine engine (no random-move substitute): negamax with
   alpha-beta pruning, MVV-LVA move ordering, a bounded quiescence
   search, and a material + piece-square-table evaluation. Difficulty
   maps to real search strength (depth cap + time budget + how far
   below the best move it may settle):
     easy   depth 1,  ~160ms, may pick within 90cp of best
     medium depth 2,  ~380ms, may pick within 35cp of best
     hard   depth 3,  ~850ms, always best
     expert depth 4+, ~1500ms, always best (+ deeper via time)

   Lifecycle mirrors the other Fun Time games: one state object, an
   epoch guard that invalidates stale async continuations, every
   listener registered through chOn(), and chTeardown() on any route
   change. CSS is injected once, fully namespaced under .ch-.
   ============================================================ */

/* ---- Injected styles (namespaced; no global selectors touched) ---- */
(function chInjectCss() {
  if (document.getElementById('ch-style')) return;
  const st = document.createElement('style');
  st.id = 'ch-style';
  st.textContent = `
.ch-wide{max-width:1120px}
.ch-setup{padding:18px 22px;border-bottom:1px solid var(--line)}
.ch-setup-grid{display:flex;flex-wrap:wrap;gap:22px}
.ch-field{min-width:190px}
.ch-field>label{display:block;font:700 11px Sora;letter-spacing:.14em;text-transform:uppercase;color:#8b98ad;margin-bottom:8px}
.ch-seg{display:inline-flex;border:1px solid var(--line2);border-radius:10px;overflow:hidden;background:rgba(0,0,0,.28)}
.ch-seg button{appearance:none;border:0;border-right:1px solid var(--line);background:transparent;color:#aab6c8;padding:9px 16px;font:600 13px Sora;cursor:pointer}
.ch-seg button:last-child{border-right:0}
.ch-seg button:hover{color:#fff;background:rgba(94,242,192,.08)}
.ch-seg button.on{background:var(--acc);color:#06251b}
.ch-setup-actions{margin-top:20px;display:flex;gap:12px;align-items:center;flex-wrap:wrap}
.ch-note{font:400 12.5px Sora;color:#8b98ad}
.ch-layout{display:grid;grid-template-columns:minmax(0,1fr) 296px;gap:22px;padding:18px 22px}
@media(max-width:980px){.ch-layout{grid-template-columns:1fr}}
.ch-status{margin:0 22px;padding:12px 16px;border:1px solid var(--line);border-radius:12px;background:rgba(0,0,0,.28);font:600 14px Sora;color:#dbe4f0;display:flex;align-items:center;gap:10px;min-height:46px}
.ch-status .dot{width:9px;height:9px;border-radius:50%;background:var(--acc);box-shadow:0 0 10px var(--acc);flex:none}
.ch-status.warn{border-color:rgba(255,107,129,.5)}.ch-status.warn .dot{background:var(--red);box-shadow:0 0 10px var(--red)}
.ch-status.good{border-color:rgba(94,242,192,.5)}
.ch-think{display:inline-flex;align-items:center;gap:9px}
.ch-think .spin{width:13px;height:13px;border:2px solid rgba(94,242,192,.25);border-top-color:var(--acc);border-radius:50%;animation:chspin .8s linear infinite;flex:none}
@keyframes chspin{to{transform:rotate(360deg)}}
.ch-capbar{display:flex;align-items:center;gap:2px;min-height:28px;font-size:20px;padding:5px 2px;flex-wrap:wrap;line-height:1}
.ch-capbar .lbl{font:700 10px Sora;letter-spacing:.12em;text-transform:uppercase;color:#7f8ca1;margin-right:8px}
.ch-capbar .adv{font:700 12px JetBrains Mono,monospace;color:var(--acc);margin-left:8px}
.ch-capbar .gp{margin-right:-3px}
.ch-capbar .gp.pw{color:#f4f7fc;text-shadow:0 0 8px rgba(94,242,192,.35)}
.ch-capbar .gp.pb{color:#10141d;-webkit-text-stroke:1px #93a7cc;text-shadow:0 0 8px rgba(111,183,255,.3)}
.ch-board-wrap{position:relative;margin:6px 0}
.ch-board{display:grid;grid-template-columns:repeat(8,1fr);grid-template-rows:repeat(8,1fr);width:100%;max-width:min(660px,88vh);aspect-ratio:1;margin:0 auto;border:1px solid var(--line2);border-radius:12px;overflow:hidden;box-shadow:0 20px 55px rgba(0,0,0,.5),0 0 0 1px rgba(94,242,192,.07);touch-action:manipulation;outline:none;user-select:none;-webkit-user-select:none}
.ch-board:focus-visible{outline:2px solid var(--acc);outline-offset:3px}
.ch-sq{position:relative;display:flex;align-items:center;justify-content:center;cursor:pointer;font-size:min(7vw,54px);line-height:1}
.ch-sq.l{background:linear-gradient(145deg,#2b3346,#242c3d)}
.ch-sq.d{background:linear-gradient(145deg,#181f2d,#131927)}
.ch-sq .gp{position:relative;z-index:2;transform:translateY(-2.5%)}
.ch-pw{color:#f4f7fc;text-shadow:0 2px 5px rgba(0,0,0,.6),0 0 16px rgba(94,242,192,.30)}
.ch-pb{color:#0f131c;-webkit-text-stroke:1.3px #93a7cc;text-shadow:0 0 3px rgba(147,167,204,.55),0 0 14px rgba(111,183,255,.32)}
.ch-sq.sel{box-shadow:inset 0 0 0 3px var(--acc)}
.ch-sq.last::after{content:'';position:absolute;inset:0;background:rgba(255,200,87,.17);z-index:1}
.ch-sq.dot::before{content:'';position:absolute;width:26%;height:26%;border-radius:50%;background:rgba(94,242,192,.78);box-shadow:0 0 10px rgba(94,242,192,.65);z-index:2}
.ch-sq.cap::before{content:'';position:absolute;inset:7%;border-radius:50%;border:3px solid rgba(255,107,129,.9);background:rgba(255,107,129,.13);box-shadow:0 0 14px rgba(255,107,129,.5);z-index:2}
.ch-sq.chk{box-shadow:inset 0 0 0 3px var(--red),inset 0 0 24px rgba(255,107,129,.5)}
.ch-sq.cur{outline:2px dashed rgba(111,183,255,.85);outline-offset:-3px;z-index:3}
.ch-coord{position:absolute;font:700 10px JetBrains Mono,monospace;opacity:.55;z-index:3;pointer-events:none}
.ch-coord.f{right:4px;bottom:2px}.ch-coord.r{left:4px;top:2px}
.ch-ovl{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;background:rgba(7,9,13,.8);backdrop-filter:blur(5px);z-index:10;border-radius:12px}
.ch-ovl[hidden]{display:none}
.ch-ovl-box{display:flex;flex-direction:column;gap:14px;align-items:center;background:var(--panel);border:1px solid var(--line2);padding:22px 26px;border-radius:16px;box-shadow:0 24px 70px rgba(0,0,0,.65);text-align:center}
.ch-ovl-title{font:800 21px Sora;letter-spacing:-.01em}
.ch-ovl-sub{font:500 13.5px Sora;color:#9aa7ba;margin-top:-6px}
.ch-promo-row{display:flex;gap:12px}
.ch-promo-row button{appearance:none;cursor:pointer;border:1px solid var(--line2);background:rgba(0,0,0,.3);border-radius:12px;width:64px;height:64px;display:flex;align-items:center;justify-content:center;font-size:42px;line-height:1;transition:border-color .15s,transform .15s}
.ch-promo-row button:hover{border-color:var(--acc);transform:translateY(-2px)}
.ch-side{display:flex;flex-direction:column;gap:14px;min-width:0}
.ch-card{border:1px solid var(--line);border-radius:12px;background:rgba(0,0,0,.26);padding:12px 14px}
.ch-card h4{margin:0 0 10px;font:700 11px Sora;letter-spacing:.13em;text-transform:uppercase;color:#8b98ad}
.ch-evalrow{display:flex;align-items:center;gap:10px}
.ch-evalbar{flex:1;height:10px;border-radius:6px;background:#0d1119;border:1px solid var(--line);overflow:hidden}
.ch-evalbar>i{display:block;height:100%;width:50%;background:linear-gradient(90deg,var(--acc),#9ffde0);transition:width .35s ease}
.ch-evaltxt{font:700 12px JetBrains Mono,monospace;color:#dbe4f0;min-width:52px;text-align:right}
.ch-hist{max-height:264px;overflow:auto;font:500 12.5px/2 JetBrains Mono,monospace;color:#aeb9ca}
.ch-hist .no{color:#67748a;margin-right:6px}
.ch-hist .mv{cursor:default;padding:1px 5px;border-radius:5px}
.ch-hist .mv.last{background:rgba(94,242,192,.14);color:var(--acc)}
.ch-hist .empty{color:#67748a;font-family:Sora}
.ch-diffbar{display:flex;gap:6px;flex-wrap:wrap}
.ch-diffbar button{appearance:none;cursor:pointer;border:1px solid var(--line2);background:rgba(0,0,0,.25);color:#aab6c8;border-radius:8px;padding:6px 11px;font:600 12px Sora}
.ch-diffbar button:hover{color:#fff;border-color:var(--acc-dim)}
.ch-diffbar button.on{background:var(--acc);border-color:var(--acc);color:#06251b}
.ch-meta{font:500 12px Sora;color:#8b98ad;display:flex;flex-direction:column;gap:5px}
.ch-meta b{color:#cfd9e6;font-weight:700}
/* ---- Fullscreen / immersive layout (Chess-only, fully namespaced) -----
   Three triggers activate the SAME layout:
     #chPanel:fullscreen — native fullscreen scoped to the panel
     html:fullscreen body.ch-fs-doc — native fullscreen on <html> (card-click
                          launch made before the panel existed)
     body.ch-fs          — CSS immersive fallback when native is denied
   The panel becomes a viewport-filling app shell: compact header with live
   status, compact config row, board as the centerpiece sized from the
   viewport (never clipped, always square), sidebar beside it, controls
   under the board. Nothing floats far below; no dead space. */

/* While Chess is playing (any mode), hide ALL dashboard chrome so ONLY the
   Chess interface is visible: no sidebar, no topbar, no game cards behind.
   This applies to native fullscreen, immersive fallback, and inline play. */
body.ch-playing .sidebar,
body.ch-playing .topbar{display:none !important}
body.ch-playing .main{padding:0 !important;margin:0 !important}
body.ch-playing .content{padding:0 !important}
body.ch-playing .content-inner{max-width:none !important;padding:0 !important;margin:0 !important}

#chPanel:fullscreen,
html:fullscreen body.ch-fs-doc #chPanel,
body.ch-fs #chPanel{
  position:fixed;inset:0;z-index:200;
  width:100vw;height:100vh;max-width:none;
  margin:0;border:0;border-radius:0;
  background:var(--bg);
  padding:0;gap:0;overflow:hidden;
}
/* App shell grid: header / main / (controls live inside main) */
#chPanel:fullscreen .ft-panel-h,
html:fullscreen body.ch-fs-doc #chPanel .ft-panel-h,
body.ch-fs #chPanel .ft-panel-h{display:none}        /* replaced by ch-fsbar */
#chPanel:fullscreen .ft-game-top,
html:fullscreen body.ch-fs-doc #chPanel .ft-game-top,
body.ch-fs #chPanel .ft-game-top{display:none}       /* dashboard chrome gone */
/* Inline (non-FS) play: the view's own back-link header stays, but the
   dashboard sidebar/topbar are hidden via body.ch-playing above; make the
   chess panel fill the main column. */
body.ch-playing #chPanel{
  margin:0 auto;width:100%;
  max-width:min(1180px,100%);
}
/* PHASE 1 · phones (inline play): the setup block + status strip pushed
   the board ~584px down a 844px viewport and the action row 700px below
   the fold. On narrow portrait screens the panel compacts (header folds,
   setup wraps tightly, board first) and the page scrolls naturally in
   .content for the sidebar cards. */
@media (max-width:980px){
  body.ch-playing #chPanel{padding:12px 12px 14px;gap:10px}
  body.ch-playing #chPanel .ft-panel-h{display:none}     /* title duplicates the top bar */
  .ch-setup{padding:10px 12px}
  .ch-setup-grid{gap:10px}
  .ch-field{min-width:0}
  .ch-field>label{margin-bottom:4px;font-size:9.5px}
  .ch-seg button{padding:6px 10px;font-size:11.5px}
  .ch-diffbar button{padding:5px 9px;font-size:11px}
  .ch-setup-actions{margin-top:8px;gap:8px}
  .ch-note{display:none}
  .ch-status{margin:0 12px;padding:8px 12px;font-size:12.5px;min-height:38px}
  .ch-layout{padding:10px 12px;gap:10px}
  .ch-board{max-width:min(100%,calc(100vh - 320px))}
  .ch-capbar{font-size:15px;min-height:22px}
}
@media (max-width:560px){
  body.ch-playing #chPanel{padding:8px 8px 10px;gap:8px}
  .ch-setup{padding:8px 8px}
  .ch-setup-grid{gap:8px}
  .ch-seg button{padding:6px 9px;font-size:11px}
  .ch-status{margin:0 8px;font-size:12px}
  .ch-layout{padding:6px 8px;gap:8px}
  .ch-board{width:min(calc(100vw - 20px),calc(100vh - 300px));max-width:100%;font-size:0}
  .ch-sq{font-size:min(9vw,34px)}
  .ft-actions{gap:6px}
  .ft-actions .ft-btn{padding:7px 10px;font-size:11px}
}
/* Landscape phones (inline): keep every fixed row ultra-compact so the
   64-square board stays as large as the short viewport allows. */
@media (max-height:520px) and (orientation:landscape){
  body.ch-playing #chPanel{padding:6px 10px 8px;gap:6px}
  body.ch-playing #chPanel .ft-panel-h{display:none}
  .ch-setup{padding:4px 10px}
  .ch-setup-grid{gap:8px}
  .ch-field>label{display:none}
  .ch-seg button{padding:4px 8px;font-size:10.5px}
  .ch-diffbar button{padding:3px 7px;font-size:10px}
  .ch-setup-actions{margin-top:4px}
  .ch-status{margin:0 10px;padding:4px 10px;font-size:11.5px;min-height:28px}
  .ch-layout{padding:4px 10px;gap:8px}
  .ch-board{width:min(calc(100vh - 210px),calc(100vw - 24px));max-width:100%}
  .ch-sq{font-size:clamp(13px,5vh,30px)}
  .ch-capbar{font-size:12px;min-height:16px}
  .ft-actions{gap:5px}
  .ft-actions .ft-btn{padding:5px 9px;font-size:10px}
}
#chPanel:fullscreen .content,
html:fullscreen body.ch-fs-doc #chPanel .content,
body.ch-fs #chPanel .content,
#chPanel:fullscreen .content-inner,
html:fullscreen body.ch-fs-doc #chPanel .content-inner,
body.ch-fs #chPanel .content-inner{padding:0;margin:0;max-width:none;display:contents}

/* Compact header: title + live status on one line */
.ch-fsbar{
  display:none;
}
#chPanel:fullscreen .ch-fsbar,
html:fullscreen body.ch-fs-doc #chPanel .ch-fsbar,
body.ch-fs #chPanel .ch-fsbar{
  display:flex;align-items:center;gap:16px;flex-wrap:wrap;
  padding:10px 18px;min-height:52px;
  border-bottom:1px solid var(--line);
  background:rgba(15,19,28,.92);
}
.ch-fsbar .ch-fs-title{font:800 17px Sora;letter-spacing:-.01em;color:#f4f7fc;white-space:nowrap}
.ch-fsbar .ch-fs-status{
  flex:1;min-width:180px;display:flex;align-items:center;gap:9px;
  font:600 13px Sora;color:#dbe4f0;
  border:1px solid var(--line);border-radius:9px;
  padding:7px 12px;background:rgba(0,0,0,.3);
}
.ch-fsbar .ch-fs-status .dot{width:8px;height:8px;border-radius:50%;background:var(--acc);box-shadow:0 0 9px var(--acc);flex:none}
.ch-fsbar .ch-fs-status.warn .dot{background:var(--red);box-shadow:0 0 9px var(--red)}
.ch-fsbar .ch-fs-status .spin{width:12px;height:12px;border:2px solid rgba(94,242,192,.25);border-top-color:var(--acc);border-radius:50%;animation:chspin .8s linear infinite;flex:none}
.ch-fsbar .ch-fs-info{font:600 11px JetBrains Mono,monospace;color:#8b98ad;white-space:nowrap}

/* Compact config row */
#chPanel:fullscreen .ch-setup,
html:fullscreen body.ch-fs-doc #chPanel .ch-setup,
body.ch-fs #chPanel .ch-setup{
  padding:8px 18px;border-bottom:1px solid var(--line);background:rgba(10,13,20,.6);
}
#chPanel:fullscreen .ch-setup-grid,
html:fullscreen body.ch-fs-doc #chPanel .ch-setup-grid,
body.ch-fs #chPanel .ch-setup-grid{gap:14px;flex-wrap:wrap}
#chPanel:fullscreen .ch-field,
html:fullscreen body.ch-fs-doc #chPanel .ch-field,
body.ch-fs #chPanel .ch-field{min-width:0}
#chPanel:fullscreen .ch-field>label,
html:fullscreen body.ch-fs-doc #chPanel .ch-field>label,
body.ch-fs #chPanel .ch-field>label{margin-bottom:5px;font-size:9.5px}
#chPanel:fullscreen .ch-seg button,
html:fullscreen body.ch-fs-doc #chPanel .ch-seg button,
body.ch-fs #chPanel .ch-seg button{padding:6px 12px;font-size:12px}
#chPanel:fullscreen .ch-diffbar button,
html:fullscreen body.ch-fs-doc #chPanel .ch-diffbar button,
body.ch-fs #chPanel .ch-diffbar button{padding:5px 10px;font-size:11.5px}
#chPanel:fullscreen .ch-setup-actions,
html:fullscreen body.ch-fs-doc #chPanel .ch-setup-actions,
body.ch-fs #chPanel .ch-setup-actions{margin-top:8px}
#chPanel:fullscreen .ch-setup-actions .ch-note,
html:fullscreen body.ch-fs-doc #chPanel .ch-setup-actions .ch-note,
body.ch-fs #chPanel .ch-setup-actions .ch-note{display:none}   /* hint not needed in FS */

/* Standalone status strip (inline view keeps it; FS hides it — header has it) */
#chPanel:fullscreen .ch-status:not(.ch-fs-status),
html:fullscreen body.ch-fs-doc #chPanel .ch-status:not(.ch-fs-status),
body.ch-fs #chPanel .ch-status:not(.ch-fs-status){display:none}

/* Main area: board centerpiece + sidebar, fills remaining height */
#chPanel:fullscreen .ch-layout,
html:fullscreen body.ch-fs-doc #chPanel .ch-layout,
body.ch-fs #chPanel .ch-layout{
  flex:1;min-height:0;
  display:grid;
  grid-template-columns:minmax(0,1fr) minmax(240px,300px);
  gap:18px;padding:14px 18px;
  max-width:none;margin:0;width:auto;   /* stretch: never margin-auto shrink */
  align-content:center;
}
/* Board column: cap bars above/below, board centered & sized from viewport.
   Height budget: 100vh - header(~52) - config(~86) - controls(~64) - paddings.
   The min() keeps it square and clip-free at every tested resolution. */
#chPanel:fullscreen .ch-main,
html:fullscreen body.ch-fs-doc #chPanel .ch-main,
body.ch-fs #chPanel .ch-main{
  display:flex;flex-direction:column;align-items:center;justify-content:center;
  min-height:0;min-width:0;gap:6px;
}
#chPanel:fullscreen .ch-capbar,
html:fullscreen body.ch-fs-doc #chPanel .ch-capbar,
body.ch-fs #chPanel .ch-capbar{min-height:24px;font-size:clamp(15px,1.6vh,21px);padding:2px 2px;align-self:center;max-width:min(72vh,86vw)}
#chPanel:fullscreen .ch-board-wrap,
html:fullscreen body.ch-fs-doc #chPanel .ch-board-wrap,
body.ch-fs #chPanel .ch-board-wrap{margin:0;display:flex;align-items:center;justify-content:center;min-height:0}
#chPanel:fullscreen .ch-board,
html:fullscreen body.ch-fs-doc #chPanel .ch-board,
body.ch-fs #chPanel .ch-board{
  width:min(
    calc(100vh - 330px),   /* vertical budget: header+config+controls+cap bars */
    calc(100vw - 380px),   /* horizontal budget beside sidebar */
    900px                  /* readability cap */
  );
  max-width:100%;
  aspect-ratio:1;margin:0;
}
#chPanel:fullscreen .ch-sq,
html:fullscreen body.ch-fs-doc #chPanel .ch-sq,
body.ch-fs #chPanel .ch-sq{font-size:clamp(18px,5.4vh,58px)}

/* Sidebar scrolls internally; never pushes layout */
#chPanel:fullscreen .ch-side,
html:fullscreen body.ch-fs-doc #chPanel .ch-side,
body.ch-fs #chPanel .ch-side{
  min-height:0;overflow-y:auto;gap:10px;padding-right:2px;
  scrollbar-width:thin;
}
#chPanel:fullscreen .ch-card,
html:fullscreen body.ch-fs-doc #chPanel .ch-card,
body.ch-fs #chPanel .ch-card{padding:10px 12px}
#chPanel:fullscreen .ch-hist,
html:fullscreen body.ch-fs-doc #chPanel .ch-hist,
body.ch-fs #chPanel .ch-hist{max-height:none;flex:1;min-height:80px}
#chPanel:fullscreen .ch-side .ch-card:nth-child(2),
html:fullscreen body.ch-fs-doc #chPanel .ch-side .ch-card:nth-child(2),
body.ch-fs #chPanel .ch-side .ch-card:nth-child(2){display:flex;flex-direction:column;min-height:0;flex:1}

/* Controls: directly under the board, never far below */
#chPanel:fullscreen .ft-actions,
html:fullscreen body.ch-fs-doc #chPanel .ft-actions,
body.ch-fs #chPanel .ft-actions{
  padding:8px 18px 12px;border-top:1px solid var(--line);
  background:rgba(10,13,20,.6);justify-content:center;margin:0;
}
#chPanel:fullscreen .ft-btn,
html:fullscreen body.ch-fs-doc #chPanel .ft-btn,
body.ch-fs #chPanel .ft-btn{padding:8px 14px;font-size:12.5px}

/* Launch row hidden in FS (we're already there); retry shown only when
   native FS is unavailable AND we're in immersive fallback. */
#chPanel:fullscreen #chOvLaunch,
html:fullscreen body.ch-fs-doc #chPanel #chOvLaunch,
body.ch-fs #chPanel #chOvLaunch{display:none}
.ch-fs-retry{
  display:none;appearance:none;cursor:pointer;
  align-items:center;gap:7px;
  border:1px dashed rgba(94,242,192,.45);border-radius:9px;
  background:rgba(94,242,192,.06);color:var(--acc);
  font:700 12px Sora;padding:7px 13px;
}
body.ch-fs.immersive #chFsBtn{display:none}          /* native unavailable: hide toggle */
body.ch-fs.immersive .ch-fs-retry{display:inline-flex}

/* Responsive tuning for the tested matrix */
@media(max-width:1180px){
  /* tighter sidebar on 1024-1180 widths */
  #chPanel:fullscreen .ch-layout,
  html:fullscreen body.ch-fs-doc #chPanel .ch-layout,
  body.ch-fs #chPanel .ch-layout{grid-template-columns:minmax(0,1fr) minmax(210px,250px);gap:12px;padding:10px 12px}
  #chPanel:fullscreen .ch-board,
  html:fullscreen body.ch-fs-doc #chPanel .ch-board,
  body.ch-fs #chPanel .ch-board{width:min(calc(100vh - 330px),calc(100vw - 310px),820px)}
}
@media(max-width:980px){
  /* portrait tablets / narrow windows: sidebar moves BELOW the board */
  #chPanel:fullscreen .ch-layout,
  html:fullscreen body.ch-fs-doc #chPanel .ch-layout,
  body.ch-fs #chPanel .ch-layout{grid-template-columns:minmax(0,1fr);overflow-y:auto;align-content:start;grid-auto-rows:min-content}
  #chPanel:fullscreen .ch-main,
  html:fullscreen body.ch-fs-doc #chPanel .ch-main,
  body.ch-fs #chPanel .ch-main{order:0;justify-content:flex-start;flex:none}
  #chPanel:fullscreen .ch-side,
  html:fullscreen body.ch-fs-doc #chPanel .ch-side,
  body.ch-fs #chPanel .ch-side{order:1;flex-direction:row;flex-wrap:wrap;overflow:visible}
  #chPanel:fullscreen .ch-side .ch-card,
  html:fullscreen body.ch-fs-doc #chPanel .ch-side .ch-card,
  body.ch-fs #chPanel .ch-side .ch-card{flex:1 1 220px}
  #chPanel:fullscreen .ch-side .ch-card:nth-child(2),
  html:fullscreen body.ch-fs-doc #chPanel .ch-side .ch-card:nth-child(2),
  body.ch-fs #chPanel .ch-side .ch-card:nth-child(2){flex:1 1 100%}
  #chPanel:fullscreen .ch-hist,
  html:fullscreen body.ch-fs-doc #chPanel .ch-hist,
  body.ch-fs #chPanel .ch-hist{max-height:140px}
  #chPanel:fullscreen .ch-board,
  html:fullscreen body.ch-fs-doc #chPanel .ch-board,
  body.ch-fs #chPanel .ch-board{width:min(calc(100vw - 32px),calc(100vh - 360px),720px)}
}
@media(max-width:560px){
  /* phones (390×844): single column, compact everything */
  #chPanel:fullscreen .ch-fsbar,
  html:fullscreen body.ch-fs-doc #chPanel .ch-fsbar,
  body.ch-fs #chPanel .ch-fsbar{padding:8px 10px;gap:8px}
  #chPanel:fullscreen .ch-fsbar .ch-fs-info,
  html:fullscreen body.ch-fs-doc #chPanel .ch-fsbar .ch-fs-info,
  body.ch-fs #chPanel .ch-fsbar .ch-fs-info{display:none}
  #chPanel:fullscreen .ch-setup,
  html:fullscreen body.ch-fs-doc #chPanel .ch-setup,
  body.ch-fs #chPanel .ch-setup{padding:6px 10px}
  #chPanel:fullscreen .ch-layout,
  html:fullscreen body.ch-fs-doc #chPanel .ch-layout,
  body.ch-fs #chPanel .ch-layout{padding:8px 10px;gap:8px}
  #chPanel:fullscreen .ch-board,
  html:fullscreen body.ch-fs-doc #chPanel .ch-board,
  body.ch-fs #chPanel .ch-board{width:min(calc(100vw - 20px),calc(100vh - 400px),560px)}
  #chPanel:fullscreen .ch-sq,
  html:fullscreen body.ch-fs-doc #chPanel .ch-sq,
  body.ch-fs #chPanel .ch-sq{font-size:clamp(15px,9.5vw,34px)}
  #chPanel:fullscreen .ft-actions,
  html:fullscreen body.ch-fs-doc #chPanel .ft-actions,
  body.ch-fs #chPanel .ft-actions{padding:6px 10px 10px;gap:7px}
  #chPanel:fullscreen .ft-btn,
  html:fullscreen body.ch-fs-doc #chPanel .ft-btn,
  body.ch-fs #chPanel .ft-btn{padding:7px 10px;font-size:11.5px}
}
/* Landscape phones (e.g. 844×390): ultra-compact, sidebar hidden behind scroll */
@media(max-height:520px) and (orientation:landscape){
  #chPanel:fullscreen .ch-fsbar,
  html:fullscreen body.ch-fs-doc #chPanel .ch-fsbar,
  body.ch-fs #chPanel .ch-fsbar{min-height:40px;padding:5px 12px}
  #chPanel:fullscreen .ch-setup,
  html:fullscreen body.ch-fs-doc #chPanel .ch-setup,
  body.ch-fs #chPanel .ch-setup{padding:4px 12px}
  #chPanel:fullscreen .ch-layout,
  html:fullscreen body.ch-fs-doc #chPanel .ch-layout,
  body.ch-fs #chPanel .ch-layout{padding:6px 12px;gap:10px}
  /* PHASE 1 FIX · the old calc(100vh - 230px) starved the board to ~160px
     on a 390px-tall phone once the fsbar + setup rows were counted. The
     budget now reflects the compacted chrome and lets the layout's own
     scroll handle the sidebar below. */
  #chPanel:fullscreen .ch-board,
  html:fullscreen body.ch-fs-doc #chPanel .ch-board,
  body.ch-fs #chPanel .ch-board{width:min(calc(100vh - 176px),calc(100vw - 300px),700px)}
  #chPanel:fullscreen .ch-capbar,
  html:fullscreen body.ch-fs-doc #chPanel .ch-capbar,
  body.ch-fs #chPanel .ch-capbar{min-height:18px;font-size:13px}
}
.ft-card-art.ch{background:radial-gradient(120% 120% at 20% 0%,#1d2a3f 0%,#101724 55%,#0b101b 100%)}
.ch-card-knight{display:block;font-size:clamp(64px,11vw,104px);line-height:1;color:#f4f7fc;text-shadow:0 0 26px rgba(94,242,192,.5),0 4px 14px rgba(0,0,0,.6);transform:translateY(2%)}
.ch-card-strip{position:absolute;left:0;right:0;bottom:0;height:16px;display:grid;grid-template-columns:repeat(8,1fr);opacity:.85}
.ch-card-strip i{height:100%}
.ch-card-strip i:nth-child(odd){background:#242c3d}.ch-card-strip i:nth-child(even){background:#141a28}
`;
  document.head.appendChild(st);
})();

/* ---- Constants ------------------------------------------------------- */
const CH_MATE = 100000;
const CH_VAL = { p: 100, n: 320, b: 330, r: 500, q: 900, k: 20000 };
/* Unicode glyphs (filled set for both colours; colour comes from CSS). */
const CH_GLYPH = { k: '\u265A', q: '\u265B', r: '\u265C', b: '\u265D', n: '\u265E', p: '\u265F' };
const CH_FILES = 'abcdefgh';
/* Piece-square tables, white-oriented, index 0 = a8 (matches chess.js
   board()[row][col] with row 0 = rank 8). Black mirrors the rank. */
const CH_PST = {
  p: [0,0,0,0,0,0,0,0, 100,170,210,250,250,210,170,100, 50,90,110,130,130,110,90,50,
      30,50,70,80,80,70,50,30, 20,30,50,60,60,50,30,20, 10,15,20,40,40,20,15,10,
      5,10,10,25,25,10,10,5, 0,0,0,0,0,0,0,0],
  n: [-50,-40,-30,-30,-30,-30,-40,-50, -40,-20,0,0,0,0,-20,-40, -30,0,10,15,15,10,0,-30,
      -30,5,15,20,20,15,5,-30, -30,0,15,20,20,15,0,-30, -30,5,10,15,15,10,5,-30,
      -40,-20,0,0,0,0,-20,-40, -50,-40,-30,-30,-30,-30,-40,-50],
  b: [-20,-10,-10,-10,-10,-10,-10,-20, -10,0,0,0,0,0,0,-10, -10,0,5,10,10,5,0,-10,
      -10,5,5,10,10,5,5,-10, -10,0,10,10,10,10,0,-10, -10,10,10,10,10,10,10,-10,
      -10,5,0,0,0,0,5,-10, -20,-10,-10,-10,-10,-10,-10,-20],
  r: [0,0,0,0,0,0,0,0, 5,10,10,10,10,10,10,5, -5,0,0,0,0,0,0,-5, -5,0,0,0,0,0,0,-5,
      -5,0,0,0,0,0,0,-5, -5,0,0,0,0,0,0,-5, -5,0,0,0,0,0,0,-5, 0,0,0,5,5,0,0,0],
  q: [-20,-10,-10,-5,-5,-10,-10,-20, -10,0,0,0,0,0,0,-10, -10,0,5,5,5,5,0,-10,
      -5,0,5,5,5,5,0,-5, 0,0,5,5,5,5,0,-5, -10,5,5,5,5,5,0,-10,
      -10,0,5,0,0,0,0,-10, -20,-10,-10,-5,-5,-10,-10,-20],
  k: [-30,-40,-40,-50,-50,-40,-40,-30, -30,-40,-40,-50,-50,-40,-40,-30,
      -30,-40,-40,-50,-50,-40,-40,-30, -30,-40,-40,-50,-50,-40,-40,-30,
      -20,-30,-30,-40,-40,-30,-30,-20, -10,-20,-20,-20,-20,-20,-20,-10,
      20,20,0,0,0,0,20,20, 20,30,10,0,0,10,30,20],
  ke: [-50,-40,-30,-20,-20,-30,-40,-50, -30,-20,-10,0,0,-10,-20,-30,
       -30,-10,20,30,30,20,-10,-30, -30,-10,30,40,40,30,-10,-30,
       -30,-10,30,40,40,30,-10,-30, -30,-10,20,30,30,20,-10,-30,
       -30,-30,0,0,0,0,-30,-30, -50,-30,-30,-30,-30,-30,-30,-50],
};
/* Real search strength per difficulty — never a random-move substitute. */
const CH_DIFF = {
  easy:   { label: 'Easy',   maxDepth: 1, timeMs: 160,  noiseCp: 90 },
  medium: { label: 'Medium', maxDepth: 2, timeMs: 380,  noiseCp: 35 },
  hard:   { label: 'Hard',   maxDepth: 3, timeMs: 850,  noiseCp: 0  },
  expert: { label: 'Expert', maxDepth: 4, timeMs: 1500, noiseCp: 0  },
};

/* ---- Engine: evaluation ----------------------------------------------
   Material + piece-square tables, returned in centipawns from WHITE's
   perspective (positive = white is better). */
function chEvalWhite(game) {
  const b = game.board();
  let score = 0, mat = 0;
  for (let r = 0; r < 8; r++) {
    const row = b[r];
    for (let c = 0; c < 8; c++) {
      const p = row[c];
      if (!p) continue;
      if (p.type !== 'k') mat += CH_VAL[p.type] * (p.color === 'w' ? 1 : -1);
    }
  }
  /* Endgame detection: few heavy pieces left -> centralise the king. */
  const endgame = Math.abs(mat) <= 1300 && !hasQueen(b);
  const kt = endgame ? 'ke' : 'k';
  for (let r = 0; r < 8; r++) {
    const row = b[r];
    for (let c = 0; c < 8; c++) {
      const p = row[c];
      if (!p) continue;
      const t = p.type === 'k' ? kt : p.type;
      const tbl = CH_PST[t];
      if (p.color === 'w') score += CH_VAL[p.type] + tbl[r * 8 + c];
      else score -= CH_VAL[p.type] + tbl[(7 - r) * 8 + c];
    }
  }
  return score;
}
function hasQueen(board) {
  for (let r = 0; r < 8; r++) for (let c = 0; c < 8; c++) {
    const p = board[r][c];
    if (p && p.type === 'q') return true;
  }
  return false;
}

/* MVV-LVA move ordering: promotions, then big captures by cheap attackers. */
function chOrderMoves(moves) {
  const scored = [];
  for (let i = 0; i < moves.length; i++) {
    const m = moves[i];
    let s = 0;
    if (m.promotion) s += 9000 + (CH_VAL[m.promotion] || 0);
    if (m.captured) s += 10 * CH_VAL[m.captured] - CH_VAL[m.piece];
    if (m.san && m.san.indexOf('+') >= 0) s += 60;
    scored.push([s, m]);
  }
  scored.sort((a, b) => b[0] - a[0]);
  const out = new Array(scored.length);
  for (let i = 0; i < scored.length; i++) out[i] = scored[i][1];
  return out;
}

/* Search context (single search at a time — main thread). */
const CH_SEARCH = { deadline: 0, nodes: 0, abort: false };

/* Quiescence: resolve captures/promotions so the horizon never ends on a
   dangling tactical shot. Stand-pat prunes quiet positions immediately. */
function chQuiesce(game, alpha, beta, ply) {
  CH_SEARCH.nodes++;
  if ((CH_SEARCH.nodes & 1023) === 0 && Date.now() > CH_SEARCH.deadline) CH_SEARCH.abort = true;
  if (CH_SEARCH.abort) return alpha;
  const turnW = game.turn() === 'w';
  const stand = turnW ? chEvalWhite(game) : -chEvalWhite(game);
  if (stand >= beta) return stand;
  if (stand > alpha) alpha = stand;
  if (ply >= 6) return alpha;
  const caps = [];
  const all = game.moves({ verbose: true });
  for (let i = 0; i < all.length; i++) {
    const m = all[i];
    if (m.captured || m.promotion) caps.push(m);
  }
  const ordered = chOrderMoves(caps);
  for (let i = 0; i < ordered.length; i++) {
    game.move(ordered[i]);
    const score = -chQuiesce(game, -beta, -alpha, ply + 1);
    game.undo();
    if (CH_SEARCH.abort) return alpha;
    if (score >= beta) return score;
    if (score > alpha) alpha = score;
  }
  return alpha;
}

/* Negamax with alpha-beta. Terminal scores prefer faster mates. */
function chNegamax(game, depth, alpha, beta, ply) {
  CH_SEARCH.nodes++;
  if ((CH_SEARCH.nodes & 1023) === 0 && Date.now() > CH_SEARCH.deadline) CH_SEARCH.abort = true;
  if (CH_SEARCH.abort) return alpha;
  if (game.isCheckmate()) return ply - CH_MATE;          // side to move is mated
  if (game.isDraw() || game.isStalemate()) return 0;
  if (depth <= 0) return chQuiesce(game, alpha, beta, 0);
  const moves = chOrderMoves(game.moves({ verbose: true }));
  let best = -Infinity;
  for (let i = 0; i < moves.length; i++) {
    game.move(moves[i]);
    const score = -chNegamax(game, depth - 1, -beta, -alpha, ply + 1);
    game.undo();
    if (CH_SEARCH.abort) return best === -Infinity ? alpha : best;
    if (score > best) best = score;
    if (score > alpha) alpha = score;
    if (alpha >= beta) break;
  }
  return best;
}

/* Root search: iterative deepening under a hard time budget. Returns
   { from, to, promotion, san, score, depth, nodes } or null. Every
   difficulty runs this same real engine — only strength knobs change. */
function chFindBestMove(fen, diffKey) {
  const cfg = CH_DIFF[diffKey] || CH_DIFF.medium;
  const game = new Chess(fen);
  const legal = game.moves({ verbose: true });
  if (!legal.length) return null;
  if (legal.length === 1) {
    const m = legal[0];
    return { from: m.from, to: m.to, promotion: m.promotion || undefined,
             san: m.san, score: 0, depth: 0, nodes: 1 };
  }
  CH_SEARCH.deadline = Date.now() + cfg.timeMs;
  CH_SEARCH.nodes = 0;
  CH_SEARCH.abort = false;
  let rootScores = legal.map(m => ({ m, score: -Infinity }));
  let completedDepth = 0;
  /* When a difficulty may settle for a near-best move, every root score
     must be EXACT — alpha-propagated windows only bound siblings. Those
     levels are shallow (depth<=2), so full windows stay cheap. Hard and
     expert (noise 0) keep the fast narrowing window: they only need the
     arg-max, which a bound can never corrupt. */
  const exactRoot = cfg.noiseCp > 0;
  for (let d = 1; d <= cfg.maxDepth; d++) {
    CH_SEARCH.abort = false;
    let alpha = -Infinity;
    const results = [];
    let finished = true;
    /* Search previous-best first: sharpens alpha-beta ordering. */
    rootScores.sort((a, b) => b.score - a.score);
    for (let i = 0; i < rootScores.length; i++) {
      const { m } = rootScores[i];
      game.move(m);
      const lo = exactRoot ? -Infinity : -Infinity;
      const hi = exactRoot ? Infinity : -alpha;
      const score = -chNegamax(game, d - 1, lo, hi, 1);
      game.undo();
      if (CH_SEARCH.abort) { finished = false; break; }
      results.push({ m, score });
      if (score > alpha) alpha = score;
    }
    if (finished && results.length === rootScores.length) {
      rootScores = results;
      completedDepth = d;
    } else break;
    if (Date.now() > CH_SEARCH.deadline) break;
  }
  rootScores.sort((a, b) => b.score - a.score);
  let pick = rootScores[0];
  /* Easy/Medium may settle for a near-best move (still engine-evaluated,
     never a blind random pick); Hard/Expert always play the best found.
     A found mate is never diluted — every level finishes a mate in hand. */
  const noise = cfg.noiseCp;
  const isMateScore = Math.abs(pick.score) > CH_MATE - 1000;
  if (noise > 0 && !isMateScore && rootScores.length > 1) {
    const window = pick.score - noise;
    const pool = rootScores.filter(x => x.score >= window);
    pick = pool[Math.floor(Math.random() * pool.length)];
  }
  const m = pick.m;
  return { from: m.from, to: m.to, promotion: m.promotion || undefined,
           san: m.san, score: pick.score, depth: completedDepth,
           nodes: CH_SEARCH.nodes };
}

/* ---- Audio (own context; separate from the other games) -------------- */
const CHA = { ctx: null, master: null, on: false };
function chAudioInit() {
  if (CHA.ctx) return true;
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return false;
  try {
    CHA.ctx = new AC();
    CHA.master = CHA.ctx.createGain();
    CHA.master.gain.value = 0.3;
    CHA.master.connect(CHA.ctx.destination);
    return true;
  } catch { CHA.ctx = null; return false; }
}
function chBeep(freq, dur, vol, type) {
  if (!CHA.on || !CHA.ctx || !freq) return;
  const t = CHA.ctx.currentTime;
  const osc = CHA.ctx.createOscillator();
  const gain = CHA.ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(vol, t + 0.008);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(CHA.master);
  osc.start(t); osc.stop(t + dur + 0.02);
}
function chSfx(kind) {
  switch (kind) {
    case 'move':  chBeep(520, 0.055, 0.12, 'triangle'); break;
    case 'cap':   chBeep(300, 0.09, 0.16, 'square');
                  setTimeout(() => chBeep(190, 0.07, 0.10, 'square'), 45); break;
    case 'check': [740, 990].forEach((f, i) =>
                    setTimeout(() => chBeep(f, 0.08, 0.13, 'sawtooth'), i * 90)); break;
    case 'win':   [523, 659, 784, 1047].forEach((f, i) =>
                    setTimeout(() => chBeep(f, 0.14, 0.14), i * 110)); break;
    case 'lose':  [392, 311, 233].forEach((f, i) =>
                    setTimeout(() => chBeep(f, 0.18, 0.14), i * 140)); break;
    case 'draw':  [440, 440].forEach((f, i) =>
                    setTimeout(() => chBeep(f, 0.12, 0.11), i * 160)); break;
    case 'ui':    chBeep(880, 0.04, 0.08); break;
    case 'err':   chBeep(180, 0.09, 0.10, 'square'); break;
  }
}
function chAudioDispose() {
  if (CHA.ctx) { try { CHA.ctx.close(); } catch { /* already closed */ } }
  CHA.ctx = null; CHA.master = null;
}
function chSoundToggle() {
  CHA.on = !CHA.on;
  if (CHA.on && !chAudioInit()) CHA.on = false;
  else if (CHA.on && CHA.ctx.state === 'suspended') CHA.ctx.resume().catch(() => {});
  chSoundButton();
}
function chSoundButton() {
  const btn = document.getElementById('chSoundBtn');
  if (!btn) return;
  btn.setAttribute('aria-pressed', String(CHA.on));
  btn.classList.toggle('on', CHA.on);
  const lbl = document.getElementById('chSoundLbl');
  if (lbl) lbl.textContent = CHA.on ? 'sound on' : 'sound off';
}

/* ---- Fullscreen lifecycle (Chess-only; deliberately NOT the shared FTFS) --
   Launch flow: the Fun Time card's click handler calls chTryLaunchFullscreen()
   INSIDE the user gesture (browsers require it), then renderChView() mounts.
   Outcomes:
     • native fullscreen granted  -> #chPanel:fullscreen layout
     • native denied/unsupported  -> CSS immersive fallback (body.ch-fs.immersive),
       honestly labelled: we never claim native when only immersive is active.
   The user's explicit exit (Esc / Exit Fullscreen) sets fs.userExited for the
   session; we never force fullscreen back on while they play. A fresh launch
   (card click or #funtime/ch navigation) always attempts again. */
function chFsNativeOn() {
  return !!(document.fullscreenElement || document.webkitFullscreenElement);
}
function chFsSupported() {
  const el = document.documentElement;
  return !!(el.requestFullscreen || el.webkitRequestFullscreen);
}

/* Attempt native fullscreen on the Chess panel. Must run inside a user
 * gesture. Returns true when native fullscreen was actually entered.
 *
 * NOTE: we request on <html> when the panel isn't mounted yet (card click)
 * and deliberately do NOT "transfer" to the panel afterwards — chained
 * requestFullscreen calls leave some engines' state stuck. Document-level
 * fullscreen renders identically: chSyncFsState() adds body.ch-fs, and the
 * immersive selectors below style #chPanel the same way. */
async function chTryLaunchFullscreen() {
  if (CHG.fs.userExited) return false;           // respect the user's choice
  if (chFsNativeOn()) { CHG.fs.immersive = false; return true; }
  /* One native request at a time: chained requestFullscreen calls (e.g. a
     card-click request on <html> followed by a mount-time request on the
     panel) leave some engines' fullscreen state stuck. */
  if (CHG.fs.requestPending) return false;
  /* Record the intent FIRST: the card click triggers hash navigation that
     runs renderFunTime -> chTeardown -> renderChView. Teardown resets mount
     state, so the intent survives there and is applied after mount. */
  CHG.fs.launchIntent = true;
  if (!chFsSupported()) { chEnterImmersive(); return false; }
  const panel = document.getElementById('chPanel');
  const target = panel || document.documentElement;
  const req = target.requestFullscreen || target.webkitRequestFullscreen;
  if (!req) { chEnterImmersive(); return false; }
  CHG.fs.requestPending = true;
  try {
    await req.call(target);
    if (chFsNativeOn()) {
      CHG.fs.immersive = false;
      chSyncFsState();
      return true;
    }
  } catch { /* denied: permissions policy, lost gesture, etc. */ }
  finally { CHG.fs.requestPending = false; }
  chEnterImmersive();
  return false;
}

/* CSS immersive mode: fills the viewport without claiming native fullscreen. */
function chEnterImmersive() {
  CHG.fs.immersive = true;
  chSyncFsState();
}
function chLeaveImmersive() {
  CHG.fs.immersive = false;
  chSyncFsState();
}

/* Reflect state onto <body> classes + button labels. Single source of truth.
   body.ch-fs applies the immersive layout for BOTH native modes:
   panel-scoped (#chPanel:fullscreen) and document-scoped (html:fullscreen,
   from a card click made before the panel existed).
   body.ch-playing additionally hides ALL dashboard chrome (sidebar, topbar)
   so only the Chess interface is visible while playing — in native
   fullscreen, immersive fallback, and even inline play.
   The shared FTSESS classes (ft-playing/ft-immersive) mirror this for the
   other four games; Chess sets them too so the whole Fun Time section
   behaves identically. */
function chSyncFsState() {
  const native = chFsNativeOn();
  const docScoped = native && !!(document.fullscreenElement === document.documentElement ||
                                 document.webkitFullscreenElement === document.documentElement);
  const immersive = CHG.fs.immersive && !native;
  document.body.classList.toggle('ch-fs', native || immersive);
  document.body.classList.toggle('immersive', immersive);
  document.body.classList.toggle('ch-fs-doc', docScoped);
  document.body.classList.toggle('ch-playing', true);
  /* Shared session state: ft-playing hides dashboard chrome for every game;
     ft-immersive marks Chess's CSS fallback the same way it does for the
     others. Never set ft-immersive when native fullscreen is genuinely on. */
  document.body.classList.toggle('ft-playing', true);
  document.body.classList.toggle('ft-immersive', !!immersive);
  const btn = document.getElementById('chFsBtn');
  if (btn) {
    btn.setAttribute('aria-pressed', String(native));
    btn.textContent = native ? 'Exit Fullscreen' : 'Fullscreen';
  }
}

/* User explicitly exited fullscreen (Esc key or Exit button). */
function chHandleUserExit() {
  CHG.fs.userExited = true;
  chLeaveImmersive();
  chSyncFsState();
}

/* Leave the Chess game and return to the Fun Time selector.
   This is what Esc does while playing: exit fullscreen AND go back — one
   press, no intermediate state. Guarded so the Escape keydown, the native
   fullscreenchange event, and any racing handler can never navigate twice
   or tear down twice. */
function chLeaveGame() {
  if (CHG.fs.leaving) return;          // exactly-once navigation
  CHG.fs.leaving = true;
  CHG.fs.userExited = true;            // don't auto re-enter this session
  chLeaveImmersive();
  chSyncFsState();
  /* Navigate away; renderFunTime's teardown path cleans listeners via the
     hashchange listener registered in renderChView. If the hash is somehow
     already #funtime (double-fire race), force the re-render explicitly. */
  if (location.hash.startsWith('#funtime/ch')) {
    location.hash = '#funtime';
  } else if (location.hash === '#funtime' || location.hash === '') {
    /* Already navigating there — nothing to do; teardown will run. */
  }
}

/* Enter/leave via the in-game button. Leaving here is an explicit user exit. */
async function chToggleFullscreenButton() {
  if (chFsNativeOn()) {
    chHandleUserExit();
    try {
      if (document.exitFullscreen) await document.exitFullscreen();
      else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
    } catch { /* reflect() reconciles */ }
    return;
  }
  if (CHG.fs.immersive) { chHandleUserExit(); return; }
  const ok = await chTryLaunchFullscreen();
  if (!ok && !CHG.fs.immersive) chEnterImmersive();
}

/* Keep the compact FS header status line in sync with game state. */
function chRenderFsStatus() {
  const host = document.getElementById('chFsStatus');
  const txt = document.getElementById('chFsStatusTxt');
  if (!host || !txt) return;
  const s = chStatusInfo();
  const g = CHG.game;
  let label = s.text;
  if (CHG.thinking) label = 'AI is thinking…';
  else if (g && !CHG.over) {
    label = (g.turn() === 'w' ? 'White' : 'Black') + ' to move' +
            (CHG.mode === 'ai' ? (g.turn() === CHG.humanColor ? ' (you)' : ' (AI)') : '') +
            (g.isCheck() ? ' · Check!' : '');
  }
  txt.textContent = label;
  host.classList.toggle('warn', !!g && (CHG.over || g.isCheck()));
  const info = document.getElementById('chFsInfo');
  if (info) info.textContent = CHG.info || '';
}

/* ---- Game state ------------------------------------------------------- */
const CHG = {
  epoch: 0, cleanup: [],
  game: null,              // chess.js instance (source of truth)
  mode: 'ai',              // 'ai' | '2p'
  difficulty: 'medium',
  humanColor: 'w',         // side the human plays in AI mode
  selected: null,          // square name under selection
  legal: [],               // verbose moves for the selected square
  lastMove: null,          // {from,to} of the last applied move
  over: null,              // {kind, winner, reason}
  thinking: false,
  pendingPromo: null,      // {from,to} awaiting piece choice
  cursor: { r: 4, c: 4 },  // keyboard cursor (board coords)
  info: '',                // engine line for the status area
  /* --- fullscreen session state (Chess-only; not the shared FTFS) --- */
  fs: {
    userExited: false,     // user explicitly left fullscreen this session
    attempted: false,      // launch attempt made for this mount
    immersive: false,      // CSS immersive mode active (NOT native fullscreen)
    launchIntent: false,   // card click requested fullscreen; applied post-mount
    requestPending: false, // a native requestFullscreen() is in flight
    leaving: false,        // guard: Esc/fullscreenchange navigate exactly once
    prevNative: false,     // was native fullscreen on at the previous event
  },
};

function chOn(target, ev, fn, opts) {
  target.addEventListener(ev, fn, opts);
  CHG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
}
function ch$(id) { return document.getElementById(id); }

/* Start a fresh game with the current settings. */
function chNewGame() {
  CHG.epoch++;
  CHG.game = new Chess();
  CHG.selected = null; CHG.legal = []; CHG.lastMove = null;
  CHG.over = null; CHG.thinking = false; CHG.pendingPromo = null;
  CHG.cursor = { r: 4, c: 4 }; CHG.info = '';
  /* A brand-new game is a fresh session: the user's earlier explicit exit
     applied to the previous game. (Relaunching Chess always re-attempts.) */
}

/* Orientation: human's colour at the bottom in AI mode; white at the
   bottom in two-player mode. Returns true when black sits at the bottom. */
function chFlipped() { return CHG.mode === 'ai' && CHG.humanColor === 'b'; }

/* Board coordinates <-> square names. chess.js board(): row 0 = rank 8. */
function chSq(r, c) { return CH_FILES[c] + (8 - r); }
function chRC(sq) {
  const c = CH_FILES.indexOf(sq[0]);
  return { r: 8 - parseInt(sq[1], 10), c };
}
/* Display order: rows/cols run from the far corner toward the viewer. */
function chDispRows() {
  const f = chFlipped();
  const rows = [];
  for (let i = 0; i < 8; i++) rows.push(f ? 7 - i : i);
  return rows;
}
function chDispCols() {
  const f = chFlipped();
  const cols = [];
  for (let i = 0; i < 8; i++) cols.push(f ? 7 - i : i);
  return cols;
}

/* ---- Rendering -------------------------------------------------------- */
function chRenderBoard() {
  const board = document.getElementById('chBoard');
  if (!board || !CHG.game) return;
  const b = CHG.game.board();
  const legalByTo = {};
  for (const m of CHG.legal) legalByTo[m.to] = m;
  let kingSq = null;
  if (CHG.game.isCheck()) {
    const side = CHG.game.turn();
    outer: for (let r = 0; r < 8; r++) for (let c = 0; c < 8; c++) {
      const p = b[r][c];
      if (p && p.type === 'k' && p.color === side) { kingSq = chSq(r, c); break outer; }
    }
  }
  const rows = chDispRows(), cols = chDispCols();
  board.innerHTML = '';
  for (const dr of rows) {
    for (const dc of cols) {
      const sq = chSq(dr, dc);
      const cell = document.createElement('div');
      const light = (dr + dc) % 2 === 0;
      cell.className = 'ch-sq ' + (light ? 'l' : 'd');
      cell.dataset.sq = sq;
      cell.setAttribute('role', 'gridcell');
      cell.setAttribute('aria-label', sq);
      const p = b[dr][dc];
      if (p) {
        const g = document.createElement('span');
        g.className = 'gp ' + (p.color === 'w' ? 'ch-pw' : 'ch-pb');
        g.textContent = CH_GLYPH[p.type];
        g.setAttribute('aria-hidden', 'true');
        cell.appendChild(g);
        cell.setAttribute('aria-label',
          sq + ' ' + (p.color === 'w' ? 'white' : 'black') + ' ' + p.type);
      }
      if (CHG.selected === sq) cell.classList.add('sel');
      if (CHG.lastMove && (CHG.lastMove.from === sq || CHG.lastMove.to === sq))
        cell.classList.add('last');
      if (legalByTo[sq]) cell.classList.add(legalByTo[sq].captured ? 'cap' : 'dot');
      if (kingSq === sq) cell.classList.add('chk');
      if (CHG.cursor.r === dr && CHG.cursor.c === dc) cell.classList.add('cur');
      /* Coordinates on the outer edge only. */
      if (dc === cols[7]) {
        const co = document.createElement('span');
        co.className = 'ch-coord r'; co.textContent = 8 - dr;
        cell.appendChild(co);
      }
      if (dr === rows[7]) {
        const co = document.createElement('span');
        co.className = 'ch-coord f'; co.textContent = CH_FILES[dc];
        cell.appendChild(co);
      }
      board.appendChild(cell);
    }
  }
}

function chCapturedLists() {
  /* Derive captured material from history (verbose), newest last. */
  const out = { w: [], b: [] };
  if (!CHG.game) return out;
  const hist = CHG.game.history({ verbose: true });
  for (const m of hist) {
    if (m.captured) out[m.color].push(m.captured);
  }
  const order = { q: 0, r: 1, b: 2, n: 3, p: 4 };
  out.w.sort((a, b) => order[a] - order[b]);
  out.b.sort((a, b) => order[a] - order[b]);
  return out;
}
function chMaterialDiff() {
  const caps = chCapturedLists();
  const sum = a => a.reduce((t, t2) => t + CH_VAL[t2], 0);
  return sum(caps.w) - sum(caps.b);   // positive = white ahead
}
function chRenderCaptured() {
  const hostW = ch$('chCapTop'), hostB = ch$('chCapBot');
  if (!hostW || !hostB || !CHG.game) return;
  const caps = chCapturedLists();
  /* In AI mode the top bar shows the opponent's gains; keep it simple:
     top = pieces WHITE has captured, bottom = pieces BLACK captured. */
  const fill = (host, arr, colorCls) => {
    host.innerHTML = '<span class="lbl">' + host.dataset.lbl + '</span>';
    if (!arr.length) { host.innerHTML += '<span class="adv">—</span>'; return; }
    for (const t of arr) {
      const s = document.createElement('span');
      s.className = 'gp ' + colorCls;
      s.textContent = CH_GLYPH[t];
      host.appendChild(s);
    }
  };
  fill(hostW, caps.w, 'pw');
  fill(hostB, caps.b, 'pb');
  const diff = chMaterialDiff();
  const advEl = ch$('chAdv');
  if (advEl) {
    if (diff > 0) advEl.textContent = '+' + Math.round(diff / 100) || '+0';
    else if (diff < 0) advEl.textContent = '−' + Math.round(-diff / 100);
    else advEl.textContent = '';
    if (Math.abs(diff) >= 100) {
      advEl.style.color = '#f4f7fc';
      advEl.title = diff > 0 ? 'White is ahead on material' : 'Black is ahead on material';
    } else { advEl.style.color = ''; advEl.title = ''; }
  }
}

function chEvalText() {
  if (!CHG.game) return '0.0';
  const cp = chEvalWhite(CHG.game);
  const v = cp / 100;
  return (v > 0 ? '+' : '') + v.toFixed(1);
}
function chRenderEval() {
  const bar = ch$('chEvalBar'), txt = ch$('chEvalTxt');
  if (!bar || !txt || !CHG.game) return;
  const cp = chEvalWhite(CHG.game);
  /* Map ±800cp onto 8%..92% so the bar stays readable. */
  const clamped = Math.max(-800, Math.min(800, cp));
  const pct = 50 + (clamped / 800) * 42;
  bar.style.width = pct.toFixed(1) + '%';
  txt.textContent = chEvalText();
}

function chRenderHistory() {
  const host = ch$('chHistory');
  if (!host || !CHG.game) return;
  const hist = CHG.game.history();
  if (!hist.length) {
    host.innerHTML = '<div class="empty">No moves yet — make the first move.</div>';
    return;
  }
  let html = '';
  for (let i = 0; i < hist.length; i += 2) {
    const n = i / 2 + 1;
    const wm = hist[i], bm = hist[i + 1];
    html += '<div><span class="no">' + n + '.</span>' +
      '<span class="mv' + (i === hist.length - 1 ? ' last' : '') + '">' + wm + '</span>' +
      (bm ? '<span class="mv' + (i + 1 === hist.length - 1 ? ' last' : '') + '">' + bm + '</span>' : '') +
      '</div>';
  }
  host.innerHTML = html;
  host.scrollTop = host.scrollHeight;
}

/* Compose the status line from the authoritative game state. */
function chStatusInfo() {
  const g = CHG.game;
  if (!g) return { text: '', cls: '' };
  if (CHG.over) {
    const o = CHG.over;
    if (o.kind === 'checkmate')
      return { text: 'Checkmate — ' + (o.winner === 'w' ? 'White' : 'Black') + ' wins.',
               cls: 'warn' };
    if (o.kind === 'stalemate') return { text: 'Stalemate — draw.', cls: 'warn' };
    return { text: 'Draw — ' + o.reason + '.', cls: 'warn' };
  }
  if (CHG.thinking)
    return { text: 'AI is thinking…', cls: '', think: true };
  const turnName = g.turn() === 'w' ? 'White' : 'Black';
  let text = turnName + ' to move';
  if (CHG.mode === 'ai') {
    text += g.turn() === CHG.humanColor ? ' (you)' : ' (AI)';
  }
  if (g.isCheck()) text += ' · Check!';
  return { text, cls: g.isCheck() ? 'warn' : '' };
}
function chRenderStatus() {
  const el = ch$('chStatus');
  if (!el) return;
  const s = chStatusInfo();
  el.className = 'ch-status ' + s.cls;
  el.innerHTML =
    (s.think
      ? '<span class="ch-think"><span class="spin"></span>AI is thinking…</span>'
      : '<span class="dot"></span>' + s.text) +
    (CHG.info ? ' <span style="margin-left:auto;font:600 12px JetBrains Mono,monospace;color:#8b98ad">' + CHG.info + '</span>' : '');
}

function chRenderMeta() {
  const el = ch$('chMeta');
  if (!el) return;
  const parts = [];
  parts.push('<span>Mode: <b>' + (CHG.mode === 'ai' ? 'Player vs AI' : 'Two players') + '</b></span>');
  if (CHG.mode === 'ai') {
    parts.push('<span>You play <b>' + (CHG.humanColor === 'w' ? 'White' : 'Black') + '</b> · Difficulty: <b>' +
      (CH_DIFF[CHG.difficulty] || CH_DIFF.medium).label + '</b></span>');
  } else {
    parts.push('<span>Pass-and-play on one board.</span>');
  }
  parts.push('<span>Moves: <b>' + (CHG.game ? CHG.game.history().length : 0) + '</b></span>');
  el.innerHTML = parts.join('');
}

function chRenderAll() {
  chRenderBoard(); chRenderCaptured(); chRenderEval();
  chRenderHistory(); chRenderStatus(); chRenderMeta();
  chRenderFsStatus();
}

/* Detect and record terminal states once. */
function chCheckOver() {
  const g = CHG.game;
  if (!g || CHG.over) return;  if (g.isCheckmate()) {
    CHG.over = { kind: 'checkmate', winner: g.turn() === 'w' ? 'b' : 'w' };
    chSfx(CHG.mode === 'ai'
      ? (CHG.over.winner === CHG.humanColor ? 'win' : 'lose')
      : 'win');
  } else if (g.isStalemate()) {
    CHG.over = { kind: 'stalemate' }; chSfx('draw');
  } else if (g.isInsufficientMaterial()) {
    CHG.over = { kind: 'draw', reason: 'insufficient material' }; chSfx('draw');
  } else if (g.isThreefoldRepetition()) {
    CHG.over = { kind: 'draw', reason: 'threefold repetition' }; chSfx('draw');
  } else if (g.isDraw()) {
    CHG.over = { kind: 'draw', reason: 'fifty-move rule' }; chSfx('draw');
  }
}

/* ---- Move application ------------------------------------------------- */
function chApplyMoveObj(mv) {
  const g = CHG.game;
  if (!g) return null;
  const res = g.move({ from: mv.from, to: mv.to, promotion: mv.promotion });
  if (!res) return null;
  CHG.lastMove = { from: res.from, to: res.to };
  CHG.selected = null; CHG.legal = [];
  if (res.captured) chSfx('cap'); else chSfx('move');
  chCheckOver();
  if (!CHG.over && g.isCheck()) chSfx('check');
  return res;
}

/* Human clicked a destination square for the selected piece. Handles the
   promotion choice before applying anything. */
function chTryHumanMove(from, to) {
  const cand = CHG.legal.filter(m => m.to === to);
  if (!cand.length) return false;
  const needsPromo = cand.some(m => m.flags.indexOf('p') >= 0 || m.promotion);
  if (needsPromo) {
    CHG.pendingPromo = { from, to };
    const ovl = ch$('chPromoOvl');
    if (ovl) ovl.hidden = false;
    chRenderBoard();
    return true;
  }
  const mv = cand[0];
  const res = chApplyMoveObj(mv);
  if (!res) { chSfx('err'); return false; }
  chAfterHumanMove();
  return true;
}
function chAfterHumanMove() {
  chRenderAll();
  if (!CHG.over && CHG.mode === 'ai' && CHG.game.turn() !== CHG.humanColor)
    chScheduleAI();
}

/* Promotion picker */
function chChoosePromo(piece) {
  const pp = CHG.pendingPromo;
  CHG.pendingPromo = null;
  const ovl = ch$('chPromoOvl');
  if (ovl) ovl.hidden = true;
  if (!pp) return;
  const res = chApplyMoveObj({ from: pp.from, to: pp.to, promotion: piece });
  if (!res) { chSfx('err'); return; }
  chAfterHumanMove();
}

/* ---- AI turn ----------------------------------------------------------
   Runs the real engine after a paint beat so "thinking…" renders first.
   The epoch guard makes a stale search harmless after reset/teardown. */
function chScheduleAI() {
  if (CHG.over || CHG.mode !== 'ai') return;
  if (!CHG.game || CHG.game.turn() === CHG.humanColor) return;
  CHG.thinking = true;
  chRenderStatus();
  const myEpoch = CHG.epoch;
  setTimeout(() => {
    if (myEpoch !== CHG.epoch) return;      // reset/restarted meanwhile
    let out = null;
    try { out = chFindBestMove(CHG.game.fen(), CHG.difficulty); }
    catch (e) { out = null; }
    if (myEpoch !== CHG.epoch) return;
    CHG.thinking = false;
    if (out) {
      const res = chApplyMoveObj(out);
      if (!res) {
        /* Engine returned something illegal (should not happen): fall back
           to the first legal move rather than freezing the game. */
        const legal = CHG.game.moves({ verbose: true });
        if (legal.length) chApplyMoveObj(legal[0]);
      }
      CHG.info = 'd' + out.depth + ' · ' + out.nodes.toLocaleString() + ' nodes';
    } else {
      const legal = CHG.game.moves({ verbose: true });
      if (legal.length) chApplyMoveObj(legal[0]);
      CHG.info = '';
    }
    chRenderAll();
  }, 40);
}

/* ---- Interaction ------------------------------------------------------ */
function chOnSquareClick(sq) {
  if (!CHG.game || CHG.over || CHG.thinking) return;
  if (CHG.mode === 'ai' && CHG.game.turn() !== CHG.humanColor) return;
  const rc = chRC(sq);
  CHG.cursor = rc;
  const b = CHG.game.board();
  const p = b[rc.r] && b[rc.r][rc.c];
  if (CHG.selected) {
    if (sq === CHG.selected) {                 // toggle off
      CHG.selected = null; CHG.legal = []; chRenderBoard(); return;
    }
    if (chTryHumanMove(CHG.selected, sq)) return;
  }
  if (p && p.color === CHG.game.turn()) {
    CHG.selected = sq;
    CHG.legal = CHG.game.moves({ square: sq, verbose: true });
    chSfx('ui');
  } else {
    CHG.selected = null; CHG.legal = [];
  }
  chRenderBoard();
}

/* Keyboard: arrows move the cursor, Enter/Space select or move,
   Escape clears selection / closes the promotion dialog. */
function chOnKey(e) {
  if (!CHG.game) return;
  if (e.key === 'Escape') {
    const ovl = ch$('chPromoOvl');
    if (ovl && !ovl.hidden) {
      CHG.pendingPromo = null; ovl.hidden = true; chRenderBoard(); return;
    }
    if (CHG.selected) { CHG.selected = null; CHG.legal = []; chRenderBoard(); }
    return;
  }
  const map = {
    ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1],
  };
  const d = map[e.key];
  if (d) {
    e.preventDefault();
    /* Cursor moves in display space so it matches what the user sees. */
    const rows = chDispRows(), cols = chDispCols();
    let di = rows.indexOf(CHG.cursor.r), dj = cols.indexOf(CHG.cursor.c);
    di = Math.min(rows.length - 1, Math.max(0, di + d[0]));
    dj = Math.min(cols.length - 1, Math.max(0, dj + d[1]));
    CHG.cursor = { r: rows[di], c: cols[dj] };
    chRenderBoard();
    return;
  }
  if (e.key === 'Enter' || e.key === ' ') {
    const sq = chSq(CHG.cursor.r, CHG.cursor.c);
    if (document.activeElement && document.activeElement.tagName === 'BUTTON') return;
    e.preventDefault();
    chOnSquareClick(sq);
  }
}

/* Undo: in AI mode rewind to the human's turn again (one full exchange);
   in two-player mode rewind a single ply. */
function chUndo() {
  if (!CHG.game || CHG.thinking) return;
  const hist = CHG.game.history();
  if (!hist.length) { chSfx('err'); return; }
  if (CHG.mode === 'ai') {
    if (CHG.game.turn() === CHG.humanColor) {
      if (hist.length >= 2) { CHG.game.undo(); CHG.game.undo(); }
      else CHG.game.undo();
    } else {
      CHG.game.undo();
    }
  } else {
    CHG.game.undo();
  }
  const h = CHG.game.history({ verbose: true });
  CHG.lastMove = h.length ? { from: h[h.length - 1].from, to: h[h.length - 1].to } : null;
  CHG.over = null; CHG.selected = null; CHG.legal = [];
  CHG.info = '';
  chSfx('ui');
  chRenderAll();
  /* If undo left the AI on move (start position with AI as white), let it move. */
  if (CHG.mode === 'ai' && !CHG.over && CHG.game.turn() !== CHG.humanColor)
    chScheduleAI();
}

function chRestart() { chNewGame(); chRenderAll(); chSfx('ui'); chMaybeAutoMoveAI(); }

/* If the AI holds the first move (human plays black), start it moving. */
function chMaybeAutoMoveAI() {
  if (CHG.mode === 'ai' && CHG.game && CHG.game.turn() !== CHG.humanColor && !CHG.over)
    chScheduleAI();
}

/* Setup controls: mode / colour / difficulty take effect on New Game. */
function chSetMode(mode) {
  CHG.mode = mode;
  chSyncSetup();
}
function chSetSide(color) {
  CHG.humanColor = color;
  chSyncSetup();
}
function chSetDifficulty(d) {
  CHG.difficulty = d;
  chSyncSetup();
}
function chSyncSetup() {
  document.querySelectorAll('#chPanel .ch-seg[data-field="mode"] button')
    .forEach(b => {
      const on = b.dataset.v === CHG.mode;
      b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on));
    });
  document.querySelectorAll('#chPanel .ch-seg[data-field="side"] button')
    .forEach(b => {
      const on = b.dataset.v === CHG.humanColor;
      b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on));
    });
  document.querySelectorAll('#chPanel .ch-diffbar button')
    .forEach(b => {
      const on = b.dataset.d === CHG.difficulty;
      b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on));
    });
  const sideSeg = document.querySelector('#chPanel .ch-field[data-field="side"]');
  const diffField = document.querySelector('#chPanel .ch-field[data-field="diff"]');
  const aiOnly = CHG.mode === 'ai';
  if (sideSeg) sideSeg.style.display = aiOnly ? '' : 'none';
  if (diffField) diffField.style.display = aiOnly ? '' : 'none';
}

/* ---- View ------------------------------------------------------------- */
function renderChView() {
  chNewGame();
  const setupVisible = true;   /* the setup card doubles as live controls */
  document.getElementById('viewRoot').innerHTML = `
  <div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Chess</div>
    </div>

    <section class="ft-panel ft-game ch-wide" id="chPanel" aria-label="Chess">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Chess</div>
          <div class="ft-blurb">Full rules — castling, en passant, promotion. Play the AI or a friend.</div>
        </div>
      </header>

      <!-- Compact header shown only in fullscreen / immersive mode -->
      <div class="ch-fsbar" aria-hidden="true">
        <span class="ch-fs-title">♞ Chess</span>
        <span class="ch-fs-status" id="chFsStatus"><span class="dot"></span><span id="chFsStatusTxt">White to move</span></span>
        <span class="ch-fs-info" id="chFsInfo"></span>
      </div>

      <div class="ch-setup">
        <div class="ch-setup-grid">
          <div class="ch-field" data-field="mode">
            <label>Mode</label>
            <div class="ch-seg" data-field="mode" role="group" aria-label="Game mode">
              <button type="button" data-v="ai">Player vs AI</button>
              <button type="button" data-v="2p">Two Players</button>
            </div>
          </div>
          <div class="ch-field" data-field="side">
            <label>Your side</label>
            <div class="ch-seg" data-field="side" role="group" aria-label="Your colour">
              <button type="button" data-v="w">White</button>
              <button type="button" data-v="b">Black</button>
            </div>
          </div>
          <div class="ch-field" data-field="diff">
            <label>Difficulty</label>
            <div class="ch-diffbar" role="group" aria-label="AI difficulty">
              ${Object.keys(CH_DIFF).map(k =>
                `<button type="button" data-d="${k}">${CH_DIFF[k].label}</button>`).join('')}
            </div>
          </div>
        </div>
        <div class="ch-setup-actions">
          <button type="button" class="ft-btn primary" id="chStart">New Game</button>
          <span class="ch-note">Changing mode / side starts a new game. Difficulty applies immediately.</span>
        </div>
      </div>

      <div class="ch-status" id="chStatus" role="status" aria-live="polite"></div>

      <div class="ch-layout">
        <div class="ch-main">
          <div class="ch-capbar" id="chCapTop" data-lbl="White took"></div>
          <div class="ch-board-wrap">
            <div class="ch-board" id="chBoard" tabindex="0" role="grid"
                 aria-label="Chess board. Use arrow keys to move, Enter to select and place."></div>
            <div class="ch-ovl" id="chPromoOvl" hidden>
              <div class="ch-ovl-box">
                <div class="ch-ovl-title">Promote pawn</div>
                <div class="ch-ovl-sub">Choose a piece</div>
                <div class="ch-promo-row" role="group" aria-label="Promotion piece">
                  <button type="button" data-p="q" aria-label="Queen">${CH_GLYPH.q}</button>
                  <button type="button" data-p="r" aria-label="Rook">${CH_GLYPH.r}</button>
                  <button type="button" data-p="b" aria-label="Bishop">${CH_GLYPH.b}</button>
                  <button type="button" data-p="n" aria-label="Knight">${CH_GLYPH.n}</button>
                </div>
              </div>
            </div>
          </div>
          <div class="ch-capbar" id="chCapBot" data-lbl="Black took"></div>
        </div>

        <aside class="ch-side">
          <div class="ch-card">
            <h4>Evaluation</h4>
            <div class="ch-evalrow">
              <div class="ch-evalbar"><i id="chEvalBar"></i></div>
              <div class="ch-evaltxt" id="chEvalTxt">0.0</div>
            </div>
          </div>
          <div class="ch-card">
            <h4>Move history</h4>
            <div class="ch-hist" id="chHistory" tabindex="0" aria-label="Move history"></div>
          </div>
          <div class="ch-card">
            <h4>Game</h4>
            <div class="ch-meta" id="chMeta"></div>
          </div>
        </aside>
      </div>

      <div id="chOvLaunch"></div>

      <div class="ft-actions">
        <button type="button" class="ft-btn" id="chUndoBtn">Undo</button>
        <button type="button" class="ft-btn" id="chRestartBtn">Restart</button>
        <button type="button" class="ft-btn" id="chFlipBtn">Flip Board</button>
        <button type="button" class="ft-btn" id="chSoundBtn" aria-pressed="false">🔊 <span id="chSoundLbl">sound off</span></button>
        <button type="button" class="ft-btn" id="chFsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn ch-fs-retry" id="chFsRetryBtn" title="Retry native fullscreen">
          ⛶ Retry Fullscreen
        </button>
        <button type="button" class="ft-btn si-exit" id="chExit">Back to Games</button>
      </div>
    </section>
  </div></div>`;

  chSyncSetup();
  chRenderAll();

  /* Every listener goes through chOn() so chTeardown() removes them all. */
  chOn(document.getElementById('chBoard'), 'click', e => {
    const cell = e.target.closest('.ch-sq');
    if (cell) chOnSquareClick(cell.dataset.sq);
  });
  chOn(document.getElementById('chBoard'), 'keydown', chOnKey);

  document.querySelectorAll('#chPanel .ch-seg[data-field="mode"] button')
    .forEach(b => chOn(b, 'click', () => { chSetMode(b.dataset.v); chNewGame(); chRenderAll(); chMaybeAutoMoveAI(); }));
  document.querySelectorAll('#chPanel .ch-seg[data-field="side"] button')
    .forEach(b => chOn(b, 'click', () => { chSetSide(b.dataset.v); chNewGame(); chRenderAll(); chMaybeAutoMoveAI(); }));
  document.querySelectorAll('#chPanel .ch-diffbar button')
    .forEach(b => chOn(b, 'click', () => { chSetDifficulty(b.dataset.d); chSfx('ui'); }));

  chOn(ch$('chStart'), 'click', () => { chNewGame(); chRenderAll(); chSfx('ui'); chMaybeAutoMoveAI(); });
  chOn(ch$('chUndoBtn'), 'click', chUndo);
  chOn(ch$('chRestartBtn'), 'click', chRestart);
  chOn(ch$('chFlipBtn'), 'click', () => {
    /* Manual flip swaps orientation in both modes. */
    CHG._manualFlip = !CHG._manualFlip;
    chRenderBoard(); chSfx('ui');
  });
  chOn(ch$('chSoundBtn'), 'click', chSoundToggle);
  chOn(ch$('chExit'), 'click', () => { CHG.fs.leaving = false; chLeaveGame(); });

  document.querySelectorAll('#chPromoOvl .ch-promo-row button')
    .forEach(b => chOn(b, 'click', () => chChoosePromo(b.dataset.p)));

  /* ---- Chess-specific fullscreen wiring (NOT the shared FTFS) ----
     The in-game Fullscreen button toggles; an explicit exit is respected
     for the session. The retry button appears only in immersive fallback. */
  chOn(ch$('chFsBtn'), 'click', chToggleFullscreenButton);
  chOn(ch$('chFsRetryBtn'), 'click', async () => {
    CHG.fs.userExited = false;               // retry is an explicit re-consent
    const ok = await chTryLaunchFullscreen();
    if (ok) chLeaveImmersive();
    chSyncFsState(); chRenderBoard();
  });

  /* Native fullscreenchange. Entering: sync classes. Leaving: the user
     pressed Esc or a browser control — return to Fun Time immediately,
     never leaving the game in a broken intermediate state. The `leaving`
     guard makes the Escape keydown and this event agree on ONE navigation
     no matter which fires first. */
  const chFsChanged = () => {
    const native = chFsNativeOn();
    if (native) {
      CHG.fs.immersive = false;
      CHG.fs.prevNative = true;
      chSyncFsState();
    } else {
      const wasNative = CHG.fs.prevNative;
      CHG.fs.prevNative = false;
      if (wasNative) {
        /* Left native fullscreen while playing: Esc semantics — go back to
           the games screen. (If we're already leaving, the guard no-ops.) */
        chLeaveGame();
      } else {
        chSyncFsState();
      }
    }
    chRenderBoard();
  };
  chOn(document, 'fullscreenchange', chFsChanged);
  chOn(document, 'webkitfullscreenchange', chFsChanged);

  /* ESC exits the GAME: fullscreen + back to Fun Time, one press.
     • Native fullscreen: the browser handles the exit itself; our handler
       only navigates (the fullscreenchange handler is guarded by `leaving`
       so there's no duplicate). We do NOT preventDefault or swallow Esc —
       the browser's own exit still happens.
     • CSS immersive fallback: no native FS involved, so we navigate and
       drop immersive mode ourselves.
     • Inline (no FS at all): still leaves the game — Esc means "back". */
  chOn(window, 'keydown', e => {
    if (e.key !== 'Escape') return;
    if (!location.hash.startsWith('#funtime/ch')) return;   // not our view
    if (CHG.fs.leaving) return;                             // already leaving
    if (chFsNativeOn()) {
      /* Ask the browser to leave fullscreen too (idempotent; some embedded
         engines don't process Esc natively). Navigation happens via the
       guarded handlers below — either this keydown path or fullscreenchange. */
      try {
        if (document.exitFullscreen) { const r = document.exitFullscreen(); if (r && r.catch) r.catch(() => {}); }
        else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
      } catch { /* browser will handle it */ }
      chLeaveGame();          // guarded: runs once even if fullscreenchange also fires
    } else {
      chLeaveGame();
    }
  });
  chOn(window, 'resize', () => { chRenderBoard(); });
  chSyncFsState();

  chOn(window, 'hashchange', () => {
    if (!location.hash.startsWith('#funtime/ch')) {
      chTeardown();
    }
  }, { once: true });

  /* Fullscreen is handled entirely by the Chess-local system above —
     deliberately NOT ftBindFullscreen/ftLaunchRow (shared FTFS stays
     untouched for the other games; Chess gets its own launch-on-click,
     immersive fallback, and session-respect semantics). */

  /* Mount-time fullscreen reconciliation:
     • Card click set launchIntent and may already have a native request in
       flight on <html> (requestPending) — do NOT issue a second request;
       chained requests stick some engines. Just sync state; when the first
       request lands, chFsChanged keeps everything consistent.
     • If no request is in flight and none was granted (denied inside the
       gesture before mount), fall back to immersive now.
     • Direct #funtime/ch navigation (no gesture): stay inline. */
  if (CHG.fs.launchIntent && !CHG.fs.userExited) {
    CHG.fs.launchIntent = false;                 // consume the intent
    if (!chFsNativeOn() && !CHG.fs.requestPending) {
      chTryLaunchFullscreen().then(() => { chRenderBoard(); }).catch(() => {});
    }
  }
  chSyncFsState();
  chRenderBoard();

  chSoundButton();
}

/* ---- Teardown --------------------------------------------------------- */
function chTeardown() {
  CHG.epoch++;                    // invalidates any pending AI continuation
  for (const off of CHG.cleanup.splice(0)) { try { off(); } catch { /* gone */ } }
  CHG.game = null;
  CHG.selected = null; CHG.legal = []; CHG.pendingPromo = null;
  CHG.thinking = false; CHG.over = null; CHG.lastMove = null;
  chAudioDispose();
  /* Leave whatever fullscreen mode we're in, quietly — EXCEPT when this
     teardown is the route's own remount cycle (card click navigated here):
     then launchIntent is set and the view re-applies fullscreen right after.
     Native exit on genuine navigation is not a user decision, so the next
     Chess launch attempts fullscreen again. */
  const remounting = CHG.fs.launchIntent && location.hash.startsWith('#funtime/ch');
  if (!remounting && chFsNativeOn()) {
    try {
      if (document.exitFullscreen) { const r = document.exitFullscreen(); if (r && r.catch) r.catch(() => {}); }
      else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
    } catch { /* already leaving */ }
  }
  if (!remounting) {
    CHG.fs.userExited = false;
    CHG.fs.attempted = false;
    CHG.fs.immersive = false;
    CHG.fs.leaving = false;        // a fresh mount can always leave again
    CHG.fs.prevNative = false;
    document.body.classList.remove('ch-fs');
    document.body.classList.remove('immersive');
    document.body.classList.remove('ch-fs-doc');
    document.body.classList.remove('ch-playing');
    document.body.classList.remove('ft-playing');   // shared session state
    document.body.classList.remove('ft-immersive');
  }
}

/* ---- Board flip support (manual flip) --------------------------------- */
(function chPatchFlip() {
  /* chFlipped() consults CHG._manualFlip so the Flip Board button works in
     both modes; keep the original definition intact and wrap it once. */
  const orig = chFlipped;
  chFlipped = function () {
    const base = orig();
    return CHG._manualFlip ? !base : base;
  };
})();

/* Expose the game on the window object for the console shell (and tests):
   the router calls renderChView()/chTeardown(), tests drive CHG/chFindBestMove. */
window.renderChView = renderChView;
window.chTeardown = chTeardown;
window.CHG = CHG;
window.CH_DIFF = CH_DIFF;
window.chFindBestMove = chFindBestMove;
window.chEvalWhite = chEvalWhite;
/* Fullscreen helpers exposed for the Fun Time card hook and tests. */
window.chTryLaunchFullscreen = chTryLaunchFullscreen;
window.chSyncFsState = chSyncFsState;
window.chFsNativeOn = chFsNativeOn;
window.CHG_FS = CHG.fs;
/* Load a position and immediately settle any terminal state + repaint, so
   externally-supplied FENs (tests, deep links) behave exactly like moves. */
window.chLoadFen = function (fen) {
  if (!CHG.game) return false;
  const ok = CHG.game.load(fen);
  CHG.selected = null; CHG.legal = []; CHG.pendingPromo = null;
  const h = CHG.game.history({ verbose: true });
  CHG.lastMove = h.length ? { from: h[h.length - 1].from, to: h[h.length - 1].to } : null;
  CHG.over = null;
  chCheckOver();
  chRenderAll();
  return ok;
};
