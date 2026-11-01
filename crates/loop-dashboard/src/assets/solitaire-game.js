/* ============================================================
   FUN TIME · GAME 7 — SOLITAIRE (CLASSIC KLONDIKE, DRAW 1)
   Route: #funtime/so · Namespace: SO / so* functions / .so- CSS.

   Two files:
     solitaire-engine.js  — pure rules (DOM-free, unit-tested)
     solitaire-game.js    — this file: rendering + interaction

   Design notes (v2 — full table redesign):
     • Slots are real card-shaped boxes (CSS aspect-ratio) so the grid
       can never collapse — v1's 2px-tall slots stacked every card.
     • One geometry pass (soPlan()) computes EVERY card position,
       z-index and per-column offset compression before painting.
     • Card faces use classic pip arrangements (2–10 laid out like a
       real deck, courts framed, Ace oversized) instead of one glyph.
     • The waste fans its last three cards; the stock shows thickness.
     • Double-click plays a smart move (foundation first, then the
       most useful tableau destination).
     • Auto-Finish appears the moment only foundation moves remain.
     • Winning triggers the iconic cascading-cards celebration with
       real gravity/bounce physics (skipped under reduced motion).

   Lifecycle mirrors the other Fun Time games exactly:
     • one state object (SOG) with an epoch guard,
     • every listener registered through soOnLocal() so soTeardown()
       removes them all on any route change,
     • automatic fullscreen from the card click via FTSESS,
     • Esc leaves the game and returns to #funtime in ONE press,
     • dashboard chrome hidden while playing (body.ft-playing),
     • zero DOM/CSS leakage between mounts.
   ============================================================ */

/* ---- Injected styles (namespaced; no global selectors touched) ---- */
(function soInjectCss() {
  if (document.getElementById('so-style')) return;
  const st = document.createElement('style');
  st.id = 'so-style';
  st.textContent = `
/* ---------- Layout ----------
   Compact spec: cards ≤130px wide (capped by .so-row max-width), status
   bar ~30px, controls ~30px. All sizes flow from the slot grid. */
.so-wide{max-width:1020px}
.so-hud{display:flex;gap:8px;flex-wrap:wrap;align-items:center;padding:0 18px}
.so-stat{display:inline-flex;align-items:center;gap:6px;border:1px solid var(--line2);border-radius:8px;
  padding:4px 10px;background:rgba(0,0,0,.32);font:600 11px Sora;color:#aab6c8}
.so-stat .k{text-transform:uppercase;letter-spacing:.08em;font-size:9.5px;color:#7f8ca1}
.so-stat .v{font:700 13.5px "JetBrains Mono",monospace;color:#eaf2ff;min-width:3ch;text-align:right}
.so-stat.score .v{color:var(--acc)}
.so-stat.time .v{color:#ffd166}
.so-stat.found .v{color:#9fd0ff}
.so-controls{display:flex;gap:8px;flex-wrap:wrap;align-items:center;padding:0 18px}
/* Compact buttons, scoped to this game only (global .ft-btn untouched). */
.so-controls .ft-btn{padding:6px 12px;font-size:11px;border-radius:8px}
.so-help{display:flex;gap:14px;flex-wrap:wrap;padding:2px 20px 4px;font:500 11px Sora;color:#7f8ca1}

/* ---------- The felt table ---------- */
.so-table{
  position:relative;margin:6px 18px 14px;border-radius:14px;
  background:
    radial-gradient(ellipse 120% 90% at 50% -10%, rgba(255,255,255,.055), transparent 55%),
    radial-gradient(ellipse 80% 60% at 50% 110%, rgba(0,0,0,.42), transparent 60%),
    linear-gradient(160deg,#0f5138 0%,#0b4230 40%,#07301f 100%);
  border:1px solid rgba(255,255,255,.07);
  box-shadow:inset 0 1px 0 rgba(255,255,255,.06), inset 0 -30px 60px -30px rgba(0,0,0,.55), 0 18px 44px -22px rgba(0,0,0,.7);
  overflow:hidden;
  user-select:none;-webkit-user-select:none;
  touch-action:manipulation;
}
.so-table::before{
  content:'';position:absolute;inset:0;pointer-events:none;z-index:0;
  background-image:
    repeating-linear-gradient(45deg, rgba(255,255,255,.012) 0 2px, transparent 2px 4px),
    repeating-linear-gradient(-45deg, rgba(0,0,0,.02) 0 2px, transparent 2px 4px);
}
.so-board{position:relative;padding:clamp(8px,1.2vw,14px);z-index:1}

/* ---------- Piles ----------
   Slots are genuine card-shaped boxes: width from the grid (capped by
   the row's max-width so cards stay ≤ ~126px), height from aspect-ratio.
   This is the structural fix — v1 slots collapsed to 2px and every card
   piled into them. */
.so-row{position:relative;display:grid;grid-template-columns:repeat(7,minmax(0,1fr));
  gap:clamp(6px,.8vw,10px);
  /* 7×cardW + 6×gap ≤ board width → cardW ≈ 118–126px on desktop */
  max-width:min(100%, calc(7 * 126px + 6 * 10px));margin:0 auto}
.so-row.top{margin-bottom:clamp(10px,1.8vh,18px)}
.so-spacer{aspect-ratio:5/7}
.so-slot{
  position:relative;border-radius:var(--soRad,10px);
  aspect-ratio:5/7;
  border:1.5px dashed rgba(255,255,255,.16);
  background:rgba(0,0,0,.14);
  transition:border-color .18s, background .18s, box-shadow .18s;
}
.so-slot::after{
  content:attr(data-mark);position:absolute;inset:0;display:grid;place-items:center;
  font-size:calc(var(--soCardH,80px) * .26);color:rgba(255,255,255,.10);line-height:1;
}
.so-slot.stock-empty::after{content:'↺';font-size:calc(var(--soCardH,80px) * .24)}
.so-slot.drop-ok{border-color:rgba(94,242,192,.75);border-style:solid;background:rgba(94,242,192,.10);
  box-shadow:0 0 0 3px rgba(94,242,192,.14), inset 0 0 22px rgba(94,242,192,.10)}
.so-slot.hint{animation:sohint 1.15s ease-in-out 2}
@keyframes sohint{0%,100%{box-shadow:0 0 0 0 rgba(94,242,192,0)}50%{box-shadow:0 0 0 4px rgba(94,242,192,.45)}}
.so-label{position:absolute;left:0;right:0;top:-15px;text-align:center;
  font:700 8.5px Sora;letter-spacing:.12em;text-transform:uppercase;color:rgba(255,255,255,.26);pointer-events:none}

/* ---------- Cards ---------- */
.so-card{
  position:absolute;left:0;top:0;
  width:var(--soCardW);height:var(--soCardH);
  border-radius:var(--soRad,10px);
  transform:translate3d(var(--tx,0px),var(--ty,0px),0);
  transition:transform .28s cubic-bezier(.22,.9,.28,1), box-shadow .2s;
  will-change:transform;
  cursor:pointer;
  box-shadow:0 1px 2px rgba(0,0,0,.35), 0 3px 8px -2px rgba(0,0,0,.35);
}
.so-card .face,.so-card .back{
  position:absolute;inset:0;border-radius:inherit;
  backface-visibility:hidden;-webkit-backface-visibility:hidden;
}
.so-card .face{
  background:linear-gradient(155deg,#fffdf7 0%,#f7f1e4 100%);
  border:1px solid rgba(0,0,0,.14);
  box-shadow:inset 0 1px 0 #fff, inset 0 0 0 1px rgba(255,255,255,.6);
  overflow:hidden;
}
.so-card .back{
  transform:rotateY(180deg);
  background:
    radial-gradient(circle at 30% 25%, rgba(94,242,192,.20), transparent 60%),
    repeating-linear-gradient(45deg, rgba(94,242,192,.10) 0 3px, transparent 3px 7px),
    repeating-linear-gradient(-45deg, rgba(94,242,192,.08) 0 3px, transparent 3px 7px),
    linear-gradient(150deg,#132a3f 0%,#0d1c2c 55%,#101f31 100%);
  border:1px solid rgba(94,242,192,.22);
  box-shadow:inset 0 0 0 3px rgba(6,12,20,.85), inset 0 1px 0 rgba(255,255,255,.08);
}
.so-card.facedown{transform:translate3d(var(--tx,0px),var(--ty,0px),0) rotateY(180deg)}
.so-card.faceup{transform:translate3d(var(--tx,0px),var(--ty,0px),0) rotateY(0deg)}
.so-card.dragging{
  transition:none !important;z-index:999 !important;
  box-shadow:0 12px 30px -8px rgba(0,0,0,.65), 0 4px 10px rgba(0,0,0,.35);
  cursor:grabbing;
}
.so-card.selected{outline:3px solid rgba(94,242,192,.85);outline-offset:1px;
  box-shadow:0 6px 18px -4px rgba(94,242,192,.5)}
.so-card.hint-card{animation:sohintcard 1.15s ease-in-out 2}
@keyframes sohintcard{0%,100%{box-shadow:0 1px 2px rgba(0,0,0,.35)}50%{box-shadow:0 0 0 4px rgba(94,242,192,.6), 0 6px 20px rgba(94,242,192,.4)}}
.so-card.invalid{animation:soshake .3s}
@keyframes soshake{0%,100%{margin-left:0}25%{margin-left:-5px}75%{margin-left:5px}}

/* corner indices */
.so-card .idx{position:absolute;display:flex;flex-direction:column;align-items:center;line-height:.95;
  font-family:Sora;font-weight:800;letter-spacing:-.03em}
.so-card .idx.tl{top:4.5%;left:6.5%}
.so-card .idx.br{bottom:4.5%;right:6.5%;transform:rotate(180deg)}
.so-card .idx .r{font-size:calc(var(--soCardH) * .155)}
.so-card .idx .r.wide{font-size:calc(var(--soCardH) * .128);letter-spacing:-.06em}
.so-card .idx .s{font-size:calc(var(--soCardH) * .135);margin-top:1px}

/* classic pip field for 2..10 */
.so-card .pips{position:absolute;inset:6% 17%;pointer-events:none}
.so-card .pip{position:absolute;transform:translate(-50%,-50%);
  font-size:calc(var(--soCardH) * .155);line-height:1;text-shadow:0 1px 0 rgba(0,0,0,.06)}
.so-card .pip.flip{transform:translate(-50%,-50%) rotate(180deg)}
.so-card .ace{position:absolute;inset:0;display:grid;place-items:center;
  font-size:calc(var(--soCardH) * .44);line-height:1;text-shadow:0 1px 0 rgba(0,0,0,.06)}
.so-card .court{position:absolute;inset:16% 22%;border:1.5px solid currentColor;border-radius:8%;
  display:flex;flex-direction:column;align-items:center;justify-content:center;gap:2%;opacity:.94}
.so-card .court .letter{font:800 calc(var(--soCardH) * .26) Sora;letter-spacing:-.04em;line-height:1}
.so-card .court .csuit{font-size:calc(var(--soCardH) * .175);line-height:1}
.so-card.red{color:#c8102e}
.so-card.black{color:#151a22}
.so-brand{position:absolute;right:12px;bottom:9px;font:700 9.5px Sora;
  letter-spacing:.16em;color:rgba(255,255,255,.16);text-transform:uppercase;pointer-events:none;z-index:2}

/* ---------- Overlays ---------- */
.so-overlay{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;
  background:radial-gradient(ellipse at center,rgba(4,10,8,.72) 0%,rgba(3,8,6,.92) 100%);
  backdrop-filter:blur(4px);z-index:300;text-align:center;padding:18px}
.so-overlay.hidden{display:none}
.so-ov-inner{max-width:480px;width:100%}
.so-ov-title{font:800 clamp(24px,4vw,38px) Sora;letter-spacing:-.02em;color:#f2f7ff;margin-bottom:6px;
  text-shadow:0 0 30px rgba(94,242,192,.25)}
.so-ov-sub{font:500 13.5px Sora;color:#9fb0c7;line-height:1.65;margin-bottom:16px}
.so-ov-stats{display:flex;gap:12px;justify-content:center;margin-bottom:18px;flex-wrap:wrap}
.so-ov-stat{border:1px solid var(--line2);border-radius:12px;padding:10px 18px;background:rgba(0,0,0,.4)}
.so-ov-stat .k{display:block;font:700 10px Sora;letter-spacing:.14em;text-transform:uppercase;color:#7f8ca1;margin-bottom:4px}
.so-ov-stat .v{font:800 21px "JetBrains Mono",monospace;color:#fff}
.so-ov-stat.hl .v{color:var(--acc)}
.so-ov-btn{appearance:none;border:0;border-radius:12px;padding:13px 34px;cursor:pointer;
  font:800 15px Sora;letter-spacing:.02em;color:#06251b;background:linear-gradient(135deg,#5ef2c0,#37c493);
  box-shadow:0 8px 26px rgba(94,242,192,.28);transition:transform .12s,box-shadow .12s}
.so-ov-btn:hover{transform:translateY(-2px);box-shadow:0 12px 32px rgba(94,242,192,.38)}
.so-ov-btn:active{transform:translateY(0)}
.so-key{border:1px solid var(--line2);border-bottom-width:3px;border-radius:8px;padding:5px 11px;
  font:700 11.5px "JetBrains Mono",monospace;color:#cdd9ea;background:rgba(0,0,0,.35)}
.so-toast{position:absolute;left:50%;bottom:14px;transform:translateX(-50%) translateY(8px);
  background:rgba(6,12,20,.92);border:1px solid var(--line2);border-radius:10px;
  padding:9px 16px;font:600 12.5px Sora;color:#dbe4f0;opacity:0;transition:opacity .2s,transform .2s;
  pointer-events:none;z-index:150;max-width:88%;text-align:center}
.so-toast.show{opacity:1;transform:translateX(-50%) translateY(0)}

/* ---------- Auto-finish callout ---------- */
.so-auto{position:absolute;left:50%;top:14px;transform:translateX(-50%);z-index:120;
  display:flex;align-items:center;gap:10px;
  background:rgba(6,12,20,.92);border:1px solid rgba(94,242,192,.45);border-radius:12px;
  padding:8px 10px 8px 16px;box-shadow:0 10px 30px -10px rgba(0,0,0,.6)}
.so-auto.hidden{display:none}
.so-auto .txt{font:700 12.5px Sora;color:#eaf7f1;letter-spacing:.02em}
.so-auto button{appearance:none;border:0;border-radius:9px;padding:8px 16px;cursor:pointer;
  font:800 12.5px Sora;color:#06251b;background:linear-gradient(135deg,#5ef2c0,#37c493)}
.so-auto button:hover{filter:brightness(1.07)}
.so-auto.pulse{animation:soautopulse 1.6s ease-in-out infinite}
@keyframes soautopulse{0%,100%{box-shadow:0 10px 30px -10px rgba(0,0,0,.6)}50%{box-shadow:0 10px 34px -8px rgba(94,242,192,.5)}}

/* ---------- Win cascade ---------- */
.so-card.cascade{transition:none !important;pointer-events:none}

/* ---------- Fullscreen / immersive ---------- */
body.so-fs #soPanel,body.ft-playing.ft-immersive #soPanel{
  max-width:none;width:100vw;height:100vh;margin:0;border-radius:0;border:none;overflow:hidden;
  padding:clamp(6px,1.2vh,14px) clamp(10px,2vw,24px);box-sizing:border-box;justify-content:flex-start;
  background:var(--bg,#0a0d13);display:flex;flex-direction:column;
}
body.so-fs #soPanel .ft-panel-h,body.ft-playing.ft-immersive #soPanel .ft-panel-h{display:none}
body.so-fs .so-table,body.ft-playing.ft-immersive .so-table{
  flex:1;min-height:0;display:flex;flex-direction:column;margin:8px 0 0;
}
body.so-fs .so-board,body.ft-playing.ft-immersive .so-board{flex:1;min-height:0}
body.so-fs .so-hud,body.ft-playing.ft-immersive .so-hud{padding:0}
body.so-fs .so-controls,body.ft-playing.ft-immersive .so-controls{padding:0}
body.ft-playing #soPanel{margin:0 auto;width:100%;max-width:min(1180px,100%)}

/* ---------- Responsive ---------- */
@media (max-width:760px){
  .so-row{gap:4px}
  .so-label{display:none}
  .so-help{display:none}
  .so-hud{padding:0 12px;gap:6px}
  .so-stat{padding:3px 8px}
  .so-stat .v{font-size:12px}
  .so-controls{padding:0 12px}
  .so-controls .ft-btn{padding:5px 9px;font-size:10.5px}
  .so-table{margin:5px 10px 10px}
}
@media (prefers-reduced-motion:reduce){
  .so-card{transition:none !important}
  .so-slot.hint,.so-card.hint-card,.so-auto.pulse{animation:none}
}
`;
  document.head.appendChild(st);
})();

/* ---- State ------------------------------------------------------------- */
const SOG = {
  epoch: 0, cleanup: [],
  game: null,                 // the engine state (SolitaireEngine.createGame)
  els: new Map(),             // card id -> element
  pos: new Map(),             // card id -> {x,y} last painted
  plan: null,                 // geometry plan from the last soPlan()
  selected: null,             // {pile:'tableau'|'waste'|'foundation', index, cardId}
  status: 'ready',            // ready | playing | paused | won
  startedAt: 0, elapsedMs: 0, timerId: 0,
  hintTimer: 0, toastTimer: 0, hintIdx: 0,
  reducedMotion: false,
  dealOrder: null,            // card objects in TRUE initial deal order
  won: false,
  autoTimer: 0,               // auto-finish stepper interval
  cascade: null,              // win animation state
  autoToastShown: false,
};
const SOS = { on: false, ctx: null, master: null };   // sound

/* ---- Helpers ------------------------------------------------------------ */
function so$(id) { return document.getElementById(id); }
function soOn(target, ev, fn, opts) {
  target.addEventListener(ev, fn, opts);
  SOG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
}
function soFmtTime(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return String(Math.floor(s / 60)).padStart(2, '0') + ':' + String(s % 60).padStart(2, '0');
}
const SO_SUITS = { S: '\u2660', H: '\u2665', D: '\u2666', C: '\u2663' };
const SO_SUIT_NAMES = { S: 'Spades', H: 'Hearts', D: 'Diamonds', C: 'Clubs' };
const SO_RANKS = { 1: 'A', 11: 'J', 12: 'Q', 13: 'K' };
function soRankText(r) { return SO_RANKS[r] || String(r); }

/* Classic pip arrangements, [x%, y%] inside the pip field.
   Bottom-half pips rotate 180° like a real deck. */
const SO_PIPS = {
  2:  [[50, 10], [50, 90]],
  3:  [[50, 10], [50, 50], [50, 90]],
  4:  [[18, 10], [82, 10], [18, 90], [82, 90]],
  5:  [[18, 10], [82, 10], [50, 50], [18, 90], [82, 90]],
  6:  [[18, 10], [82, 10], [18, 50], [82, 50], [18, 90], [82, 90]],
  7:  [[18, 10], [82, 10], [50, 30], [18, 50], [82, 50], [18, 90], [82, 90]],
  8:  [[18, 10], [82, 10], [50, 30], [18, 50], [82, 50], [50, 70], [18, 90], [82, 90]],
  9:  [[18, 8], [82, 8], [18, 36], [82, 36], [50, 50], [18, 64], [82, 64], [18, 92], [82, 92]],
  10: [[18, 8], [82, 8], [50, 22], [18, 36], [82, 36], [18, 64], [82, 64], [50, 78], [18, 92], [82, 92]],
};

/* ---- Sound (lightweight WebAudio, off by default like the arcade) ------ */
function soAudioInit() {
  if (SOS.ctx) return true;
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return false;
  try {
    SOS.ctx = new AC();
    SOS.master = SOS.ctx.createGain();
    SOS.master.gain.value = 0.28;
    SOS.master.connect(SOS.ctx.destination);
    return true;
  } catch { SOS.ctx = null; return false; }
}
function soTone(freq, dur, vol, type) {
  if (!SOS.on || !SOS.ctx || !freq) return;
  const t = SOS.ctx.currentTime;
  const osc = SOS.ctx.createOscillator();
  const gain = SOS.ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(vol, t + 0.008);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(SOS.master);
  osc.start(t); osc.stop(t + dur + 0.02);
}
function soSfx(kind) {
  if (!SOS.on) return;
  switch (kind) {
    case 'draw':   soTone(520, .05, .10, 'triangle'); break;
    case 'place':  soTone(300, .05, .12, 'triangle');
                   setTimeout(() => soTone(220, .04, .07, 'sine'), 35); break;
    case 'flip':   soTone(660, .04, .08, 'sine'); break;
    case 'invalid':soTone(140, .12, .10, 'square'); break;
    case 'found':  soTone(660, .07, .10, 'sine');
                   setTimeout(() => soTone(880, .09, .10, 'sine'), 70); break;
    case 'recycle':soTone(240, .07, .08, 'sawtooth');
                   setTimeout(() => soTone(180, .07, .07, 'sawtooth'), 60); break;
    case 'win':    [523, 659, 784, 1047].forEach((f, i) =>
                     setTimeout(() => soTone(f, .14, .12), i * 110)); break;
    case 'ui':     soTone(880, .04, .06); break;
  }
}
function soSoundToggle() {
  SOS.on = !SOS.on;
  if (SOS.on && !soAudioInit()) SOS.on = false;
  else if (SOS.on && SOS.ctx.state === 'suspended') SOS.ctx.resume().catch(() => {});
  soSoundButton();
}
function soSoundButton() {
  const btn = so$('soSoundBtn'); if (!btn) return;
  btn.setAttribute('aria-pressed', String(SOS.on));
  btn.classList.toggle('on', SOS.on);
  const lbl = so$('soSoundLbl'); if (lbl) lbl.textContent = SOS.on ? 'sound on' : 'sound off';
}

/* =========================================================================
   GEOMETRY — one planning pass decides everything before painting.
   ========================================================================= */

/** Measure the table. Card size comes from the slot box the grid chose,
 *  so slots and cards can never disagree. */
function soLayout() {
  const board = so$('soBoard'); if (!board) return null;
  const slot = so$('soStock'); if (!slot) return null;
  const br = board.getBoundingClientRect();
  const sr = slot.getBoundingClientRect();
  const cardW = sr.width, cardH = sr.height;
  if (cardW < 10 || cardH < 10) return null;      // grid not ready yet
  const rad = Math.max(6, Math.round(cardW * 0.11));
  board.style.setProperty('--soCardW', cardW + 'px');
  board.style.setProperty('--soCardH', cardH + 'px');
  board.style.setProperty('--soRad', rad + 'px');
  return { board, br, cardW, cardH, rad, stockX: sr.left - br.left, stockY: sr.top - br.top };
}

/** Natural (uncompressed) stacking offsets — compact: face-down cards
 *  peek less than face-up ones, keeping columns readable but short. */
function soNaturalOffsets(cardH) {
  return {
    down: Math.max(5, Math.round(cardH * 0.115)),
    up:   Math.max(10, Math.round(cardH * 0.24)),
  };
}

/**
 * Build the full geometry plan: every card's x/y/z.
 *
 * Tableau columns compress when they would overflow the board: each
 * column's offsets shrink proportionally (with sane floors) so the whole
 * column stays visible — the classic behaviour of professional tables.
 */
function soPlan(L) {
  const g = SOG.game; if (!g || !L) return null;
  const br = L.br;
  const rectOf = id => {
    const el = so$(id); if (!el) return null;
    const r = el.getBoundingClientRect();
    if (r.width < 2) return null;
    return { x: r.left - br.left, y: r.top - br.top, w: r.width, h: r.height };
  };

  const stock = rectOf('soStock');
  const waste = rectOf('soWaste');
  const foundations = [0, 1, 2, 3].map(i => rectOf('soF' + i));
  const tabs = [0, 1, 2, 3, 4, 5, 6].map(i => rectOf('soT' + i));
  if (!stock || !waste || foundations.some(f => !f) || tabs.some(t => !t)) return null;

  // --- board height management -----------------------------------------
  // Inline: the board grows with the tallest natural column (page scrolls).
  // Fullscreen: height is fixed by flex, so columns compress to fit.
  const nat = soNaturalOffsets(L.cardH);
  const topRowBottom = tabs[0].y;
  const naturalNeeded = Math.max(...g.tableau.map(col => {
    let acc = 0;
    for (let i = 0; i < col.length - 1; i++) acc += col[i].faceUp ? nat.up : nat.down;
    return acc;
  }), nat.up * 2);
  const fs = document.body.classList.contains('so-fs')
          || document.body.classList.contains('ft-immersive');
  if (!fs) {
    const pad = parseFloat(getComputedStyle(L.board).paddingBottom || '0');
    const inlineH = Math.round(topRowBottom + L.cardH + Math.min(naturalNeeded, L.cardH * 2.6) + pad);
    L.board.style.height = '';
    L.board.style.minHeight = Math.max(340, inlineH) + 'px';
  } else {
    L.board.style.height = '';
    L.board.style.minHeight = '';
  }
  const boardH = L.board.getBoundingClientRect().height;
  const minStackSpace = nat.up * 2;
  const availStack = Math.max(minStackSpace, boardH - topRowBottom - L.cardH - 4);

  // per-column cumulative offsets (compressed to fit)
  const colY = g.tableau.map(col => {
    const ys = [0];
    let needed = 0;
    for (let i = 0; i < col.length - 1; i++) needed += col[i].faceUp ? nat.up : nat.down;
    let scale = 1;
    if (needed > availStack && needed > 0) scale = availStack / needed;
    const down = Math.max(4, Math.round(nat.down * scale));
    const up =   Math.max(9, Math.round(nat.up * scale));
    for (let i = 0; i < col.length - 1; i++) {
      ys.push(ys[i] + (col[i].faceUp ? up : down));
    }
    return ys;
  });

  // waste fan: newest three cards spread horizontally (compact)
  const fanX = Math.max(8, Math.round(L.cardW * 0.18));

  const pos = new Map();
  const put = (card, x, y, z) => pos.set(card.id, { x, y, z });

  g.stock.forEach((c, i) =>
    put(c, stock.x + Math.min(i * 0.3, 4), stock.y - Math.min(i * 0.22, 3), 2 + i));
  g.waste.forEach((c, i) => {
    const fi = Math.max(0, i - (g.waste.length - 3));
    put(c, waste.x + fi * fanX, waste.y, 2 + i);
  });
  g.foundations.forEach((f, fi) => f.forEach((c, i) =>
    put(c, foundations[fi].x, foundations[fi].y, 2 + i)));
  g.tableau.forEach((col, ci) => col.forEach((c, i) =>
    put(c, tabs[ci].x, tabs[ci].y + colY[ci][i], 2 + i)));

  return { pos, boardH, availStack, fanX };
}

/* ---- Card elements ------------------------------------------------------ */

/** Build a card element with a classic face: corner indices, pip field
 *  (2–10), oversized Ace pip, or a framed court monogram. */
function soCardEl(card) {
  let el = SOG.els.get(card.id);
  if (el) return el;
  el = document.createElement('div');
  el.className = 'so-card ' + (card.red ? 'red' : 'black');
  el.dataset.id = card.id;
  const r = soRankText(card.rank), s = SO_SUITS[card.suit];

  let centre;
  if (card.rank === 1) {
    centre = `<span class="ace">${s}</span>`;
  } else if (card.rank > 10) {
    centre = `<span class="court"><span class="letter">${r}</span><span class="csuit">${s}</span></span>`;
  } else {
    const pips = SO_PIPS[card.rank]
      .map(([x, y]) => `<span class="pip${y > 50 ? ' flip' : ''}" style="left:${x}%;top:${y}%">${s}</span>`)
      .join('');
    centre = `<span class="pips">${pips}</span>`;
  }
  const wide = card.rank === 10 ? ' wide' : '';
  el.innerHTML =
    `<div class="face">
       <span class="idx tl"><span class="r${wide}">${r}</span><span class="s">${s}</span></span>
       ${centre}
       <span class="idx br"><span class="r${wide}">${r}</span><span class="s">${s}</span></span>
     </div><div class="back"></div>`;
  el.setAttribute('role', 'button');
  el.setAttribute('aria-label', `${r === '10' ? 'Ten' : r} of ${SO_SUIT_NAMES[card.suit]}`);
  SOG.els.set(card.id, el);
  return el;
}

/** Paint every card from the current plan. Writes only --tx/--ty, face
 *  classes and z-index; CSS transitions animate the rest. */
function soPaint(instant) {
  const L = soLayout(); if (!L || !SOG.game) return;
  const plan = soPlan(L); if (!plan) return;
  SOG.plan = plan;
  const board = L.board;
  const place = (card) => {
    const el = soCardEl(card);
    if (el.parentNode !== board) board.appendChild(el);
    const p = plan.pos.get(card.id); if (!p) return;
    el.style.setProperty('--tx', p.x + 'px');
    el.style.setProperty('--ty', p.y + 'px');
    el.classList.toggle('facedown', !card.faceUp);
    el.classList.toggle('faceup', !!card.faceUp);
    el.style.zIndex = String(p.z);
    SOG.pos.set(card.id, { x: p.x, y: p.y });
    if (instant) {
      el.style.transition = 'none';
      requestAnimationFrame(() => { el.style.transition = ''; });
    }
  };
  SOG.game.stock.forEach(place);
  SOG.game.waste.forEach(place);
  SOG.game.foundations.forEach(f => f.forEach(place));
  SOG.game.tableau.forEach(t => t.forEach(place));

  const g = SOG.game;
  const stockEl = so$('soStock');
  if (stockEl) stockEl.classList.toggle('stock-empty', g.stock.length === 0 && g.waste.length > 0);
  soHud();
}

/* ---- HUD ---------------------------------------------------------------- */
function soHud() {
  const g = SOG.game; if (!g) return;
  const sc = so$('soScore'), mv = so$('soMoves'), tm = so$('soTime'), fd = so$('soFound');
  const onF = g.foundations.reduce((n, f) => n + f.length, 0);
  if (sc) sc.textContent = String(g.score);
  if (mv) mv.textContent = String(g.moves);
  if (tm) tm.textContent = soFmtTime(SOG.elapsedMs);
  if (fd) fd.textContent = onF + '/52';
  const undo = so$('soUndoBtn'); if (undo) undo.disabled = g.history.length === 0;
  soMaybeAutoCallout();
}

/** All that remains is pushing cards to the foundations? Offer the
 *  one-click Auto-Finish every professional table has. */
function soAutoAvailable() {
  const g = SOG.game; if (!g || SOG.status === 'won' || SOG.status === 'paused') return false;
  if (g.stock.length || g.waste.length) return false;
  for (const col of g.tableau) for (const c of col) if (!c.faceUp) return false;
  return g.foundations.reduce((n, f) => n + f.length, 0) < 52;
}
function soMaybeAutoCallout() {
  const box = so$('soAutoBox'); if (!box) return;
  const show = soAutoAvailable() && !SOG.autoTimer;
  box.classList.toggle('hidden', !show);
  if (show && !SOG.autoToastShown) {
    SOG.autoToastShown = true;
    soToast('All cards are face up — Auto-Finish available');
  }
}

/* ---- Timer -------------------------------------------------------------- */
function soTimerStart() {
  if (SOG.timerId) return;
  SOG.timerId = setInterval(() => {
    if (SOG.status !== 'playing') return;
    SOG.elapsedMs = Date.now() - SOG.startedAt;
    const tm = so$('soTime'); if (tm) tm.textContent = soFmtTime(SOG.elapsedMs);
  }, 500);
}
function soTimerStop() { if (SOG.timerId) { clearInterval(SOG.timerId); SOG.timerId = 0; } }
function soTouch() {
  if (SOG.status === 'ready') {
    SOG.status = 'playing';
    SOG.startedAt = Date.now() - SOG.elapsedMs;
    soTimerStart();
  }
}

/* ---- Toast -------------------------------------------------------------- */
function soToast(msg) {
  const el = so$('soToast'); if (!el) return;
  el.textContent = msg; el.classList.add('show');
  clearTimeout(SOG.toastTimer);
  SOG.toastTimer = setTimeout(() => el.classList.remove('show'), 2400);
}

/* ---- Dealing animation --------------------------------------------------- */
function soDealAnimation() {
  const g = SOG.game;
  const board = so$('soBoard');
  if (!board || SOG.reducedMotion) { soPaint(true); return; }
  soPaint(true);
  const L = soLayout(); if (!L) return;

  // Start every card stacked on the stock, then deal row by row.
  const all = [...g.tableau.flat(), ...g.stock];
  all.forEach(c => {
    const el = soCardEl(c);
    el.style.transition = 'none';
    el.style.setProperty('--tx', L.stockX + 'px');
    el.style.setProperty('--ty', L.stockY + 'px');
  });
  void board.offsetWidth;                          // force layout

  // Stagger: row-major like a real dealer, ~60ms per row.
  const order = [];
  g.tableau.forEach((col, ci) => col.forEach((c, ri) => order.push({ c, ri, ci })));
  order.sort((a, b) => a.ri - b.ri || a.ci - b.ci);
  order.forEach(({ c, ri }) => {
    const el = soCardEl(c);
    el.style.transition = '';
    el.style.transitionDelay = (ri * 60) + 'ms';
  });
  soPaint(false);
  const total = order.length ? (Math.max(...order.map(o => o.ri)) + 1) * 60 + 350 : 350;
  setTimeout(() => {
    SOG.els.forEach(el => { el.style.transitionDelay = ''; });
  }, total);
}

/* ---- Selection & moves -------------------------------------------------- */
function soClearSelection() {
  SOG.selected = null;
  SOG.els.forEach(el => el.classList.remove('selected'));
  document.querySelectorAll('.so-slot.drop-ok').forEach(s => s.classList.remove('drop-ok'));
}

/** Which pile+index does a card belong to right now? */
function soLocate(cardId) {
  const g = SOG.game;
  for (let i = 0; i < 7; i++) {
    const j = g.tableau[i].findIndex(c => c.id === cardId);
    if (j >= 0) return { pile: 'tableau', index: i, pos: j };
  }
  const w = g.waste.findIndex(c => c.id === cardId);
  if (w >= 0) return { pile: 'waste', pos: w };
  for (let f = 0; f < 4; f++) {
    const j = g.foundations[f].findIndex(c => c.id === cardId);
    if (j >= 0) return { pile: 'foundation', index: f, pos: j };
  }
  const s = g.stock.findIndex(c => c.id === cardId);
  if (s >= 0) return { pile: 'stock', pos: s };
  return null;
}
function soCardById(id) {
  const g = SOG.game;
  for (const c of g.tableau.flat()) if (c.id === id) return c;
  for (const c of g.waste) if (c.id === id) return c;
  for (const c of g.foundations.flat()) if (c.id === id) return c;
  for (const c of g.stock) if (c.id === id) return c;
  return null;
}

/** Mark legal drop targets for the current selection. */
function soShowDropTargets(sel) {
  const E = window.SolitaireEngine, g = SOG.game;
  document.querySelectorAll('.so-slot.drop-ok').forEach(s => s.classList.remove('drop-ok'));
  if (!sel) return;
  const card = soCardById(sel.cardId); if (!card) return;
  for (let c = 0; c < 7; c++) {
    if (sel.pile === 'tableau' && sel.index === c) continue;
    if (E.canMoveToTableau(g.tableau[c], card)) {
      const el = so$('soT' + c); if (el) el.classList.add('drop-ok');
    }
  }
  for (let f = 0; f < 4; f++) {
    if (sel.pile === 'foundation' && sel.index === f) continue;
    if (E.canMoveToFoundation(g.foundations[f], card)) {
      const el = so$('soF' + f); if (el) el.classList.add('drop-ok');
    }
  }
}

/** Run an engine move, repaint, handle side effects. Returns success. */
function soApplyMove(fn, sfx) {
  const ok = fn();
  if (!ok) { soSfx('invalid'); return false; }
  soTouch();
  soPaint(false);
  soClearSelection();
  if (sfx) soSfx(sfx);
  if (window.SolitaireEngine.checkWin(SOG.game)) soWin();
  return true;
}

/** Click handling for a card. */
function soCardClick(cardId) {
  const E = window.SolitaireEngine, g = SOG.game;
  const loc = soLocate(cardId); if (!loc) return;
  const card = soCardById(cardId);

  // Stock top → draw (also handled by the slot, but cards cover it)
  if (loc.pile === 'stock') { soStockClick(); return; }

  // Face-down cards are not interactive except the stock
  if (!card.faceUp) return;

  // Second click on the same card → smart move (foundation first)
  if (SOG.selected && SOG.selected.cardId === cardId) {
    if (soTrySmartMove(loc, card)) return;
    soClearSelection();
    return;
  }

  // Clicking a destination while something is selected → attempt move
  if (SOG.selected) {
    if (soAttemptDrop(loc)) return;
  }

  // Select this card (or the run starting at it)
  if (loc.pile === 'waste') {
    if (loc.pos !== g.waste.length - 1) return;   // only the top waste card
    SOG.selected = { pile: 'waste', cardId };
  } else if (loc.pile === 'foundation') {
    if (loc.pos !== g.foundations[loc.index].length - 1) return;
    SOG.selected = { pile: 'foundation', index: loc.index, cardId };
  } else {
    const col = g.tableau[loc.index];
    if (!E.isValidRun(col, loc.pos)) return;      // not a movable sequence
    SOG.selected = { pile: 'tableau', index: loc.index, cardId };
  }
  const sel = SOG.selected;
  soClearSelection();
  SOG.selected = sel;
  const el = SOG.els.get(cardId); if (el) el.classList.add('selected');
  soShowDropTargets(SOG.selected);
  soSfx('ui');
}

/**
 * Smart move: where would this card most usefully go?
 * Foundation first (the goal), otherwise the best tableau destination —
 * preferring a non-empty column, then one that uncovers a face-down card.
 */
function soTrySmartMove(loc, card) {
  const E = window.SolitaireEngine, g = SOG.game;
  if (loc.pile === 'foundation') return false;
  if (loc.pile === 'tableau' && loc.pos !== g.tableau[loc.index].length - 1) {
    // A run click: try moving the whole run to another column.
    const col = g.tableau[loc.index];
    const count = col.length - loc.pos;
    if (count > 1) {
      for (let d = 0; d < 7; d++) {
        if (d === loc.index) continue;
        if (E.canMoveToTableau(g.tableau[d], card)
            && (g.tableau[d].length > 0 || count < col.length)) {
          return soApplyMove(
            () => E.moveTableauToTableau(g, loc.index, d, count), 'place');
        }
      }
    }
    return false;
  }
  // Single top card: foundation first.
  for (let f = 0; f < 4; f++) {
    if (E.canMoveToFoundation(g.foundations[f], card)) {
      const done = soApplyMove(
        () => loc.pile === 'waste'
          ? E.moveWasteTo(g, { kind: 'foundation', index: f })
          : E.moveTableauToFoundation(g, loc.index, f),
        'found');
      if (done) return true;
    }
  }
  return false;
}

/** A selected card was clicked onto pile `loc`. Returns true if moved. */
function soAttemptDrop(dest) {
  const E = window.SolitaireEngine, g = SOG.game;
  const sel = SOG.selected; if (!sel) return false;
  const card = soCardById(sel.cardId); if (!card) return false;

  if (dest.pile === 'waste' || dest.pile === 'stock') return false;

  if (dest.pile === 'foundation') {
    if (sel.pile === 'waste') {
      return soApplyMove(() => E.moveWasteTo(g, { kind: 'foundation', index: dest.index }), 'found');
    }
    if (sel.pile === 'tableau') {
      return soApplyMove(() => E.moveTableauToFoundation(g, sel.index, dest.index), 'found');
    }
    return false;
  }

  if (dest.pile === 'tableau') {
    if (sel.pile === 'waste') {
      return soApplyMove(() => E.moveWasteTo(g, { kind: 'tableau', index: dest.index }), 'place');
    }
    if (sel.pile === 'tableau') {
      const from = g.tableau[sel.index];
      const pos = from.findIndex(c => c.id === sel.cardId);
      if (pos < 0) return false;
      const count = from.length - pos;
      return soApplyMove(
        () => E.moveTableauToTableau(g, sel.index, dest.index, count), 'place');
    }
    if (sel.pile === 'foundation') {
      return soApplyMove(() => E.moveFoundationToTableau(g, sel.index, dest.index), 'place');
    }
  }
  return false;
}

/** Stock click: draw, or recycle when exhausted. */
function soStockClick() {
  const E = window.SolitaireEngine, g = SOG.game;
  if (g.status === 'won') return;
  if (g.stock.length > 0) {
    soApplyMove(() => E.drawFromStock(g), 'draw');
  } else if (g.waste.length > 0) {
    soApplyMove(() => E.recycleWaste(g), 'recycle');
  } else {
    soToast('Stock and waste are both empty');
  }
}

/* ---- Drag & drop (pointer events; mouse + touch) ----------------------- */
function soBindDrag() {
  const board = so$('soBoard'); if (!board) return;
  let drag = null;   // {ids, startX, startY, moved, loc, base:[{el,tx,ty}]}

  soOn(board, 'pointerdown', e => {
    if (SOG.status === 'won') return;
    const cardEl = e.target.closest('.so-card');
    if (!cardEl || !cardEl.dataset.id) return;
    if (e.button !== undefined && e.button !== 0) return;
    const id = cardEl.dataset.id;
    const loc = soLocate(id);
    if (!loc) return;
    const g = SOG.game, E = window.SolitaireEngine;
    if (loc.pile === 'stock') return;                    // stock is click-only
    const card = soCardById(id);
    if (!card || !card.faceUp) return;
    if (loc.pile === 'waste' && loc.pos !== g.waste.length - 1) return;
    if (loc.pile === 'foundation' && loc.pos !== g.foundations[loc.index].length - 1) return;
    if (loc.pile === 'tableau' && !E.isValidRun(g.tableau[loc.index], loc.pos)) return;

    // the run being dragged (tableau) or the single card
    const ids = [];
    if (loc.pile === 'tableau') {
      g.tableau[loc.index].slice(loc.pos).forEach(c => ids.push(c.id));
    } else ids.push(id);

    drag = { ids, startX: e.clientX, startY: e.clientY, moved: false, loc,
             base: ids.map(cid => {
               const el = SOG.els.get(cid);
               return { el, tx: parseFloat(el.style.getPropertyValue('--tx')) || 0,
                        ty: parseFloat(el.style.getPropertyValue('--ty')) || 0 };
             }) };
    try { board.setPointerCapture(e.pointerId); } catch { /* fine */ }
    e.preventDefault();
  });

  soOn(board, 'pointermove', e => {
    if (!drag) return;
    const dx = e.clientX - drag.startX, dy = e.clientY - drag.startY;
    if (!drag.moved) {
      if (Math.abs(dx) < 4 && Math.abs(dy) < 4) return;
      drag.moved = true;
      drag.base.forEach(b => b.el.classList.add('dragging'));
      soShowDropTargets({ pile: drag.loc.pile, index: drag.loc.index, cardId: drag.ids[0] });
    }
    drag.base.forEach((b, i) => {
      b.el.style.setProperty('--tx', (b.tx + dx) + 'px');
      b.el.style.setProperty('--ty', (b.ty + dy) + 'px');
      b.el.style.zIndex = String(1000 + i);
    });
  });

  const finish = e => {
    if (!drag) return;
    const d = drag; drag = null;
    d.base.forEach(b => b.el.classList.remove('dragging'));
    if (!d.moved) {
      soPaint(false);                 // plain tap — click handler selects
      return;
    }
    // Hide the dragged cards for the hit-test: they sit under the cursor
    // at z-index 999, so elementFromPoint would otherwise return the card
    // being dragged instead of the pile beneath it.
    d.base.forEach(b => { b.el.style.visibility = 'hidden'; });
    const el = document.elementFromPoint(e.clientX, e.clientY);
    d.base.forEach(b => { b.el.style.visibility = ''; });
    const slot = el && (el.closest('.so-slot') || el.closest('.so-card'));
    let handled = false;
    if (slot) {
      const dest = soSlotFromElement(slot);
      if (dest) {
        const g = SOG.game, E = window.SolitaireEngine;
        const card = soCardById(d.ids[0]);
        if (dest.pile === 'tableau') {
          const from = d.loc.pile === 'tableau' ? g.tableau[d.loc.index] : null;
          const count = from ? from.length - from.findIndex(c => c.id === d.ids[0]) : 1;
          handled = soApplyMove(() => {
            if (d.loc.pile === 'waste') return E.moveWasteTo(g, { kind: 'tableau', index: dest.index });
            if (d.loc.pile === 'foundation') return E.moveFoundationToTableau(g, d.loc.index, dest.index);
            return E.moveTableauToTableau(g, d.loc.index, dest.index, count);
          }, 'place');
        } else if (dest.pile === 'foundation') {
          handled = soApplyMove(() => {
            if (d.loc.pile === 'waste') return E.moveWasteTo(g, { kind: 'foundation', index: dest.index });
            if (d.loc.pile === 'tableau') return E.moveTableauToFoundation(g, d.loc.index, dest.index);
            return false;
          }, 'found');
        }
      }
    }
    if (!handled) soSfx('invalid');
    soPaint(false);
    soClearSelection();
  };
  soOn(board, 'pointerup', finish);
  soOn(board, 'pointercancel', finish);
}

/** Map an element under the pointer to a destination pile. */
function soSlotFromElement(el) {
  const slot = el.closest('.so-slot');
  if (slot) {
    if (slot.id === 'soStock' || slot.id === 'soWaste') return null;
    const m = /^soF(\d)$/.exec(slot.id);
    if (m) return { pile: 'foundation', index: +m[1] };
    const t = /^soT(\d)$/.exec(slot.id);
    if (t) return { pile: 'tableau', index: +t[1] };
    return null;
  }
  const card = el.closest('.so-card');
  if (card && card.dataset.id) {
    const loc = soLocate(card.dataset.id);
    if (!loc) return null;
    if (loc.pile === 'tableau') return { pile: 'tableau', index: loc.index };
    if (loc.pile === 'foundation') return { pile: 'foundation', index: loc.index };
  }
  return null;
}

/* ---- Hint ---------------------------------------------------------------- */
function soHint() {
  const E = window.SolitaireEngine, g = SOG.game;
  if (g.status === 'won') return;
  const h = E.getHint(g);
  soToast(h.text);
  if (!h.move) return;
  const src = soCardById(h.move.card.id);
  if (src) {
    const el = SOG.els.get(src.id);
    if (el) { el.classList.remove('hint-card'); void el.offsetWidth; el.classList.add('hint-card'); }
  }
  const slot = soSlotElFor(h.move.to);
  if (slot) { slot.classList.remove('hint'); void slot.offsetWidth; slot.classList.add('hint'); }
  clearTimeout(SOG.hintTimer);
  SOG.hintTimer = setTimeout(() => {
    document.querySelectorAll('.so-slot.hint').forEach(s => s.classList.remove('hint'));
    document.querySelectorAll('.so-card.hint-card').forEach(s => s.classList.remove('hint-card'));
  }, 2500);
}
function soSlotElFor(to) {
  if (!to) return null;
  if (to.pile === 'foundation') return so$('soF' + to.index);
  if (to.pile === 'tableau') return so$('soT' + to.index);
  return null;
}

/* ---- Auto-Finish --------------------------------------------------------- */
/** Play every remaining foundation move with a pleasant cadence. */
function soAutoFinish() {
  const E = window.SolitaireEngine, g = SOG.game;
  if (!soAutoAvailable()) return;
  soTouch();
  const box = so$('soAutoBox'); if (box) box.classList.add('hidden');
  SOG.autoTimer = setInterval(() => {
    // find any tableau top that can go up
    let moved = false;
    outer:
    for (let c = 0; c < 7; c++) {
      const col = g.tableau[c];
      if (!col.length) continue;
      const card = col[col.length - 1];
      for (let f = 0; f < 4; f++) {
        if (E.canMoveToFoundation(g.foundations[f], card)) {
          E.moveTableauToFoundation(g, c, f);
          soPaint(false);
          soSfx('found');
          moved = true;
          break outer;
        }
      }
    }
    if (!moved || E.checkWin(g)) {
      clearInterval(SOG.autoTimer);
      SOG.autoTimer = 0;
      if (E.checkWin(g)) soWin();
    }
  }, 120);
}

/* ---- Win: the classic cascading cards ------------------------------------ */
/** Launch the iconic bounce-cascade: each foundation card gets physics
 *  (gravity, horizontal velocity, floor bounce) on a rAF loop. */
function soWin() {
  if (SOG.won) return;
  SOG.won = true;
  SOG.status = 'won';
  soTimerStop();
  const g = SOG.game;
  const box = so$('soAutoBox'); if (box) box.classList.add('hidden');
  soSfx('win');

  const L = soLayout();
  if (!SOG.reducedMotion && L) {
    soCascade(L);
  } else {
    setTimeout(() => soShowWinOverlay(), 400);
  }
}

function soCascade(L) {
  const board = L.board;
  const br = L.board.getBoundingClientRect();
  const cards = [];
  // Release order: top of each foundation in turn (K down to A per suit).
  const queue = [];
  for (let round = 0; round < 13; round++) {
    for (let f = 0; f < 4; f++) {
      const pile = SOG.game.foundations[f];
      const idx = pile.length - 1 - round;
      if (idx >= 0) queue.push(pile[idx]);
    }
  }
  queue.forEach((card, i) => {
    const el = SOG.els.get(card.id);
    if (!el) return;
    const p = SOG.pos.get(card.id) || { x: 0, y: 0 };
    cards.push({
      el, x: p.x, y: p.y,
      vx: (i % 2 === 0 ? 1 : -1) * (1.6 + Math.random() * 2.4),
      vy: -2 - Math.random() * 3,
      delay: i * 90,
      t0: performance.now(),
      done: false,
    });
    el.classList.add('cascade');
  });

  const G = 0.45;                    // gravity
  const FLOOR = br.height - L.cardH - 4;
  let live = cards.length;
  const step = now => {
    for (const c of cards) {
      if (c.done) continue;
      if (now - c.t0 < c.delay) continue;
      c.vy += G;
      c.x += c.vx;
      c.y += c.vy;
      if (c.y >= FLOOR) {            // bounce: lose energy each hit
        c.y = FLOOR;
        c.vy = -Math.abs(c.vy) * 0.72;
        if (Math.abs(c.vy) < 2.2) { c.vy = 0; c.done = true; live--; }
      }
      c.el.style.setProperty('--tx', c.x + 'px');
      c.el.style.setProperty('--ty', c.y + 'px');
      // keep bouncers above settled cards
      c.el.style.zIndex = String(400 + Math.round(Math.max(0, FLOOR - c.y)));
    }
    if (live > 0) {
      SOG.cascade = requestAnimationFrame(step);
    } else {
      SOG.cascade = null;
      setTimeout(() => soShowWinOverlay(), 500);
    }
  };
  SOG.cascade = requestAnimationFrame(step);
}

function soShowWinOverlay() {
  const ov = so$('soOverlay'); if (!ov) return;
  const g = SOG.game;
  so$('soOvTitle').textContent = 'You Win!';
  so$('soOvSub').innerHTML = 'All 52 cards reached the foundations.';
  so$('soOvStats').classList.remove('hidden');
  so$('soRsScore').textContent = String(g.score);
  so$('soRsTime').textContent = soFmtTime(SOG.elapsedMs);
  so$('soRsMoves').textContent = String(g.moves);
  so$('soOvBtn').textContent = 'Play Again';
  so$('soOvBtn').dataset.kind = 'again';
  ov.classList.remove('hidden');
}

/* ---- Game lifecycle ------------------------------------------------------- */
function soNewGame() {
  const E = window.SolitaireEngine;
  soStopCascade();
  if (SOG.autoTimer) { clearInterval(SOG.autoTimer); SOG.autoTimer = 0; }
  SOG.els.forEach(el => el.remove());
  SOG.els.clear();
  SOG.pos.clear();
  SOG.game = E.createGame({ drawCount: 1 });
  // True initial deal order, captured BEFORE any move can mutate piles.
  // Restart replays exactly this.
  SOG.dealOrder = [...SOG.game.tableau.flat(), ...SOG.game.stock];
  SOG.status = 'ready'; SOG.won = false;
  SOG.elapsedMs = 0; soTimerStop();
  SOG.autoToastShown = false;
  soClearSelection();
  soReadyOverlay();
  soDealAnimation();
  soHud();
}
function soRestart() {
  // Replay the SAME deal from the order captured at deal time.
  const E = window.SolitaireEngine;
  if (!SOG.game || !SOG.dealOrder || SOG.dealOrder.length !== 52) { soNewGame(); return; }
  soStopCascade();
  if (SOG.autoTimer) { clearInterval(SOG.autoTimer); SOG.autoTimer = 0; }
  const seq = SOG.dealOrder;
  SOG.els.forEach(el => el.remove());
  SOG.els.clear();
  SOG.pos.clear();
  const fresh = E.createGame({ drawCount: 1 });
  fresh.tableau = [[], [], [], [], [], [], []];
  fresh.stock = []; fresh.waste = []; fresh.foundations = [[], [], [], []];
  let k = 0;
  for (let col = 0; col < 7; col++) {
    for (let row = 0; row <= col; row++) {
      // Reuse the ORIGINAL card objects: stable identity keeps element
      // bindings and faceUp flags valid across restarts.
      const c = seq[k++];
      c.faceUp = row === col;
      fresh.tableau[col].push(c);
    }
  }
  while (k < seq.length) { seq[k++].faceUp = false; fresh.stock.push(seq[k - 1]); }
  fresh.score = 0; fresh.moves = 0; fresh.recycles = 0;
  fresh.status = 'ready'; fresh.history = [];
  SOG.game = fresh;
  SOG.status = 'ready'; SOG.won = false;
  SOG.elapsedMs = 0; soTimerStop();
  SOG.autoToastShown = false;
  soClearSelection();
  soReadyOverlay();
  soDealAnimation();
  soHud();
  soToast('Same deal restarted');
}
function soStopCascade() {
  if (SOG.cascade) { cancelAnimationFrame(SOG.cascade); SOG.cascade = null; }
  document.querySelectorAll('.so-card.cascade').forEach(el => el.classList.remove('cascade'));
}
function soReadyOverlay() {
  const ov = so$('soOverlay'); if (!ov) return;
  so$('soOvTitle').textContent = 'Solitaire';
  so$('soOvSub').innerHTML =
    'Classic Klondike · Draw 1.<br>Build the four foundations from Ace to King.' +
    '<div class="so-ov-keys" style="margin-top:14px">' +
    '<span class="so-key">click</span><span class="so-key">drag</span>' +
    '<span class="so-key">double-click = smart move</span></div>';
  so$('soOvStats').classList.add('hidden');
  so$('soOvBtn').textContent = 'Start Playing';
  so$('soOvBtn').dataset.kind = 'start';
  ov.classList.remove('hidden');
}
function soUndo() {
  const E = window.SolitaireEngine, g = SOG.game;
  if (!g.history.length) { soToast('Nothing to undo'); return; }
  if (SOG.autoTimer) { clearInterval(SOG.autoTimer); SOG.autoTimer = 0; }
  E.undo(g);
  SOG.won = false;
  soStopCascade();
  // Undo from the win screen: restart the frozen HUD clock.
  if (SOG.status === 'won') { SOG.status = 'playing'; soTimerStart(); }
  soPaint(false);
  soClearSelection();
  soHud();
  soSfx('ui');
}
function soPauseToggle() {
  if (SOG.status === 'playing') {
    SOG.status = 'paused';
    SOG.elapsedMs = Date.now() - SOG.startedAt;
    soTimerStop();
    const ov = so$('soOverlay');
    if (ov) {
      so$('soOvTitle').textContent = 'Paused';
      so$('soOvSub').textContent = 'Take your time.';
      so$('soOvStats').classList.add('hidden');
      so$('soOvBtn').textContent = 'Resume';
      so$('soOvBtn').dataset.kind = 'resume';
      ov.classList.remove('hidden');
    }
  } else if (SOG.status === 'paused') {
    SOG.status = 'playing';
    SOG.startedAt = Date.now() - SOG.elapsedMs;
    soTimerStart();
    so$('soOverlay').classList.add('hidden');
  }
}

/* ---- View ---------------------------------------------------------------- */
function renderSoView() {
  document.getElementById('viewRoot').innerHTML = `
  <div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Solitaire</div>
    </div>

    <section class="ft-panel ft-game so-wide" id="soPanel" aria-label="Solitaire Classic Klondike card game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Solitaire <span style="font-weight:500;color:var(--ink-dim)">· Classic Klondike</span></div>
          <div class="ft-blurb">Draw 1 · build foundations Ace→King by suit · alternate colours down the tableau.</div>
        </div>
      </header>

      <div class="so-hud" role="status" aria-live="off">
        <span class="so-stat score"><span class="k">score</span><span class="v" id="soScore">0</span></span>
        <span class="so-stat"><span class="k">moves</span><span class="v" id="soMoves">0</span></span>
        <span class="so-stat time"><span class="k">time</span><span class="v" id="soTime">00:00</span></span>
        <span class="so-stat found"><span class="k">home</span><span class="v" id="soFound">0/52</span></span>
      </div>

      <div class="so-table" id="soTable">
        <div class="so-board" id="soBoard">
          <div class="so-row top">
            <div class="so-slot" id="soStock" data-pile="stock" title="Stock — click to draw"><span class="so-label">stock</span></div>
            <div class="so-slot" id="soWaste" data-pile="waste"><span class="so-label">waste</span></div>
            <div class="so-spacer" aria-hidden="true"></div>
            <div class="so-slot" id="soF0" data-pile="foundation" data-mark="${SO_SUITS.S}"><span class="so-label">${SO_SUITS.S}</span></div>
            <div class="so-slot" id="soF1" data-pile="foundation" data-mark="${SO_SUITS.H}"><span class="so-label">${SO_SUITS.H}</span></div>
            <div class="so-slot" id="soF2" data-pile="foundation" data-mark="${SO_SUITS.D}"><span class="so-label">${SO_SUITS.D}</span></div>
            <div class="so-slot" id="soF3" data-pile="foundation" data-mark="${SO_SUITS.C}"><span class="so-label">${SO_SUITS.C}</span></div>
          </div>
          <div class="so-row tab">
            ${[0, 1, 2, 3, 4, 5, 6].map(i =>
              `<div class="so-slot" id="soT${i}" data-pile="tableau"></div>`).join('')}
          </div>

          <div class="so-auto hidden pulse" id="soAutoBox">
            <span class="txt">Only foundation moves left</span>
            <button type="button" id="soAutoBtn">⚡ Auto-Finish</button>
          </div>

          <div class="so-overlay hidden" id="soOverlay" role="dialog" aria-modal="false" aria-label="Game status">
            <div class="so-ov-inner">
              <div class="so-ov-title" id="soOvTitle">Solitaire</div>
              <div class="so-ov-sub" id="soOvSub"></div>
              <div class="so-ov-stats hidden" id="soOvStats">
                <div class="so-ov-stat hl"><span class="k">score</span><span class="v" id="soRsScore">0</span></div>
                <div class="so-ov-stat"><span class="k">time</span><span class="v" id="soRsTime">00:00</span></div>
                <div class="so-ov-stat"><span class="k">moves</span><span class="v" id="soRsMoves">0</span></div>
              </div>
              <button type="button" class="so-ov-btn" id="soOvBtn" data-kind="start">Start Playing</button>
            </div>
          </div>

          <div class="so-toast" id="soToast" role="status"></div>
          <div class="so-brand">Soket Workflow</div>
        </div>
      </div>

      <div class="so-controls" role="group" aria-label="Game controls">
        <button type="button" class="ft-btn primary" id="soNewBtn">New Game</button>
        <button type="button" class="ft-btn" id="soRestartBtn">Restart</button>
        <button type="button" class="ft-btn" id="soUndoBtn">Undo</button>
        <button type="button" class="ft-btn" id="soHintBtn">Hint</button>
        <button type="button" class="ft-btn" id="soPauseBtn">Pause</button>
        <button type="button" class="ft-btn si-sound" id="soSoundBtn" aria-pressed="false">
          <span class="si-snd-ico" aria-hidden="true">♪</span><span id="soSoundLbl">sound off</span>
        </button>
        <button type="button" class="ft-btn" id="soFsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn si-exit" id="soExit">Back to Games</button>
      </div>

      <div class="so-help">
        <span>click / drag to move</span><span>double-click → smart move</span>
        <span>N new</span><span>Z undo</span><span>H hint</span><span>P pause</span><span>F fullscreen</span>
      </div>
    </section>
  </div></div>`;

  SOG.epoch++;
  const myEpoch = SOG.epoch;
  SOG.cleanup = [];
  SOG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const soOnLocal = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    SOG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };

  // Fresh deal on every mount (arcade convention).
  soNewGame();
  soSoundButton();

  // ---- card interactions (delegated on the board) ----
  soOnLocal(so$('soBoard'), 'click', e => {
    if (SOG.epoch !== myEpoch) return;
    if (SOG.status === 'won') return;
    const cardEl = e.target.closest('.so-card');
    if (cardEl && cardEl.dataset.id) { soCardClick(cardEl.dataset.id); return; }
    const slot = e.target.closest('.so-slot');
    if (slot) {
      if (slot.id === 'soStock') { soStockClick(); return; }
      if (SOG.selected) {
        const dest = soSlotFromElement(slot);
        if (dest && soAttemptDrop(dest)) return;
      }
      soClearSelection();
    }
  });
  soOnLocal(so$('soTable'), 'selectstart', e => e.preventDefault());

  soBindDrag();

  // ---- persistent controls ----
  soOnLocal(so$('soNewBtn'), 'click', () => { soNewGame(); soSfx('ui'); });
  soOnLocal(so$('soRestartBtn'), 'click', () => soRestart());
  soOnLocal(so$('soUndoBtn'), 'click', () => soUndo());
  soOnLocal(so$('soHintBtn'), 'click', () => soHint());
  soOnLocal(so$('soPauseBtn'), 'click', () => soPauseToggle());
  soOnLocal(so$('soSoundBtn'), 'click', () => soSoundToggle());
  soOnLocal(so$('soFsBtn'), 'click', () => soToggleFullscreen());
  soOnLocal(so$('soAutoBtn'), 'click', () => soAutoFinish());
  if (typeof ftFsSupported === 'function' && !ftFsSupported()) {
    const b = so$('soFsBtn'); if (b) b.style.display = 'none';
  }
  soOnLocal(so$('soExit'), 'click', () => { location.hash = '#funtime'; });

  // overlay button: start / resume / play again
  soOnLocal(so$('soOvBtn'), 'click', () => {
    const kind = so$('soOvBtn').dataset.kind;
    if (kind === 'again') { soNewGame(); return; }
    if (kind === 'resume') { soPauseToggle(); return; }
    so$('soOverlay').classList.add('hidden');
    soTouch();
  });

  // ---- keyboard ----
  // Escape is owned by the shared session controller (ftSessionBind).
  const GAME_KEYS = new Set(['n','N','z','Z','h','H','p','P','f','F','u','U']);
  soOnLocal(document, 'keydown', e => {
    if (SOG.epoch !== myEpoch) return;
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    const k = e.key;
    if ((k === 'z' || k === 'Z') && (e.metaKey || e.ctrlKey)) { e.preventDefault(); soUndo(); return; }
    if (GAME_KEYS.has(k)) e.preventDefault();
    if (k === 'n' || k === 'N') { soNewGame(); return; }
    if (k === 'z' || k === 'Z' || k === 'u' || k === 'U') { soUndo(); return; }
    if (k === 'h' || k === 'H') { soHint(); return; }
    if (k === 'p' || k === 'P') { soPauseToggle(); return; }
    if (k === 'f' || k === 'F') { soToggleFullscreen(); return; }
  });

  // ---- resize refits positions ----
  let rz = 0;
  soOnLocal(window, 'resize', () => {
    clearTimeout(rz);
    rz = setTimeout(() => { if (SOG.epoch === myEpoch) soPaint(true); }, 90);
  });

  // pause when the tab hides (keeps the clock honest)
  soOnLocal(document, 'visibilitychange', () => {
    if (SOG.epoch !== myEpoch) return;
    if (document.hidden && SOG.status === 'playing') soPauseToggle();
  });

  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, dashboard chrome hidden while playing.
  if (typeof ftSessionAfterMount === 'function') ftSessionAfterMount('so', 'so-fs');
  if (typeof ftSessionBind === 'function') ftSessionBind(soOnLocal, () => soPaint(true));

  soPaint(true);
}

/* ---- Fullscreen (shared FTFS convention) ---------------------------------- */
function soToggleFullscreen() {
  if (typeof ftToggleFullscreen === 'function') {
    ftToggleFullscreen('so', 'so-fs', null);
  }
}

/* ---- Teardown -------------------------------------------------------------- */
function soTeardown() {
  SOG.epoch++;                     // invalidates stale async continuations
  soTimerStop();
  soStopCascade();
  if (SOG.autoTimer) { clearInterval(SOG.autoTimer); SOG.autoTimer = 0; }
  clearTimeout(SOG.hintTimer);
  clearTimeout(SOG.toastTimer);
  for (const off of SOG.cleanup) { try { off(); } catch { /* gone */ } }
  SOG.cleanup = [];
  SOG.els.forEach(el => el.remove());
  SOG.els.clear();
  SOG.pos.clear();
  SOG.game = null;
  SOG.selected = null;
  SOG.status = 'ready'; SOG.won = false;
  if (SOS.ctx) { try { SOS.ctx.close(); } catch { /* already closed */ } }
  SOS.ctx = null; SOS.master = null;
  ftExitFullscreenQuiet();
  document.body.classList.remove('so-fs');
  ftSessionRestore();               // restore dashboard chrome
}

/* Expose for the console shell (router + tests). Mirrors timberman-game.js. */
window.renderSoView = renderSoView;
window.soTeardown = soTeardown;
window.SOG = SOG;
