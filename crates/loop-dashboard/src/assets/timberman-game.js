/* ============================================================
   FUN TIME · GAME 6 — TIMBER MAN ULTRA
   Route: #funtime/tm · Namespace: TM / tm* functions / .tm- CSS.

   A polished take on the arcade classic: chop the tree from the left
   or right side, dodge the branches, beat the countdown timer. Every
   chop adds score AND time; intensity ramps as the score climbs.

   Lifecycle mirrors the other Fun Time games exactly:
     • one state object (TMG) with an epoch guard that invalidates
       stale async continuations,
     • every listener registered through tmOn() so tmTeardown()
       removes them all on any route change,
     • automatic fullscreen from the card click via the shared FTSESS
       controller (ftSessionLaunch → ftSessionAfterMount), with the
       honest CSS immersive fallback when native is denied,
     • Esc leaves the game and returns to #funtime in ONE press,
     • dashboard chrome hidden while playing (body.ft-playing),
     • zero DOM/CSS leakage between mounts (namespaced + torn down).

   Rendering: pure canvas (no external assets, no CDNs). The
   lumberjack, trunk and branches are drawn procedurally with the
   dashboard's own design tokens (var(--acc) green, neon accents).
   ============================================================ */

/* ---- Injected styles (namespaced; no global selectors touched) ---- */
(function tmInjectCss() {
  if (document.getElementById('tm-style')) return;
  const st = document.createElement('style');
  st.id = 'tm-style';
  st.textContent = `
.tm-wide{max-width:980px}
/* ============ STAGE: the playfield owns the panel =====================
   PHASE 1/2 · The stage is a self-contained arcade screen: canvas fills
   it (letterboxed by tmResize at the correct aspect ratio), and the HUD
   lives INSIDE it as compact translucent overlays — no rows above or
   below, no permanent bottom control panel. */
.tm-stage{position:relative;flex:1;min-height:300px;width:100%;margin:0;
  border-radius:14px;overflow:hidden;border:1px solid var(--line2);
  background:linear-gradient(180deg,#08101f 0%,#0d1a2e 45%,#12233b 80%,#15283f 100%);
  user-select:none;-webkit-user-select:none;
  touch-action:none;display:flex;align-items:center;justify-content:center}
#tmCanvas{display:block;touch-action:none}
/* Inline (non-fullscreen) play: the panel gets a real viewport-derived
   height budget so tmResize() can fit the canvas without page scroll. */
body.ft-playing #tmPanel{margin:0 auto;width:100%;max-width:min(980px,100%);
  height:calc(100vh - 16px);padding:clamp(8px,1.6vh,16px) clamp(10px,2vw,22px);gap:0}
/* Fullscreen / immersive: the panel IS the screen. */
body.tm-fs #tmPanel,body.ft-playing.ft-immersive #tmPanel{
  max-width:none;width:100vw;height:100vh;margin:0;border-radius:0;border:none;
  overflow:hidden;justify-content:flex-start;box-sizing:border-box;
  padding:clamp(8px,1.8vh,18px) clamp(10px,2.5vw,26px);
  padding-left:max(clamp(10px,2.5vw,26px),env(safe-area-inset-left,0));
  padding-right:max(clamp(10px,2.5vw,26px),env(safe-area-inset-right,0));
  padding-top:max(clamp(8px,1.8vh,18px),env(safe-area-inset-top,0));
  padding-bottom:max(clamp(8px,1.8vh,18px),env(safe-area-inset-bottom,0));
  background:var(--bg,#0a0d13)}
body.tm-fs #tmPanel .ft-panel-h,
body.ft-playing.ft-immersive #tmPanel .ft-panel-h{display:none}

/* ============ PHASE 2 · OVERLAY HUD ===================================
   Compact translucent panels pinned to the stage edges. Three clusters:
   top-left score · top-center time+level · top-right best + icons. */
.tm-hud{position:absolute;inset:0;z-index:3;pointer-events:none;
  display:grid;grid-template-columns:1fr auto 1fr;grid-template-rows:auto 1fr;
  align-items:start;padding:clamp(8px,1.6vh,14px) clamp(10px,1.8vw,18px);
  gap:8px;font-family:Sora,sans-serif}
.tm-hud>*{pointer-events:none}
/* Interactive HUD children (icon buttons) re-enable pointer events so
   they stay clickable above the stage's chop zones. */
.tm-hud .tm-ibtn,.tm-hud button{pointer-events:auto}
.tm-stat{display:inline-flex;align-items:center;gap:8px;
  border:1px solid rgba(140,160,190,.22);border-radius:11px;
  padding:6px 12px;background:rgba(8,13,22,.58);
  backdrop-filter:blur(6px);-webkit-backdrop-filter:blur(6px);
  font:600 10px Sora;color:#93a3ba;letter-spacing:.12em;text-transform:uppercase;
  box-shadow:0 2px 12px rgba(0,0,0,.28)}
.tm-stat .v{font:800 17px "JetBrains Mono",monospace;color:#eaf2ff;
  min-width:2ch;text-align:right;font-variant-numeric:tabular-nums;
  transition:transform .12s ease}
.tm-stat.bump .v{transform:scale(1.22);color:#fff}
.tm-hud-l{grid-column:1;justify-self:start;display:flex;gap:8px}
.tm-hud-c{grid-column:2;justify-self:center;display:flex;flex-direction:column;
  align-items:center;gap:6px}
.tm-hud-r{grid-column:3;justify-self:end;display:flex;gap:8px;align-items:center}
.tm-stat.score .v{color:var(--acc)}
.tm-stat.best .v{color:#ffd166}
.tm-stat.time .v{color:var(--acc);min-width:4.2ch}
.tm-stat.time.warn .v{color:#ff8fa3}
.tm-stat.time.crit .v{color:#ff4d6d;animation:tmblink .5s ease-in-out infinite alternate}
@keyframes tmblink{to{opacity:.4}}
.tm-time-bar{width:110px;height:4px;border-radius:99px;background:rgba(0,0,0,.5);
  border:1px solid rgba(140,160,190,.2);overflow:hidden}
.tm-time-bar i{display:block;height:100%;width:100%;border-radius:99px;
  background:linear-gradient(90deg,#5ef2c0,#3ecf9a);transition:width .15s linear}
.tm-time-bar.warn i{background:linear-gradient(90deg,#ff8fa3,#ff4d6d)}
.tm-time-bar.crit i{background:linear-gradient(90deg,#ff4d6d,#c9184a)}
.tm-lvl-chip{font:700 9.5px Sora;letter-spacing:.16em;color:#8ecdf5;
  text-transform:uppercase;text-shadow:0 1px 6px rgba(0,0,0,.7)}
/* compact icon buttons (fullscreen + settings gear) */
.tm-ibtn{appearance:none;border:1px solid rgba(140,160,190,.25);border-radius:10px;
  width:34px;height:32px;display:inline-flex;align-items:center;justify-content:center;
  background:rgba(8,13,22,.58);backdrop-filter:blur(6px);cursor:pointer;color:#aab6c8;
  transition:color .12s,border-color .12s,background .12s;padding:0}
.tm-ibtn:hover{color:#eaf2ff;border-color:rgba(94,242,192,.5)}
.tm-ibtn svg{width:16px;height:16px;display:block}
@media (max-width:560px){
  .tm-hud{padding:8px 10px;gap:6px}
  .tm-stat{padding:4px 9px;font-size:8.5px;gap:6px}
  .tm-stat .v{font-size:14px}
  .tm-stat.time .v{font-size:14px}
  .tm-time-bar{width:86px}
  .tm-ibtn{width:30px;height:28px}
}
@media (max-height:520px) and (orientation:landscape){
  .tm-hud{padding:6px 10px;gap:5px}
  .tm-stat{padding:3px 8px;font-size:8px}
  .tm-stat .v{font-size:12.5px}
  .tm-time-bar{width:76px;height:3px}
  .tm-ibtn{width:28px;height:26px}
}

/* ============ touch zone hints (mobile, ready state only) ============= */
.tm-zones{position:absolute;inset:auto 0 0 0;z-index:2;display:none;
  justify-content:space-between;padding:0 6% 14px;pointer-events:none}
.tm-zones span{font:800 11px Sora;letter-spacing:.14em;color:rgba(234,242,255,.5);
  text-transform:uppercase;text-shadow:0 2px 8px rgba(0,0,0,.8)}
@media (pointer:coarse){.tm-stage[data-zones="1"] .tm-zones{display:flex}}

/* ============ overlays: ready / paused / over ========================= */
.tm-overlay{position:absolute;inset:0;z-index:6;display:flex;align-items:center;
  justify-content:center;background:radial-gradient(ellipse at center,
    rgba(6,10,18,.6) 0%,rgba(6,10,18,.88) 100%);
  backdrop-filter:blur(4px);text-align:center;padding:18px}
.tm-overlay.hidden{display:none}
.tm-ov-inner{max-width:480px;width:100%}
.tm-ov-title{font:800 clamp(24px,4vw,38px) Sora;letter-spacing:-.02em;color:#f2f7ff;
  margin-bottom:8px;text-shadow:0 0 30px rgba(94,242,192,.25)}
.tm-ov-sub{font:500 13.5px Sora;color:#9fb0c7;line-height:1.65;margin-bottom:16px}
.tm-ov-sub b{color:#e5edf9}
.tm-ov-keys{display:inline-flex;gap:8px;margin:0 0 16px;flex-wrap:wrap;justify-content:center}
.tm-key{border:1px solid var(--line2);border-bottom-width:3px;border-radius:8px;
  padding:5px 11px;font:700 11px "JetBrains Mono",monospace;color:#cdd9ea;
  background:rgba(0,0,0,.35)}
.tm-ov-stats{display:flex;gap:12px;justify-content:center;margin-bottom:16px;flex-wrap:wrap}
.tm-ov-stat{border:1px solid var(--line2);border-radius:12px;padding:9px 18px;
  background:rgba(0,0,0,.4)}
.tm-ov-stat .k{display:block;font:700 9.5px Sora;letter-spacing:.14em;
  text-transform:uppercase;color:#7f8ca1;margin-bottom:4px}
.tm-ov-stat .v{font:800 22px "JetBrains Mono",monospace;color:#fff}
.tm-ov-stat.hl .v{color:var(--acc)}
.tm-ov-row{display:flex;gap:10px;justify-content:center;flex-wrap:wrap}
.tm-ov-btn{appearance:none;border:0;border-radius:12px;padding:13px 30px;cursor:pointer;
  font:800 15px Sora;letter-spacing:.02em;color:#06251b;
  background:linear-gradient(135deg,#5ef2c0,#37c493);
  box-shadow:0 8px 26px rgba(94,242,192,.28);transition:transform .12s,box-shadow .12s}
.tm-ov-btn:hover{transform:translateY(-2px);box-shadow:0 12px 32px rgba(94,242,192,.38)}
.tm-ov-btn:active{transform:translateY(0)}
.tm-ov-btn.ghost{background:transparent;color:#cdd9ea;border:1px solid var(--line2);
  box-shadow:none}
.tm-ov-btn.ghost:hover{color:#fff;border-color:rgba(94,242,192,.5)}
.tm-ov-best{margin-top:14px;font:700 12px Sora;color:#ffd166;letter-spacing:.06em}
/* score popup near the lumberjack */
.tm-float{position:absolute;font:800 15px "JetBrains Mono",monospace;color:var(--acc);
  pointer-events:none;text-shadow:0 2px 8px rgba(0,0,0,.6);
  animation:tmfloat .8s ease-out forwards;z-index:4}
@keyframes tmfloat{to{transform:translateY(-46px);opacity:0}}
/* level-up toast */
.tm-toast{position:absolute;left:50%;top:18%;transform:translateX(-50%);z-index:5;
  pointer-events:none;text-align:center;animation:tmtoast 1.4s ease-out forwards}
.tm-toast .t1{font:800 clamp(20px,3vw,30px) Sora;letter-spacing:.04em;color:#8ecdf5;
  text-shadow:0 0 24px rgba(110,168,255,.55)}
.tm-toast .t2{font:700 11px Sora;letter-spacing:.22em;color:#9fb0c7;
  text-transform:uppercase;margin-top:2px}
@keyframes tmtoast{0%{opacity:0;transform:translateX(-50%) translateY(14px) scale(.92)}
  12%{opacity:1;transform:translateX(-50%) translateY(0) scale(1)}
  78%{opacity:1}100%{opacity:0;transform:translateX(-50%) translateY(-22px) scale(1.03)}}

/* ============ PHASE 6 · SETTINGS OVERLAY ============================== */
.tm-settings{position:absolute;inset:0;z-index:8;display:flex;align-items:center;
  justify-content:center;background:rgba(6,10,18,.72);backdrop-filter:blur(5px);padding:16px}
.tm-settings.hidden{display:none}
.tm-set-card{width:min(320px,94%);border:1px solid var(--line2);border-radius:16px;
  background:rgba(11,17,28,.96);box-shadow:0 18px 48px rgba(0,0,0,.5);
  padding:16px 16px 14px;display:flex;flex-direction:column;gap:11px;
  max-height:100%;overflow-y:auto}
.tm-set-head{display:flex;align-items:center;justify-content:space-between}
.tm-set-title{font:800 13px Sora;letter-spacing:.14em;color:#eaf2ff;text-transform:uppercase}
.tm-set-x{border:none;background:transparent;color:#7f8ca1;cursor:pointer;
  font-size:14px;line-height:1;padding:5px 7px;border-radius:6px}
.tm-set-x:hover{color:var(--ink);background:rgba(255,255,255,.07)}
.tm-set-btn{appearance:none;border:1px solid var(--line2);border-radius:11px;
  padding:11px 14px;background:rgba(255,255,255,.03);color:#dbe4f0;
  font:700 13px Sora;cursor:pointer;text-align:center;
  transition:color .12s,border-color .12s,background .12s}
.tm-set-btn:hover{color:#fff;border-color:rgba(94,242,192,.45)}
.tm-set-btn.primary{background:linear-gradient(135deg,#5ef2c0,#37c493);
  color:#06251b;border:none;font-weight:800}
.tm-set-btn.danger{color:#ff8fa3}
.tm-set-btn.danger:hover{border-color:rgba(255,143,171,.5)}
.tm-set-sep{height:1px;background:var(--line2);margin:2px 0}
.tm-au-row{display:flex;align-items:center;gap:10px}
.tm-au-l{flex:0 0 64px;font:600 10.5px Sora;color:#9fb0c7;letter-spacing:.08em;
  text-transform:uppercase}
.tm-au-slider{flex:1;min-width:0;accent-color:var(--acc);height:18px}
.tm-au-val{flex:0 0 34px;text-align:right;font:700 11px "JetBrains Mono",monospace;
  color:#e5edf9}
.tm-au-toggles{display:flex;gap:8px}
.tm-au-toggle{flex:1;display:inline-flex;align-items:center;justify-content:center;gap:7px;
  border:1px solid var(--line2);border-radius:9px;padding:8px 10px;background:transparent;
  color:#9fb0c7;font:700 11px Sora;cursor:pointer;
  transition:color .12s,border-color .12s,background .12s}
.tm-au-toggle:hover{color:var(--ink);border-color:var(--acc)}
.tm-au-dot{width:8px;height:8px;border-radius:50%;background:#4a5568;transition:background .12s}
.tm-au-toggle[aria-pressed="true"]{color:var(--acc);
  background:rgba(94,242,192,.09);border-color:rgba(94,242,192,.45)}
.tm-au-toggle[aria-pressed="true"] .tm-au-dot{background:var(--acc)}
.tm-set-hint{font:500 10px Sora;color:#7f8ca1;text-align:center;letter-spacing:.04em}

/* Fun Time selector card art (namespaced tm-card-*). */
.ft-card-art.tm{
  background:
    radial-gradient(circle at 30% 20%,rgba(94,242,192,.12),transparent 55%),
    linear-gradient(180deg,rgba(166,124,82,.10),rgba(7,9,13,.6));
}
.tm-card-scene{position:relative;width:min(250px,80%);height:clamp(96px,15vw,132px);
  display:block;overflow:hidden}
.tm-card-trunk{position:absolute;left:50%;bottom:0;transform:translateX(-50%);
  width:34px;height:88%;
  background:linear-gradient(90deg,#7a5230 0%,#a06b3c 45%,#6b4526 100%);
  border-radius:6px 6px 0 0}
.tm-card-branch{position:absolute;width:58px;height:12px;border-radius:8px;background:#8a5c33}
.tm-card-branch.l{left:calc(50% - 62px);top:34%;transform:rotate(-8deg)}
.tm-card-branch.r{left:calc(50% + 4px);top:56%;transform:rotate(8deg)}
.tm-card-man{position:absolute;right:16%;bottom:6px;width:30px;height:34px;
  filter:drop-shadow(0 0 8px rgba(94,242,192,.35))}
`;
  document.head.appendChild(st);
})();

/* ---- State ------------------------------------------------------------- */
const TMG = {
  epoch: 0, cleanup: [],
  state: 'ready',           // 'ready' | 'playing' | 'paused' | 'over'
  score: 0,
  best: +(localStorage.getItem('tmBest') || 0),
  timeMs: 0,                // remaining round time
  startTimeMs: 60000,       // round length: 60s countdown
  timerStarted: false,      // countdown begins on the FIRST chop
  level: 0,                 // index into TM_LEVELS (0..4)
  side: 'left',             // lumberjack's current side ('left'|'right')
  trunk: [],                // stack of segments: {branch:'left'|'right'|null}
  trunkCount: 6,            // visible trunk height
  chopAnim: 0,              // ms since last chop (drives axe swing)
  lastChopAt: 0,
  slideT: 1,                // 0→1 progress of the trunk slide-down anim
  flashRed: 0,              // ms of red hit-flash remaining
  shake: 0,                 // ms of screen shake remaining
  chopShake: 0,             // ms of micro-shake after each chop
  raf: 0,                   // requestAnimationFrame handle
  lastT: 0,                 // last frame timestamp
  ctx: null, canvas: null,
  dpr: 1, w: 0, h: 0,       // canvas metrics (CSS px)
  reducedMotion: false,
  particles: [],            // wood chips {x,y,vx,vy,life,rot,vr,size}
  floats: [],               // DOM "+1" popups (epoch-guarded)
  pausedByBlur: false,
  _lastTickSec: -1,         // countdown-tick bookkeeping
  _milestone: 0,            // next score-milestone for the reward blip
};

/* ---- AUDIO SYSTEM --------------------------------------------------------
   Professional arcade audio on ONE AudioContext (same architecture as
   fruitninja-game.js Phase 2B/2E and snakes-game.js):

     TMA.ctx        created only inside a user gesture (autoplay policy);
                    reused for the whole view lifecycle, never rebuilt.
     TMA.master     master gate: settings.on ? settings.mvol : 0
     TMA.sfxBus     effects bus → soft compressor → master
     TMA.bgm.gain   music fade bus → compressor → master
     TMA.lastPlay   per-kind dedup map (no stacked duplicates)

   MUSIC: procedural energetic arcade loop — Am–F–C–G progression, warm
   sub bass, offbeat hats, syncopated pentatonic lead. A look-ahead
   scheduler keeps ~0.35s booked ahead of the audio clock so looping is
   seamless by construction. INTENSITY tracks the difficulty level:
   each level-up adds lead density, hats and BPM via tmMusicIntensity().

   Settings persist under 'tm-audio-v1' (music/sfx on + volumes), loaded
   once per mount, applied live through click-free setTargetAtTime ramps. */
const TM_AUDIO_KEY = 'tm-audio-v1';
const TM_SFX_GAP = 45;            // ms: identical sounds closer dedupe

const TMA = {
  ctx: null, master: null,
  sfxBus: null,
  bgm: {                          // music bus + scheduler state
    gain: null, comp: null,       // fade bus → compressor → master
    started: false, timer: 0,     // ONE scheduler interval, never two
    step: 0, nextT: 0,            // 16th-note grid position + anchor
  },
  lastPlay: {},                   // kind → last start (dedup guard)
  noiseBuf: null,                 // shared white-noise buffer
  unlocked: false,                // a user gesture opened the ctx
  settings: { mu: 1, sx: 1, mv: 0.4, sv: 0.9 },  // persisted prefs
};

/** Load + validate persisted audio prefs (defensive per-field fallback). */
function tmAudioLoadPrefs() {
  try {
    const raw = localStorage.getItem(TM_AUDIO_KEY);
    if (!raw) return;
    const o = JSON.parse(raw);
    if (!o || typeof o !== 'object') return;
    const num01 = v => typeof v === 'number' && isFinite(v) && v >= 0 && v <= 1 ? v : null;
    const mv = num01(o.mv), sv = num01(o.sv);
    if (mv !== null) TMA.settings.mv = mv;
    if (sv !== null) TMA.settings.sv = sv;
    if (o.mu === 0 || o.mu === 1) TMA.settings.mu = o.mu;
    if (o.sx === 0 || o.sx === 1) TMA.settings.sx = o.sx;
  } catch { /* corrupted blob → defaults */ }
}

/** Persist current settings (best effort). */
function tmAudioSavePrefs() {
  try {
    localStorage.setItem(TM_AUDIO_KEY, JSON.stringify(TMA.settings));
  } catch { /* storage unavailable */ }
}
tmAudioLoadPrefs();

/** Create (once) or reuse the context; resume if suspended. Returns the
 *  context or null. `inGesture` guards creation to real user gestures. */
function tmAudioInit(inGesture) {
  if (TMA.ctx) {
    if (TMA.ctx.state === 'suspended') TMA.ctx.resume().catch(() => {});
    return TMA.ctx;
  }
  if (!inGesture) return null;                       // autoplay policy
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return null;
  try {
    TMA.ctx = new AC();
    TMA.master = TMA.ctx.createGain();
    TMA.master.gain.value = 0;                       // silent until applied
    TMA.master.connect(TMA.ctx.destination);
    // SFX bus + soft limiter: rapid chops stack without clipping.
    TMA.sfxBus = TMA.ctx.createGain();
    const sc = TMA.ctx.createDynamicsCompressor();
    sc.threshold.value = -14; sc.knee.value = 6; sc.ratio.value = 8;
    sc.attack.value = 0.003; sc.release.value = 0.12;
    sc.connect(TMA.master); TMA.sfxBus.connect(sc);
    // Music bus: fade gain → transparent compressor → master.
    TMA.bgm.gain = TMA.ctx.createGain();
    TMA.bgm.gain.gain.value = 0;
    const mc = TMA.ctx.createDynamicsCompressor();
    mc.threshold.value = -18; mc.knee.value = 12; mc.ratio.value = 3;
    mc.attack.value = 0.004; mc.release.value = 0.26;
    mc.connect(TMA.master); TMA.bgm.gain.connect(mc);
    // 1s shared white-noise buffer for impacts/hats.
    const len = Math.floor(TMA.ctx.sampleRate);
    TMA.noiseBuf = TMA.ctx.createBuffer(1, len, TMA.ctx.sampleRate);
    const ch = TMA.noiseBuf.getChannelData(0);
    for (let i = 0; i < len; i++) ch[i] = Math.random() * 2 - 1;
    if (TMA.ctx.state === 'suspended') TMA.ctx.resume().catch(() => {});
    tmAudioApplyVolumes();
    return TMA.ctx;
  } catch { TMA.ctx = null; return null; }
}

/** Autoplay unlock: call from every pointerdown/keydown until it sticks. */
function tmAudioUnlock() {
  if (TMA.unlocked) return;
  if (tmAudioInit(true)) TMA.unlocked = true;
}

/** Push settings into the gain graph (click-free ramps). */
function tmAudioApplyVolumes() {
  if (!TMA.ctx || !TMA.master) return;
  const t = TMA.ctx.currentTime;
  try {
    TMA.master.gain.setTargetAtTime(1, t, 0.02);
    TMA.sfxBus.gain.setTargetAtTime(TMA.settings.sx ? TMA.settings.sv : 0, t, 0.02);
    tmBgmApplyGain();
  } catch { /* ctx mid-close */ }
}

/* ---- SFX synth voices ---------------------------------------------------- */
/** One tone on the SFX bus. `when` (absolute audio time) lets cues be
 *  pre-scheduled so multi-hit sounds stay tight and never setTimeout-drift. */
function tmTone(freq, dur, vol, type, slideTo, when, bus) {
  const ctx = TMA.ctx;
  if (!ctx || !TMA.settings.sx || !freq) return;
  if (ctx.state !== 'running') return;               // never book into a frozen clock
  const t = when != null && when > ctx.currentTime ? when : ctx.currentTime;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(Math.max(20, slideTo), t + dur);
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(Math.max(0.001, vol), t + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g); g.connect(bus || TMA.sfxBus);
  osc.start(t); osc.stop(t + dur + 0.02);
}

/** Filtered noise burst on the SFX bus (wood cracks, impacts). */
function tmNoise(vol, dur, f0, f1, type, when, q) {
  const ctx = TMA.ctx;
  if (!ctx || !TMA.settings.sx || !TMA.noiseBuf) return;
  if (ctx.state !== 'running') return;
  const t = when != null && when > ctx.currentTime ? when : ctx.currentTime;
  const src = ctx.createBufferSource();
  src.buffer = TMA.noiseBuf; src.loop = true;
  const flt = ctx.createBiquadFilter();
  flt.type = type || 'bandpass'; flt.Q.value = q || 0.9;
  flt.frequency.setValueAtTime(f0, t);
  flt.frequency.exponentialRampToValueAtTime(Math.max(30, f1), t + dur);
  const g = ctx.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(Math.max(0.001, vol), t + 0.005);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(flt); flt.connect(g); g.connect(TMA.sfxBus);
  src.start(t); src.stop(t + dur + 0.02);
}

/**
 * Play a named sound effect. Every cue is composed from several scheduled
 * voices at absolute audio times (tight, no timer drift). Identical kinds
 * inside TM_SFX_GAP ms collapse to one (rapid chops can't buzz).
 */
function tmSfx(kind) {
  if (!TMA.settings.sx) return;
  const now = performance.now();
  if (TNA_lastGap(kind)) return;
  const ctx = TMA.ctx;
  if (!ctx || ctx.state !== 'running') return;       // programmatic triggers only use a live clock
  const T = ctx.currentTime;

  switch (kind) {
    /* axe chop: sharp wooden crack + body knock + pitch variation */
    case 'chop': {
      const v = 0.9 + Math.random() * 0.35;          // variation factor
      tmNoise(0.22 * v, 0.06, 1600 * v, 500, 'bandpass', T, 1.4);
      tmTone(170 * v, 0.05, 0.20, 'square', 95, T);
      tmTone(96, 0.09, 0.16, 'sine', 60, T + 0.012); // deep body thud
      break;
    }
    /* tree segment falling: deep wooden impact + low boom */
    case 'thud':
      tmNoise(0.20, 0.22, 420, 90, 'lowpass');
      tmTone(120, 0.28, 0.30, 'sine', 42);
      tmTone(64, 0.34, 0.20, 'triangle', 38, T + 0.03);
      break;
    /* character side change: subtle woosh tick */
    case 'move':
      tmNoise(0.06, 0.07, 900, 1900, 'bandpass');
      tmTone(430, 0.04, 0.05, 'triangle');
      break;
    /* score milestone: short arcade reward blip */
    case 'score':
      tmTone(880, 0.06, 0.10, 'triangle');
      tmTone(1175, 0.08, 0.10, 'triangle', null, T + 0.055);
      break;
    /* countdown tick (final 10s): urgent woodblock */
    case 'tick':
      tmTone(1050, 0.03, 0.09, 'square', 900);
      break;
    /* final-5s critical tick: brighter + double hit */
    case 'tickcrit':
      tmTone(1400, 0.03, 0.11, 'square', 1150);
      tmTone(1400, 0.025, 0.07, 'square', 1150, T + 0.09);
      break;
    /* level up: bright rising fanfare */
    case 'level':
      [523, 659, 784, 1047].forEach((f, i) =>
        tmTone(f, 0.10, 0.13, 'triangle', null, T + i * 0.07));
      tmTone(1319, 0.20, 0.10, 'sine', null, T + 0.30);
      break;
    /* game over: dramatic descending cadence + impact */
    case 'over':
      [392, 311, 247, 185].forEach((f, i) =>
        tmTone(f, 0.17, 0.15, 'sawtooth', null, T + i * 0.14));
      tmNoise(0.25, 0.5, 300, 60, 'lowpass', T + 0.55);
      tmTone(70, 0.5, 0.24, 'sine', 36, T + 0.55);
      break;
    /* new best: victory celebration arpeggio + bells */
    case 'best':
      [523, 659, 784, 1047, 1319].forEach((f, i) =>
        tmTone(f, 0.12, 0.13, 'triangle', null, T + i * 0.09));
      tmTone(1568, 0.34, 0.11, 'sine', null, T + 0.48);
      tmTone(2093, 0.30, 0.08, 'sine', null, T + 0.56);
      break;
    /* game start: quick arcade launch sweep */
    case 'start':
      tmTone(330, 0.10, 0.12, 'square', 660);
      tmTone(660, 0.08, 0.10, 'square', 990, T + 0.10);
      tmTone(990, 0.14, 0.11, 'triangle', null, T + 0.19);
      break;
    /* generic UI press */
    case 'ui':
      tmTone(880, 0.03, 0.05, 'sine', 780);
      break;
  }
}
/** Dedup bookkeeping for tmSfx. True → drop this call. */
function TNA_lastGap(kind) {
  const n = performance.now();
  if (TMA.lastPlay[kind] && n - TMA.lastPlay[kind] < TM_SFX_GAP) return true;
  TMA.lastPlay[kind] = n;
  return false;
}

/* ---- BACKGROUND MUSIC ------------------------------------------------------
   Energetic arcade loop: Am F C G ×2 (8 bars, 128 sixteenths at 118 BPM),
   kick/snare/hats, warm sub bass, syncopated pentatonic lead in bars 5–8.
   Intensity 1..5 (difficulty level) unlocks layers:
     L1 core groove (kick+snare+bass, sparse hats)
     L2 + offbeat hats everywhere
     L3 + chord stabs on the backbeat, denser bass
     L4 + lead plays bars 1–8, extra kick push
     L5 + double-time hat fills + octave bass pops
   The scheduler only BOOKS notes ahead; nothing runs per frame.          */
const TM_BGM = {
  bpm: 118, spb: 60 / 118,
  chords: [
    [220.00, 261.63, 329.63],      // Am
    [174.61, 220.00, 261.63],      // F
    [130.81, 196.00, 261.63],      // C
    [196.00, 246.94, 293.66],      // G
    [220.00, 261.63, 329.63],      // Am
    [174.61, 220.00, 261.63],      // F
    [261.63, 329.63, 392.00],      // C (lift)
    [196.00, 246.94, 293.66],      // G
  ],
  roots: [110.00, 87.31, 65.41, 98.00, 110.00, 87.31, 130.81, 98.00],
  leadPool: [440.00, 523.25, 587.33, 659.25, 783.99, 880.00],
};

/** Music intensity follows the difficulty level (clamped 1..5).
 *  TMG.level is 0-indexed (0=EASY … 4=INSANE) → intensity 1..5. */
function tmMusicIntensity() {
  return Math.max(1, Math.min(5, (TMG.level || 0) + 1));
}

/** True when the music bus chain is live. */
function tmBgmReady() {
  return !!(TMA.ctx && TMA.bgm.gain && TMA.bgm.comp);
}

/** Target music level: 0 unless enabled AND playing. */
function tmBgmTarget() {
  return (TMA.settings.mu && TMA.bgm.started) ? TMA.settings.mv : 0;
}

/** Smoothly move the fade bus to its target (fade in/out). */
function tmBgmApplyGain() {
  if (!tmBgmReady()) return;
  try {
    TMA.bgm.gain.gain.setTargetAtTime(tmBgmTarget(), TMA.ctx.currentTime, 0.12);
  } catch { /* ctx mid-close */ }
}

/** One synth voice into the music bus. */
function tmBgmVoice(freq, t, dur, vol, type, attack, release, glideTo, glideT) {
  const ctx = TMA.ctx;
  const osc = ctx.createOscillator(), g = ctx.createGain();
  osc.type = type || 'triangle';
  osc.frequency.setValueAtTime(freq, t);
  if (glideTo) osc.frequency.exponentialRampToValueAtTime(Math.max(20, glideTo), t + (glideT || dur));
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(vol, t + attack / 1000);
  g.gain.setValueAtTime(vol, t + Math.max(attack / 1000, dur - release / 1000));
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g); g.connect(TMA.bgm.gain);
  osc.start(t); osc.stop(t + dur + 0.05);
}

/** Percussive hat into the music bus. */
function tmBgmHat(t, vol) {
  const src = TMA.ctx.createBufferSource();
  src.buffer = TMA.noiseBuf;
  const flt = TMA.ctx.createBiquadFilter();
  flt.type = 'highpass'; flt.frequency.value = 7000;
  const g = TMA.ctx.createGain();
  g.gain.setValueAtTime(vol, t);
  g.gain.exponentialRampToValueAtTime(0.0001, t + 0.04);
  src.connect(flt); flt.connect(g); g.connect(TMA.bgm.gain);
  src.start(t); src.stop(t + 0.06);
}

/** Kick: sine drop 150→45 Hz. */
function tmBgmKick(t, vol) {
  tmBgmVoice(150, t, 0.16, vol, 'sine', 2, 60, 45, 0.14);
}

/** Snare-ish rim. */
function tmBgmSnare(t, vol) {
  tmBgmVoice(190, t, 0.09, vol, 'triangle', 2, 50, 150, 0.07);
}

/** Schedule ONE 16th-step of music at audio time t. */
function tmBgmStep(step, t) {
  const s = step % 128, bar = (s / 16) | 0, six = s % 16;
  const B = TM_BGM.spb, st = B / 4;
  const I = tmMusicIntensity();                     // 1..5

  // drums
  if (six === 0) tmBgmKick(t, 0.50);
  if (six === 8) tmBgmKick(t, I >= 4 ? 0.40 : 0.30);
  if (I >= 4 && six === 11) tmBgmKick(t, 0.24);     // expert push
  if (six === 4 || six === 12) tmBgmSnare(t, 0.22);
  if (I >= 2 ? six % 2 === 1 : six % 4 === 3) tmBgmHat(t, 0.10);
  if (I >= 5 && six === 14) tmBgmHat(t, 0.16);      // insane fill tick

  // chord pad (L1+) / stabs (L3+)
  if (six === 0) {
    const ch = TM_BGM.chords[bar];
    tmBgmVoice(ch[0], t, B * 3.6, 0.10, 'triangle', 30, 400);
    tmBgmVoice(ch[1], t, B * 3.6, 0.085, 'triangle', 30, 0);
    tmBgmVoice(ch[2], t, B * 3.3, 0.07, 'sine', 40, 0);
  }
  if (I >= 3 && (six === 6 || six === 14)) {
    const ch = TM_BGM.chords[bar];
    tmBgmVoice(ch[1] * 2, t, st * 2, 0.055, 'square', 4, 40);
    tmBgmVoice(ch[2] * 2, t, st * 2, 0.045, 'square', 4, 40);
  }

  // bass
  if (six === 0) tmBgmVoice(TM_BGM.roots[bar], t, B * 0.9, 0.30, 'sine', 5, 90);
  if (six === 6 || six === 14) {
    tmBgmVoice(TM_BGM.roots[bar] * 2, t, B * 0.4, 0.20, 'triangle', 5, 60);
  }
  if (I >= 5 && six === 10) tmBgmVoice(TM_BGM.roots[bar] * 2, t, st * 2, 0.14, 'square', 4, 40);

  // lead: bars 5–8 at L1–3, all bars at L4+
  const leadBars = I >= 4 ? 0 : 4;
  if (bar >= leadBars && six % 2 === 0) {
    if ((step * 13 + bar) % 5 !== 0) {
      const idx = (step * 7 + bar * 3) % TM_BGM.leadPool.length;
      tmBgmVoice(TM_BGM.leadPool[idx], t, st * 3, 0.11, 'square', 8, 70);
    }
  }
}

/** Look-ahead scheduler tick: keep ~0.35s booked ahead. */
function tmBgmSchedule() {
  if (!tmBgmReady() || !TMA.bgm.started) return;
  const horizon = TMA.ctx.currentTime + 0.35;
  while (TMA.bgm.nextT < horizon) {
    tmBgmStep(TMA.bgm.step, TMA.bgm.nextT);
    TMA.bgm.step++;
    TMA.bgm.nextT += TM_BGM.spb / 4;
  }
}

/* ---- audio UI sync ---------------------------------------------------------
   Paints the persisted audio prefs (TMA.settings) into the settings card
   (sliders + toggles) via the view's bound sync closure, TMG._auSync —
   which renderTmView() creates at mount. Safe pre-mount (no DOM → no-op).
   tmSoundButton() is a legacy alias kept for the console shell / older
   callers: the standalone sound button became the settings overlay in
   PHASE 6, so it now just repaints the same card. */
function tmSoundButton() { tmAudioSyncUi(); }
function tmAudioSyncUi() {
  if (typeof TMG !== 'undefined' && TMG._auSync) TMG._auSync();
}

/** Start the music: idempotent, gesture-owned. */
function tmBgmStart() {
  if (TMA.bgm.started) return;
  if (!tmAudioInit(false)) return;                   // only a live ctx may play
  if (!TMA.ctx || TMA.ctx.state !== 'running') return;
  TMA.bgm.started = true;
  TMA.bgm.step = 0;
  TMA.bgm.nextT = TMA.ctx.currentTime + 0.1;
  tmBgmApplyGain();                                  // fade in
  if (TMA.bgm.timer) clearInterval(TMA.bgm.timer);
  TMA.bgm.timer = setInterval(tmBgmSchedule, 120);
  tmBgmSchedule();
}

/** Stop the music + clear the scheduler (fade-out handled by the bus). */
function tmBgmStop() {
  if (TMA.bgm.timer) { clearInterval(TMA.bgm.timer); TMA.bgm.timer = 0; }
  TMA.bgm.started = false;
  tmBgmApplyGain();                                  // fade out
}

/** Full audio teardown: stop music, close the context ONCE. */
function tmAudioDispose() {
  tmBgmStop();
  if (TMA.ctx) { try { TMA.ctx.close(); } catch { /* already closed */ } }
  TMA.ctx = null; TMA.master = null; TMA.sfxBus = null;
  TMA.bgm.gain = null; TMA.bgm.comp = null;
  TMA.bgm.started = false;
  TMA.noiseBuf = null; TMA.lastPlay = {}; TMA.unlocked = false;
}

/* ---- Helpers ------------------------------------------------------------ */
function tm$(id) { return document.getElementById(id); }
function tmOn(target, ev, fn, opts) {
  target.addEventListener(ev, fn, opts);
  TMG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
}
function tmRand(n) { return Math.floor(Math.random() * n); }

/* ---- Levels ---------------------------------------------------------------
   Progressive difficulty: 5 named stages. `at` = score threshold; params
   scale branch density (prob), chop bonus refund, and the visual pace of
   the trunk slide. Fairness is structural: no two consecutive segments
   may carry a branch on the SAME side (guaranteed in generation), so a
   correct alternation always survives — difficulty comes from reading
   faster patterns, never from unwinnable states.                          */
const TM_LEVELS = [
  { name: 'EASY',   at: 0,   prob: 0.34, bonus: 900,  slideMs: 150 },
  { name: 'NORMAL', at: 15,  prob: 0.42, bonus: 750,  slideMs: 120 },
  { name: 'HARD',   at: 40,  prob: 0.50, bonus: 620,  slideMs: 95  },
  { name: 'EXPERT', at: 80,  prob: 0.58, bonus: 500,  slideMs: 75  },
  { name: 'INSANE', at: 130, prob: 0.66, bonus: 420,  slideMs: 60  },
];

/** Current level index (0..4) for a score. */
function tmLevelFor(score) {
  let i = 0;
  for (let k = 0; k < TM_LEVELS.length; k++) if (score >= TM_LEVELS[k].at) i = k;
  return i;
}

/** Live level params object. */
function tmLevelCfg() { return TM_LEVELS[TMG.level]; }

/** Difficulty factor kept for any legacy callers (1 → ~2.2). */
function tmDifficulty() { return 1 + Math.min(TMG.score / 40, 1.2); }

/** Time refunded per chop — level-scaled (urgency grows with stage). */
function tmChopBonus() { return tmLevelCfg().bonus; }

/** Fresh trunk with a safe start (no branch at the lumberjack's sides for
 *  the first two segments so the opening beats are always fair).
 *  Fairness invariant: never two branches in a row on the same side. */
function tmNewTrunk() {
  const t = [];
  let lastBranch = null;
  for (let i = 0; i < TMG.trunkCount + 4; i++) {
    if (i < 2) { t.push({ branch: null }); continue; }
    let b = null;
    if (Math.random() < tmLevelCfg().prob) {
      b = Math.random() < 0.5 ? 'left' : 'right';
      if (b === lastBranch) b = b === 'left' ? 'right' : 'left';  // fairness
    }
    t.push({ branch: b });
    lastBranch = b;
  }
  return t;
}

/** Reset the round (keeps `best`).
 *  NOTE: must NOT touch TMG.epoch. The epoch is a mount-session token:
 *  renderTmView() captures it (myEpoch / _loopEpoch) for the RAF-loop and
 *  input guards, and tmTeardown() raises it on unmount. Bumping it here
 *  would desynchronise epoch from _loopEpoch, killing the frame loop on
 *  its first tick and permanently disarming every `myEpoch` handler —
 *  including after an in-session restart. Round-scoped continuations
 *  (the "+1" float timers) are already safe: each removes only its own
 *  detached-on-remount element. */
function tmReset() {
  if (TMG.holdTimer) { clearInterval(TMG.holdTimer); TMG.holdTimer = 0; }
  TMG.state = 'ready';
  TMG.score = 0;
  TMG.timeMs = TMG.startTimeMs;
  TMG.timerStarted = false;
  TMG.level = 0;
  TMG.side = 'left';
  TMG.trunk = tmNewTrunk();
  TMG.chopAnim = 0; TMG.lastChopAt = 0;
  TMG.slideT = 1;
  TMG.flashRed = 0; TMG.shake = 0; TMG.chopShake = 0;
  TMG.particles = []; TMG.floats = [];
  TMG._lastTickSec = -1;
  TMG._milestone = 0;
  tmHud();
}

/* ---- Game logic --------------------------------------------------------- */
/** Attempt a chop on `side`. Returns true when the chop was applied. */
function tmChop(side) {
  if (TMG.state !== 'playing') return false;
  // PHASE 4 · smooth slide when the lumberjack switches sides.
  if (side !== TMG.side) {
    TMG.sideAnim.from = TMG.side;
    TMG.sideAnim.t = 0;
    tmSfx('move');
  }
  TMG.side = side;
  TMG.chopAnim = performance.now();
  TMG.lastChopAt = TMG.chopAnim;

  // Branch collision: the segment AT the lumberjack's head level (index 1
  // — index 0 is the stump below) must not hold a branch on his side.
  const headSeg = TMG.trunk[1];
  if (headSeg && headSeg.branch === side) {
    // PHASE 4 · collision feedback: wood-burst at the impact point.
    if (!TMG.reducedMotion) {
      const dir = side === 'left' ? -1 : 1;
      for (let i = 0; i < 10; i++) {
        TMG.particles.push({
          x: TM_VW / 2 + dir * 44,
          y: TM_VH * 0.60 + (Math.random() - 0.5) * 30,
          vx: dir * (0.6 + Math.random() * 2.2),
          vy: -(1.5 + Math.random() * 2.8),
          rot: Math.random() * Math.PI,
          vr: (Math.random() - 0.5) * 0.35,
          life: 1,
          size: 4 + Math.random() * 7,
        });
      }
    }
    tmGameOver('branch');
    return false;
  }

  // The countdown begins with the first successful chop (arcade convention:
  // "ready" time before that lets the player read the opening pattern).
  if (!TMG.timerStarted) { TMG.timerStarted = true; tmBgmStart(); }

  // Successful chop: score + time + shift the trunk.
  const prevLevel = TMG.level;
  TMG.score++;
  TMG.level = tmLevelFor(TMG.score);
  TMG.timeMs = Math.min(TMG.startTimeMs, TMG.timeMs + tmChopBonus());
  TMG.trunk.shift();
  TMG.slideT = 0;                       // animate the trunk sliding down
  // New bottom segment keeps the stack alive; never two branches in a row
  // on the same side (guarantees the game is always survivable).
  const top = TMG.trunk[TMG.trunk.length - 1];
  let nb = null;
  if (Math.random() < tmLevelCfg().prob) {
    const lastBranch = top ? top.branch : null;
    nb = Math.random() < 0.5 ? 'left' : 'right';
    if (nb === lastBranch) nb = nb === 'left' ? 'right' : 'left';
  }
  TMG.trunk.push({ branch: nb });

  // Feedback: chips, micro-shake, sounds.
  TMG.chopShake = 120;
  tmSpawnChips();
  tmFloatScore();
  tmSfx('chop');
  if (TMG.score % 2 === 0) tmSfx('thud');           // periodic deep impact
  if (TMG.score >= TMG._milestone + 10) {           // reward blip every 10
    TMG._milestone = TMG.score;
    tmSfx('score');
  }
  if (TMG.level > prevLevel) tmLevelUp();           // fanfare + toast
  tmHud();
  return true;
}

/** Level-up: sound + toast notification. */
function tmLevelUp() {
  tmSfx('level');
  const cfg = tmLevelCfg();
  const wrap = tm$('tmStageWrap');
  if (!wrap || TMG.reducedMotion) return;
  const el = document.createElement('div');
  el.className = 'tm-toast';
  el.innerHTML = `<div class="t1">LEVEL ${TMG.level + 1}</div><div class="t2">${cfg.name}</div>`;
  wrap.appendChild(el);
  const myEpoch = TMG.epoch;
  setTimeout(() => { if (myEpoch === TMG.epoch) el.remove(); }, 1450);
}

function tmStart() {
  if (TMG.state === 'over' || TMG.state === 'ready') tmReset();
  TMG.state = 'playing';
  TMG.lastT = performance.now();
  tmOverlay(null);
  tmSfx('start');
  tmHud();
  // Music starts with the first chop (tmChop) — the countdown and the
  // soundtrack begin together, inside the player's own rhythm.
  // Exactly one RAF chain: never stack a second on top of a live one.
  if (!TMG.raf) TMG.raf = requestAnimationFrame(tmFrame);
  else tmDraw(performance.now());   // loop already running; refresh now
}

function tmTogglePause() {
  if (TMG.state === 'playing') {
    TMG.state = 'paused';
    tmBgmStop();                    // music fades out while paused
    tmOverlay('paused');
  } else if (TMG.state === 'paused') {
    TMG.state = 'playing';
    TMG.lastT = performance.now();
    tmOverlay(null);
    if (TMG.timerStarted) tmBgmStart();   // resume the groove
  }
  tmSfx('ui');
}

function tmGameOver(reason) {
  if (TMG.state === 'over') return;
  TMG.state = 'over';
  TMG.flashRed = 320;
  TMG.shake = 340;
  tmBgmStop();                      // music out, stinger clean
  const newBest = TMG.score > TMG.best;
  if (newBest) {
    TMG.best = TMG.score;
    localStorage.setItem('tmBest', String(TMG.best));
  }
  tmSfx('over');
  if (newBest && TMG.score > 0) setTimeout(() => tmSfx('best'), 700);
  tmOverlay('over', reason, newBest);
  tmHud();
}

/* ---- Overlays / HUD ------------------------------------------------------ */
function tmOverlay(kind, reason, newBest) {
  const ov = tm$('tmOverlay');
  if (!ov) return;
  const stage = tm$('tmStageWrap');
  if (stage) stage.dataset.zones = kind === 'ready' ? '1' : '0';
  if (!kind) { ov.classList.add('hidden'); return; }
  ov.classList.remove('hidden');
  const title = tm$('tmOvTitle'), sub = tm$('tmOvSub'),
        keys = tm$('tmOvKeys'), stats = tm$('tmOvStats'), btn = tm$('tmOvBtn'),
        exit = tm$('tmOvExit'), sc = tm$('tmRsScore'), rsb = tm$('tmRsBest'),
        best = tm$('tmOvBest');
  if (kind === 'ready') {
    title.textContent = 'Timber Man Ultra';
    sub.innerHTML = 'Chop the tree. Dodge the branches.<br>The 60-second clock starts on your first chop.';
    keys.classList.remove('hidden');
    keys.innerHTML = '<span class="tm-key">← / A</span><span class="tm-key">→ / D</span>'
      + '<span class="tm-key">P pause</span><span class="tm-key">R restart</span>';
    stats.classList.add('hidden');
    btn.textContent = 'Start Chopping';
    btn.dataset.kind = 'start';
    if (exit) exit.classList.add('hidden');
    if (best) best.textContent = TMG.best > 0 ? `★ best ${TMG.best}` : '';
  } else if (kind === 'paused') {
    title.textContent = 'Paused';
    sub.innerHTML = 'Take a breath.';
    keys.classList.add('hidden');
    stats.classList.add('hidden');
    btn.textContent = 'Resume';
    btn.dataset.kind = 'resume';
    if (exit) exit.classList.add('hidden');
  } else if (kind === 'over') {
    title.textContent = newBest ? 'New Best!' : 'Timber Down!';
    sub.innerHTML = reason === 'time' ? 'The clock ran out.' : 'A branch got you.';
    keys.classList.add('hidden');
    stats.classList.remove('hidden');
    if (sc) sc.textContent = String(TMG.score);
    if (rsb) rsb.textContent = String(TMG.best);
    btn.textContent = 'Chop Again';
    btn.dataset.kind = 'again';
    if (exit) exit.classList.remove('hidden');
    if (best) best.textContent = newBest
      ? `★ new best ${TMG.best}!`
      : (TMG.best > 0 ? `★ best ${TMG.best}` : '');
  }
}

function tmHud() {
  const s = tm$('tmScore');
  if (s && s.textContent !== String(TMG.score)) {
    // PHASE 2 · smooth score update: brief scale pulse on change.
    s.textContent = String(TMG.score);
    const st = s.closest('.tm-stat');
    if (st && !TMG.reducedMotion) {
      st.classList.remove('bump');
      void st.offsetWidth;                          // restart the animation
      st.classList.add('bump');
    }
  }
  const b = tm$('tmBestV'); if (b) b.textContent = String(TMG.best);
  const t = tm$('tmTime'); if (t) t.textContent = tmTimeText();
  const bar = tm$('tmTimeBar'), fill = tm$('tmTimeFill');
  if (fill && bar) {
    const pct = Math.max(0, Math.min(100, TMG.timeMs / TMG.startTimeMs * 100));
    fill.style.width = pct + '%';
    bar.classList.toggle('warn', pct <= 40 && pct > 18);
    bar.classList.toggle('crit', pct <= 18);
  }
  // Low-time urgency on the time stat itself.
  const ts = tm$('tmTimeStat');
  if (ts) {
    const low = TMG.timerStarted && TMG.state === 'playing' && TMG.timeMs <= 10000;
    ts.classList.toggle('warn', low && TMG.timeMs > 5000);
    ts.classList.toggle('crit', low && TMG.timeMs <= 5000);
  }
  // Level chip: "NAME · LV n" (compact, top-center under the timer).
  const chip = tm$('tmLvlChip');
  if (chip) chip.textContent = `${tmLevelCfg().name} · LV ${TMG.level + 1}`;
}
function tmTimeText() {
  const s = Math.max(0, TMG.timeMs) / 1000;
  return s >= 10 ? s.toFixed(0) : s.toFixed(1);
}

/** "+1" popup near the lumberjack. Epoch-guarded DOM element. */
function tmFloatScore() {
  const wrap = tm$('tmStageWrap');
  if (!wrap || TMG.reducedMotion) return;
  const el = document.createElement('span');
  el.className = 'tm-float';
  el.textContent = '+1';
  const r = wrap.getBoundingClientRect();
  el.style.left = (TMG.side === 'left' ? 0.30 : 0.70) * r.width + 'px';
  el.style.top = 0.52 * r.height + 'px';
  wrap.appendChild(el);
  const myEpoch = TMG.epoch;
  setTimeout(() => { if (myEpoch === TMG.epoch) el.remove(); }, 850);
}

/* ---- Particles ----------------------------------------------------------- */
function tmSpawnChips() {
  if (TMG.reducedMotion) return;
  // Logical coordinates: the trunk face beside the lumberjack.
  const cx = TM_VW / 2, cy = TM_VH * 0.60;
  const dir = TMG.side === 'left' ? 1 : -1;
  for (let i = 0; i < 7; i++) {
    TMG.particles.push({
      x: cx + dir * 30,
      y: cy + (Math.random() - 0.5) * 26,
      vx: dir * (1.4 + Math.random() * 2.6),
      vy: -(1 + Math.random() * 2.4),
      rot: Math.random() * Math.PI,
      vr: (Math.random() - 0.5) * 0.3,
      life: 1,
      size: 4 + Math.random() * 6,
    });
  }
}

/* ---- Canvas rendering ----------------------------------------------------- */
function tmInit(canvas) {
  TMG.canvas = canvas;
  TMG.ctx = canvas ? canvas.getContext('2d') : null;
  if (!canvas || !TMG.ctx) { TMG.canvas = null; return false; }
  tmResize();
  return true;
}
function tmResize() {
  const c = TMG.canvas;
  if (!c || !TMG.ctx) return;
  // The stage can measure 0 while hidden (display:none ancestor, tab
  // restore, mid-fullscreen relayout). Never adopt a zero-size backing
  // store: keep the last valid metrics and repaint into them.
  const wrap = c.parentElement;                    // .tm-stage
  const availW = wrap ? wrap.clientWidth : c.clientWidth;
  const availH = wrap ? wrap.clientHeight : c.clientHeight;
  if (availW < 40 || availH < 40) { tmDraw(performance.now()); return; }

  // PHASE 1 · Letterbox the fixed logical playfield (TM_VW x TM_VH)
  // into the available stage box: largest scale that fits BOTH axes,
  // centred — no distortion, no cropping, identical gameplay at every
  // viewport / orientation / fullscreen state.
  const pad = 8;                                   // breathing room inside the stage
  const fitW = Math.max(60, availW - pad * 2);
  const fitH = Math.max(60, availH - pad * 2);
  const k = Math.min(fitW / TM_VW, fitH / TM_VH);
  const cssW = Math.round(TM_VW * k), cssH = Math.round(TM_VH * k);

  TMG.dpr = Math.min(window.devicePixelRatio || 1, 2);
  TMG.w = cssW; TMG.h = cssH;                      // CSS px of the canvas box
  c.style.width = cssW + 'px';
  c.style.height = cssH + 'px';
  c.width = Math.round(cssW * TMG.dpr);
  c.height = Math.round(cssH * TMG.dpr);
  // One transform: all drawing stays in logical units; gameplay math is
  // untouched by any later resize.
  TMG.ctx.setTransform(cssW * TMG.dpr / TM_VW, 0, 0, cssH * TMG.dpr / TM_VH, 0, 0);
  // Resizing wipes the bitmap. When the loop is idle (ready/paused/over)
  // nothing will repaint it, so draw one frame synchronously here.
  if (TMG.state !== 'playing') tmDraw(performance.now());
}

function tmFrame(now) {
  if (TMG.epoch !== TMG._loopEpoch) { TMG.raf = 0; return; }
  TMG.raf = requestAnimationFrame(tmFrame);
  const dt = Math.min(50, now - (TMG.lastT || now));
  TMG.lastT = now;

  if (TMG.state === 'playing') {
    // The countdown runs only after the first successful chop.
    if (TMG.timerStarted) {
      TMG.timeMs -= dt;
      if (TMG.timeMs <= 0) { TMG.timeMs = 0; tmGameOver('time'); }
      else {
        // Urgent ticking in the final 10s; double-strength under 5s.
        const s = Math.ceil(TMG.timeMs / 1000);
        if (s <= 10 && s !== TMG._lastTickSec) {
          TMG._lastTickSec = s;
          tmSfx(s <= 5 ? 'tickcrit' : 'tick');
        }
      }
    }
    // Trunk slide-down animation progress (0 → 1).
    if (TMG.slideT < 1) {
      TMG.slideT = Math.min(1, TMG.slideT + dt / Math.max(30, tmLevelCfg().slideMs));
    }
    tmHud();
  }
  if (TMG.flashRed > 0) TMG.flashRed -= dt;
  if (TMG.shake > 0) TMG.shake -= dt;
  if (TMG.chopShake > 0) TMG.chopShake = Math.max(0, TMG.chopShake - dt);
  tmDraw(now);
}

function tmDraw(now) {
  const ctx = TMG.ctx;
  if (!ctx) return;
  // Fixed logical playfield: tmResize() installed one scale transform, so
  // every layer below draws in stable TM_VW x TM_VH units regardless of
  // the physical canvas size (no distortion at any aspect ratio).
  const W = TM_VW, H = TM_VH;

  ctx.clearRect(0, 0, W, H);

  // Death animation clock (drives the tumble in tmDrawMan).
  if (TMG.state === 'over') TMG.deathT += 16; else TMG.deathT = 0;

  // Camera shake: strong on game over, subtle micro-pulse on every chop.
  ctx.save();
  let shakeK = 0;
  if (TMG.shake > 0) shakeK = Math.max(shakeK, TMG.shake / 340);
  if (TMG.chopShake > 0) shakeK = Math.max(shakeK, (TMG.chopShake / 120) * 0.28);
  if (shakeK > 0) {
    ctx.translate((Math.random() - 0.5) * 12 * shakeK,
                  (Math.random() - 0.5) * 12 * shakeK);
  }

  // --- environment layers (back → front) ---
  tmDrawSky(ctx, W, H);            // gradient sky, moon, stars
  tmDrawHills(ctx, W, H);          // distant ridges
  tmDrawBgForest(ctx, W, H, now);  // background silhouettes + fog
  tmDrawMotes(ctx, W, H, now);     // restrained drifting motes
  tmDrawGround(ctx, W, H, now);    // grass field, tufts, stump shadow

  // --- tree ---
  const segH = tmSegH();
  const trunkW = 64;
  const cx = W / 2;
  const baseY = H * 0.86;
  tmDrawTree(ctx, cx, baseY, trunkW, segH, now);

  // --- lumberjack ---
  tmDrawMan(ctx, cx, baseY, trunkW, segH, now);

  // --- wood chip particles ---
  for (let i = TMG.particles.length - 1; i >= 0; i--) {
    const p = TMG.particles[i];
    p.x += p.vx; p.y += p.vy; p.vy += 0.18; p.rot += p.vr; p.life -= 0.025;
    if (p.life <= 0) { TMG.particles.splice(i, 1); continue; }
    ctx.save();
    ctx.globalAlpha = Math.max(0, p.life);
    ctx.translate(p.x, p.y);
    ctx.rotate(p.rot);
    // two-tone chip: bark outside, pale cut face
    ctx.fillStyle = '#8a5a32';
    ctx.fillRect(-p.size / 2, -p.size / 4, p.size, p.size / 2);
    ctx.fillStyle = '#e9c98f';
    ctx.fillRect(-p.size / 2 + 1, -p.size / 4 + 1, p.size - 2, p.size / 4);
    ctx.restore();
  }

  ctx.restore();

  // Vignette — gentle focus falloff, never touches the HUD zone.
  const vg = ctx.createRadialGradient(W / 2, H * 0.45, H * 0.34, W / 2, H * 0.5, H * 0.78);
  vg.addColorStop(0, 'rgba(0,0,0,0)');
  vg.addColorStop(1, 'rgba(0,0,0,.30)');
  ctx.fillStyle = vg;
  ctx.fillRect(0, 0, W, H);

  // Red hit-flash (drawn unshaken, over everything).
  if (TMG.flashRed > 0) {
    ctx.fillStyle = `rgba(255,77,109,${(TMG.flashRed / 320) * 0.35})`;
    ctx.fillRect(0, 0, W, H);
  }
}

/* ============================================================
   PHASE 3/4 · RENDERING
   The playfield has a FIXED logical coordinate system (TM_VW x TM_VH)
   exactly like Space Invaders' SI.W/SI.H: tmResize() letterboxes the
   canvas at the correct aspect ratio and installs one scale transform,
   so every draw call below works in stable logical units and gameplay
   math is untouched by any viewport, orientation or fullscreen change.
   ============================================================ */
const TM_VW = 480, TM_VH = 640;      // logical viewport (portrait arcade)

/** Deterministic pseudo-random for stable background detail. */
function tmHash(n) {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
}

/** Segment height in logical units (trunk slice). */
function tmSegH() { return 62; }

/* ---- sky, distant hills, background forest --------------------------- */
function tmDrawSky(ctx, W, H) {
  const sky = ctx.createLinearGradient(0, 0, 0, H);
  sky.addColorStop(0, '#08101f');
  sky.addColorStop(0.45, '#0d1a2e');
  sky.addColorStop(0.8, '#12233b');
  sky.addColorStop(1, '#15283f');
  ctx.fillStyle = sky;
  ctx.fillRect(-20, -20, W + 40, H + 40);

  // Moon glow — quiet anchor in the upper right.
  const mx = W * 0.78, my = H * 0.14;
  const g = ctx.createRadialGradient(mx, my, 4, mx, my, 90);
  g.addColorStop(0, 'rgba(214,232,255,.30)');
  g.addColorStop(0.35, 'rgba(160,190,230,.10)');
  g.addColorStop(1, 'rgba(160,190,230,0)');
  ctx.fillStyle = g;
  ctx.beginPath(); ctx.arc(mx, my, 90, 0, Math.PI * 2); ctx.fill();
  ctx.fillStyle = 'rgba(226,238,255,.75)';
  ctx.beginPath(); ctx.arc(mx, my, 17, 0, Math.PI * 2); ctx.fill();

  // Stars — sparse, deterministic, dim enough to ignore while playing.
  ctx.fillStyle = 'rgba(220,235,255,.5)';
  for (let i = 0; i < 26; i++) {
    const x = tmHash(i * 3.1) * W;
    const y = tmHash(i * 7.7) * H * 0.42;
    const tw = 0.55 + 0.45 * Math.sin(performance.now() / 1400 + i * 2.4);
    ctx.globalAlpha = 0.16 + 0.2 * tw;
    ctx.fillRect(x, y, 1.6, 1.6);
  }
  ctx.globalAlpha = 1;
}

/** Distant hill layers (parallax bands, static — depth without noise). */
function tmDrawHills(ctx, W, H) {
  const horizon = H * 0.66;
  // far ridge
  ctx.fillStyle = 'rgba(24,48,74,.55)';
  ctx.beginPath();
  ctx.moveTo(0, horizon);
  for (let i = 0; i <= 8; i++) {
    const x = (i / 8) * W;
    ctx.quadraticCurveTo(x - W / 16, horizon - 30 - tmHash(i * 5.3) * 44, x, horizon - 6 - tmHash(i * 9.1) * 18);
  }
  ctx.lineTo(W, H); ctx.lineTo(0, H); ctx.closePath(); ctx.fill();
  // near ridge
  ctx.fillStyle = 'rgba(17,38,60,.75)';
  ctx.beginPath();
  ctx.moveTo(0, horizon + 34);
  for (let i = 0; i <= 6; i++) {
    const x = (i / 6) * W;
    ctx.quadraticCurveTo(x - W / 12, horizon + 10 - tmHash(i * 3.7 + 40) * 36, x, horizon + 30 - tmHash(i * 6.1 + 40) * 14);
  }
  ctx.lineTo(W, H); ctx.lineTo(0, H); ctx.closePath(); ctx.fill();
}

/** Background tree line: dark silhouettes flanking the play column.
 *  Drawn once per frame but cheap (simple triangles + ellipse crowns),
 *  deterministic positions, well outside the trunk's collision lane. */
function tmDrawBgForest(ctx, W, H, now) {
  const groundY = H * 0.86;
  // left cluster
  const clusters = [
    { x: -14, s: 1.00 }, { x: 52, s: 0.82 }, { x: 116, s: 0.66 },
    { x: W + 14, s: 1.04 }, { x: W - 54, s: 0.84 }, { x: W - 118, s: 0.64 },
  ];
  for (const c of clusters) {
    const hgt = 150 * c.s, wdt = 46 * c.s;
    // trunk silhouette
    ctx.fillStyle = 'rgba(13,26,38,.85)';
    ctx.fillRect(c.x - 5 * c.s, groundY - hgt, 10 * c.s, hgt);
    // crown (two stacked triangles)
    ctx.fillStyle = 'rgba(15,42,44,.8)';
    ctx.beginPath();
    ctx.moveTo(c.x, groundY - hgt - 34 * c.s);
    ctx.lineTo(c.x - wdt * 0.6, groundY - hgt * 0.62);
    ctx.lineTo(c.x + wdt * 0.6, groundY - hgt * 0.62);
    ctx.closePath(); ctx.fill();
    ctx.beginPath();
    ctx.moveTo(c.x, groundY - hgt * 0.80);
    ctx.lineTo(c.x - wdt * 0.75, groundY - hgt * 0.30);
    ctx.lineTo(c.x + wdt * 0.75, groundY - hgt * 0.30);
    ctx.closePath(); ctx.fill();
  }
  // soft fog band over the forest base — depth cue
  const fog = ctx.createLinearGradient(0, groundY - 70, 0, groundY + 10);
  fog.addColorStop(0, 'rgba(120,160,190,0)');
  fog.addColorStop(1, 'rgba(120,160,190,.10)');
  ctx.fillStyle = fog;
  ctx.fillRect(0, groundY - 70, W, 80);
}

/** Restrained atmospheric particles: drifting motes/fireflies.
 *  Persistent pool, ~14 items, gentle sine drift — never near the HUD. */
const TM_MOTES = [];
function tmDrawMotes(ctx, W, H, now) {
  if (TMG.reducedMotion) return;
  if (TM_MOTES.length === 0) {
    for (let i = 0; i < 14; i++) {
      TM_MOTES.push({
        x: tmHash(i * 13.3) * W, y: tmHash(i * 29.7) * H,
        r: 1 + tmHash(i * 7.1) * 1.6,
        ph: tmHash(i * 3.3) * Math.PI * 2,
        sp: 0.02 + tmHash(i * 11.9) * 0.05,
      });
    }
  }
  for (const m of TM_MOTES) {
    const y = (m.y - now * m.sp * 0.06) % H;
    const yy = y < 0 ? y + H : y;
    const x = m.x + Math.sin(now / 2600 + m.ph) * 14;
    const a = 0.10 + 0.13 * (0.5 + 0.5 * Math.sin(now / 900 + m.ph * 2));
    ctx.fillStyle = `rgba(180,230,200,${a})`;
    ctx.beginPath(); ctx.arc(x, yy, m.r, 0, Math.PI * 2); ctx.fill();
  }
}

/* ---- ground ----------------------------------------------------------- */
function tmDrawGround(ctx, W, H, now) {
  const gy = H * 0.86;
  // grass field with subtle vertical gradient
  const g = ctx.createLinearGradient(0, gy, 0, H);
  g.addColorStop(0, '#1c4229');
  g.addColorStop(0.25, '#173a23');
  g.addColorStop(1, '#102a19');
  ctx.fillStyle = g;
  ctx.fillRect(-20, gy, W + 40, H - gy + 20);
  // lit top edge
  ctx.fillStyle = 'rgba(94,242,192,.14)';
  ctx.fillRect(-20, gy, W + 40, 3);
  // grass tufts — deterministic, tiny, no motion
  ctx.strokeStyle = 'rgba(94,242,192,.16)';
  ctx.lineWidth = 1.5;
  for (let i = 0; i < 22; i++) {
    const x = 8 + tmHash(i * 17.3) * (W - 16);
    const y = gy + 8 + tmHash(i * 31.1) * (H - gy - 18);
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.lineTo(x + (tmHash(i * 5.5) - 0.5) * 4, y - 5);
    ctx.stroke();
  }
  // stump shadow under the tree
  ctx.fillStyle = 'rgba(0,0,0,.28)';
  ctx.beginPath();
  ctx.ellipse(W / 2, gy + 6, 78, 12, 0, 0, Math.PI * 2);
  ctx.fill();
}

/* ---- tree ------------------------------------------------------------- */
function tmDrawTree(ctx, cx, baseY, trunkW, segH, now) {
  // Trunk slide-down: after a chop the whole stack eases from one segment
  // higher to its resting position (slideT 0 → 1). Reduced-motion skips it.
  const slide = (TMG.reducedMotion || TMG.slideT >= 1) ? 1 : TMG.slideT;
  const ease = 1 - Math.pow(1 - slide, 3);            // ease-out cubic
  const offY = (1 - ease) * segH;                     // px still to travel

  const barkLight = '#a87243', barkMid = '#8a5a32', barkDark = '#6b4526';

  for (let i = 0; i < TMG.trunk.length; i++) {
    const seg = TMG.trunk[i];
    const y = baseY - i * segH + offY;

    // --- trunk slice with vertical bark gradient ---
    const g = ctx.createLinearGradient(cx - trunkW / 2, 0, cx + trunkW / 2, 0);
    g.addColorStop(0, barkDark);
    g.addColorStop(0.28, barkMid);
    g.addColorStop(0.5, barkLight);
    g.addColorStop(0.72, barkMid);
    g.addColorStop(1, '#5a3a1e');
    ctx.fillStyle = g;
    tmRoundRect(ctx, cx - trunkW / 2, y - segH, trunkW, segH - 2, 7);
    ctx.fill();

    // Bark texture: deterministic vertical fissures + cut chevron on the
    // bottom face of the topmost (just-chopped) segment.
    ctx.strokeStyle = 'rgba(40,22,8,.38)';
    ctx.lineWidth = 2;
    for (let k = 0; k < 3; k++) {
      const fx = cx + (k - 1) * trunkW * 0.26 + (tmHash(i * 9 + k * 3) - 0.5) * 5;
      ctx.beginPath();
      ctx.moveTo(fx, y - segH + 6);
      ctx.lineTo(fx + (tmHash(i * 4 + k) - 0.5) * 4, y - 6);
      ctx.stroke();
    }
    // lighter highlight stripe
    ctx.strokeStyle = 'rgba(255,220,170,.14)';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(cx - trunkW * 0.3, y - segH + 5);
    ctx.lineTo(cx - trunkW * 0.3, y - 5);
    ctx.stroke();

    // segment seam
    ctx.fillStyle = 'rgba(30,16,6,.5)';
    ctx.fillRect(cx - trunkW / 2 + 3, y - 3, trunkW - 6, 2);

    // --- branch: tapered arm + layered leaf clusters ---
    if (seg.branch) {
      const dir = seg.branch === 'left' ? -1 : 1;
      const by = y - segH * 0.68;
      const len = trunkW * 1.62;
      ctx.save();
      ctx.translate(cx + dir * trunkW / 2, by);
      // arm: two tapering rects read as a chunky pixel branch
      const bg = ctx.createLinearGradient(0, 0, dir * len, 0);
      bg.addColorStop(0, barkMid);
      bg.addColorStop(1, '#5f3d1f');
      ctx.fillStyle = bg;
      tmRoundRect(ctx, dir < 0 ? -len : 0, -8, len, 16, 8);
      ctx.fill();
      // underside shading
      ctx.fillStyle = 'rgba(0,0,0,.22)';
      tmRoundRect(ctx, dir < 0 ? -len : 0, 1, len, 7, 5);
      ctx.fill();
      // knot
      ctx.fillStyle = 'rgba(50,28,10,.55)';
      ctx.beginPath();
      ctx.arc(dir * len * 0.3, -1, 3.4, 0, Math.PI * 2);
      ctx.fill();
      // leaves: three overlapping ellipses, dark→light for volume
      ctx.fillStyle = '#256b43';
      ctx.beginPath();
      ctx.ellipse(dir * len * 0.95, -3, 22, 14, dir * 0.28, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = '#2f8a56';
      ctx.beginPath();
      ctx.ellipse(dir * len * 0.86, -11, 15, 9.5, dir * 0.22, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = '#3fa86a';
      ctx.beginPath();
      ctx.ellipse(dir * len * 1.02, -8, 9, 6, dir * 0.3, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    }
  }

  // --- canopy at the very top ---
  const topY = baseY - TMG.trunk.length * segH + offY;
  const sway = TMG.reducedMotion ? 0 : Math.sin(now / 1900) * 3;
  ctx.save();
  ctx.translate(sway, 0);
  ctx.fillStyle = '#1f5c3a';
  ctx.beginPath();
  ctx.ellipse(cx, topY + 8, trunkW * 1.62, trunkW * 1.02, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#256b43';
  ctx.beginPath();
  ctx.ellipse(cx - trunkW * 0.55, topY - 3, trunkW * 0.95, trunkW * 0.62, -0.2, 0, Math.PI * 2);
  ctx.fill();
  ctx.beginPath();
  ctx.ellipse(cx + trunkW * 0.58, topY - 1, trunkW * 0.9, trunkW * 0.58, 0.25, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#2f8a56';
  ctx.beginPath();
  ctx.ellipse(cx + trunkW * 0.1, topY - 12, trunkW * 0.7, trunkW * 0.5, 0.05, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

/* ---- lumberjack ------------------------------------------------------- */
/** Lumberjack render state (side-slide easing + death animation). */
TMG.sideAnim = { from: 'left', t: 1 };
TMG.deathT = 0;

function tmDrawMan(ctx, cx, baseY, trunkW, segH, now) {
  const dead = TMG.state === 'over';
  const dir = TMG.side === 'left' ? -1 : 1;

  // Smooth side transition: ease between the previous and current side.
  if (TMG.sideAnim.t < 1) {
    TMG.sideAnim.t = Math.min(1, TMG.sideAnim.t + 0.28);
  }
  const fromDir = TMG.sideAnim.from === 'left' ? -1 : 1;
  const mix = 1 - Math.pow(1 - TMG.sideAnim.t, 2);
  const pos = fromDir + (dir - fromDir) * mix;        // -1 .. 1 continuous
  const x = cx + pos * (trunkW / 2 + 30);

  const feetY = baseY + 4;

  // Axe swing: 0..1 progress since the last chop (wind-up → strike).
  const since = now - TMG.lastChopAt;
  let swing = 0;
  if (TMG.state === 'playing' && since < 230) {
    swing = Math.sin((since / 230) * Math.PI);
  }

  // Death: tumble away from the fatal branch + fade to ground.
  let deathK = 0, deathRot = 0, deathDy = 0;
  if (dead) {
    deathK = Math.min(1, TMG.deathT / 700);
    deathRot = dir * deathK * 1.9;                    // fall backwards
    deathDy = deathK * 26;
  }

  // Idle bob when waiting (ready screen) — keeps the scene alive.
  const bob = (!dead && TMG.state === 'ready' && !TMG.reducedMotion)
    ? Math.sin(now / 520) * 2 : 0;

  ctx.save();
  ctx.translate(x, feetY + deathDy);
  ctx.rotate(deathRot);
  ctx.scale(dir < 0 ? -1 : 1, 1);

  // shadow (drawn before body, unrotated feel)
  if (!dead) {
    ctx.fillStyle = 'rgba(0,0,0,.3)';
    ctx.beginPath(); ctx.ellipse(0, 2, 15, 4.5, 0, 0, Math.PI * 2); ctx.fill();
  }

  // ---- legs (blue jeans) ----
  ctx.strokeStyle = '#2c3a55'; ctx.lineWidth = 7.5; ctx.lineCap = 'round';
  const stride = swing > 0.4 ? (swing - 0.4) * 8 : 0;   // brace during strike
  ctx.beginPath(); ctx.moveTo(-1, -27); ctx.lineTo(-8 - stride, -1); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(1, -27); ctx.lineTo(9 + stride, -1); ctx.stroke();
  // boots
  ctx.strokeStyle = '#3a2a1a'; ctx.lineWidth = 5.5;
  ctx.beginPath(); ctx.moveTo(-8 - stride, -2); ctx.lineTo(-10 - stride, 0); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(9 + stride, -2); ctx.lineTo(11 + stride, 0); ctx.stroke();

  // ---- torso: red plaid shirt ----
  ctx.fillStyle = '#d64545';
  tmRoundRect(ctx, -12, -60, 24, 35, 8); ctx.fill();
  // plaid weave
  ctx.fillStyle = 'rgba(255,255,255,.16)';
  ctx.fillRect(-12, -53, 24, 3); ctx.fillRect(-12, -44, 24, 3); ctx.fillRect(-12, -35, 24, 3);
  ctx.fillStyle = 'rgba(120,20,20,.28)';
  ctx.fillRect(-4, -60, 3, 35); ctx.fillRect(4, -60, 3, 35);
  // belt
  ctx.fillStyle = '#3a2a1a';
  ctx.fillRect(-12, -28, 24, 5);
  ctx.fillStyle = '#ffd166';
  ctx.fillRect(-2, -27.5, 5, 4);

  // ---- head + helmet ----
  ctx.fillStyle = '#f2c19a';
  ctx.beginPath(); ctx.arc(0, -70, 10.5, 0, Math.PI * 2); ctx.fill();
  // ear
  ctx.beginPath(); ctx.arc(-4, -69, 2.2, 0, Math.PI * 2); ctx.fillStyle = '#e0a87e'; ctx.fill();
  // eye + smile (hidden when dead — X eyes)
  if (dead) {
    ctx.strokeStyle = '#5a3a2a'; ctx.lineWidth = 1.8;
    ctx.beginPath(); ctx.moveTo(2, -73); ctx.lineTo(7, -68); ctx.moveTo(7, -73); ctx.lineTo(2, -68); ctx.stroke();
  } else {
    ctx.fillStyle = '#2b2b2b';
    ctx.fillRect(3, -73, 2.4, 2.8);
    ctx.strokeStyle = '#a3704d'; ctx.lineWidth = 1.4;
    ctx.beginPath(); ctx.arc(4, -66, 3, 0.15, Math.PI - 0.5); ctx.stroke();
  }
  // beard
  ctx.fillStyle = '#8a5a3a';
  ctx.beginPath(); ctx.arc(1, -63, 6.5, 0, Math.PI); ctx.fill();
  // helmet (yellow hard hat with brim)
  ctx.fillStyle = '#ffb703';
  ctx.beginPath(); ctx.arc(0, -72, 11, Math.PI, 0); ctx.fill();
  ctx.fillRect(-13, -73, 26, 4.5);
  ctx.fillStyle = 'rgba(255,255,255,.25)';
  ctx.fillRect(-6, -79, 4, 3);

  // ---- arms + axe ----
  // rear arm (static grip)
  ctx.strokeStyle = '#d64545'; ctx.lineWidth = 6.5;
  ctx.beginPath(); ctx.moveTo(-3, -55); ctx.lineTo(6, -47); ctx.stroke();
  ctx.strokeStyle = '#f2c19a'; ctx.lineWidth = 5.5;
  ctx.beginPath(); ctx.moveTo(6, -47); ctx.lineTo(13, -42); ctx.stroke();

  // front arm + axe: wind-up raised, strike sweeps into the trunk.
  const ax = 12 + swing * 13;
  const ay = -50 + swing * 17;
  ctx.strokeStyle = '#d64545'; ctx.lineWidth = 6.5;
  ctx.beginPath(); ctx.moveTo(3, -53); ctx.lineTo(ax, ay); ctx.stroke();
  ctx.strokeStyle = '#f2c19a'; ctx.lineWidth = 5.5;
  ctx.beginPath(); ctx.moveTo(ax, ay); ctx.lineTo(ax + 8, ay + 3); ctx.stroke();
  // axe handle (wood, two-tone)
  ctx.strokeStyle = '#8a5c33'; ctx.lineWidth = 5;
  ctx.beginPath(); ctx.moveTo(ax + 2, ay + 1); ctx.lineTo(ax + 21, ay - 13); ctx.stroke();
  ctx.strokeStyle = 'rgba(255,220,170,.3)'; ctx.lineWidth = 1.6;
  ctx.beginPath(); ctx.moveTo(ax + 4, ay); ctx.lineTo(ax + 19, ay - 11); ctx.stroke();
  // axe head: steel blade + bright edge
  ctx.save();
  ctx.translate(ax + 21, ay - 13);
  ctx.rotate(0.62 - swing * 0.95);
  ctx.fillStyle = '#9fb0c4';
  tmRoundRect(ctx, -5, -10, 16, 20, 3); ctx.fill();
  ctx.fillStyle = '#e6eef8';
  ctx.beginPath();
  ctx.moveTo(11, -9); ctx.quadraticCurveTo(19, 0, 11, 9); ctx.closePath(); ctx.fill();
  ctx.restore();

  ctx.restore();
}

function tmRoundRect(ctx, x, y, w, h, r) {
  r = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/* ---- View ---------------------------------------------------------------- */
function renderTmView() {
  document.getElementById('viewRoot').innerHTML = `
  <div class="content"><div class="content-inner">
    <section class="ft-panel ft-game tm-wide" id="tmPanel" aria-label="Timber Man Ultra arcade game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Timber Man Ultra</div>
          <div class="ft-blurb">Chop fast, dodge branches, race the clock. Every chop buys time — but the refund shrinks as you climb.</div>
        </div>
      </header>

      <!-- PHASE 1/2 · The stage IS the screen: canvas letterboxed inside,
           compact translucent HUD overlaid on top. No bottom control bar. -->
      <div class="tm-stage" id="tmStageWrap" data-zones="0">
        <canvas id="tmCanvas" aria-label="Timber Man play field"
                role="img">Your browser does not support the canvas element.</canvas>

        <!-- overlay HUD -->
        <div class="tm-hud" role="status" aria-live="off">
          <div class="tm-hud-l">
            <span class="tm-stat score"><span class="k">score</span><span class="v" id="tmScore">0</span></span>
          </div>
          <div class="tm-hud-c">
            <span class="tm-stat time" id="tmTimeStat"><span class="k">time</span><span class="v" id="tmTime">60.0</span></span>
            <span class="tm-time-bar" id="tmTimeBar" aria-hidden="true"><i id="tmTimeFill"></i></span>
            <span class="tm-lvl-chip" id="tmLvlChip">EASY · LV 1</span>
          </div>
          <div class="tm-hud-r">
            <span class="tm-stat best"><span class="k">best</span><span class="v" id="tmBestV">0</span></span>
            <button type="button" class="tm-ibtn" id="tmFsBtn" aria-label="Toggle fullscreen" title="Fullscreen (F)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
                   stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M16 3h3a2 2 0 0 1 2 2v3"/>
                <path d="M8 21H5a2 2 0 0 1-2-2v-3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>
              </svg>
            </button>
            <button type="button" class="tm-ibtn" id="tmGearBtn" aria-label="Settings" title="Settings">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                   stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
              </svg>
            </button>
          </div>
        </div>

        <!-- mobile touch-zone hints (ready state only) -->
        <div class="tm-zones" aria-hidden="true"><span>◀ tap left</span><span>tap right ▶</span></div>

        <!-- status overlay: ready / paused / game over -->
        <div class="tm-overlay" id="tmOverlay" role="dialog" aria-modal="false" aria-label="Game status">
          <div class="tm-ov-inner">
            <div class="tm-ov-title" id="tmOvTitle">Timber Man Ultra</div>
            <div class="tm-ov-sub" id="tmOvSub"></div>
            <div class="tm-ov-keys hidden" id="tmOvKeys"></div>
            <div class="tm-ov-stats hidden" id="tmOvStats">
              <div class="tm-ov-stat hl"><span class="k">score</span><span class="v" id="tmRsScore">0</span></div>
              <div class="tm-ov-stat"><span class="k">best</span><span class="v" id="tmRsBest">0</span></div>
            </div>
            <div class="tm-ov-row">
              <button type="button" class="tm-ov-btn" id="tmOvBtn" data-kind="start">Start Chopping</button>
              <button type="button" class="tm-ov-btn ghost hidden" id="tmOvExit">Back to Games</button>
            </div>
            <div class="tm-ov-best" id="tmOvBest"></div>
          </div>
        </div>

        <!-- PHASE 6 · settings overlay (pauses gameplay) -->
        <div class="tm-settings hidden" id="tmSettings" role="dialog" aria-modal="true" aria-label="Settings">
          <div class="tm-set-card">
            <div class="tm-set-head">
              <span class="tm-set-title">Settings</span>
              <button type="button" class="tm-set-x" id="tmSetClose" aria-label="Close settings">✕</button>
            </div>
            <button type="button" class="tm-set-btn primary" id="tmSetResume">Resume</button>
            <button type="button" class="tm-set-btn" id="tmSetRestart">Restart Round</button>
            <button type="button" class="tm-set-btn" id="tmSetFs">Enter Fullscreen</button>
            <div class="tm-set-sep" role="presentation"></div>
            <div class="tm-au-row">
              <label class="tm-au-l" for="tmAuMusicV">Music</label>
              <input type="range" id="tmAuMusicV" class="tm-au-slider"
                     min="0" max="100" step="1" aria-label="Music volume">
              <span class="tm-au-val" id="tmAuMusicVal"></span>
            </div>
            <div class="tm-au-row">
              <label class="tm-au-l" for="tmAuSfxV">Effects</label>
              <input type="range" id="tmAuSfxV" class="tm-au-slider"
                     min="0" max="100" step="1" aria-label="Sound effects volume">
              <span class="tm-au-val" id="tmAuSfxVal"></span>
            </div>
            <div class="tm-au-toggles">
              <button type="button" class="tm-au-toggle" id="tmAuMusicT"
                      aria-pressed="true"><span class="tm-au-dot" aria-hidden="true"></span>Music</button>
              <button type="button" class="tm-au-toggle" id="tmAuSfxT"
                      aria-pressed="true"><span class="tm-au-dot" aria-hidden="true"></span>Effects</button>
            </div>
            <div class="tm-set-sep" role="presentation"></div>
            <button type="button" class="tm-set-btn danger" id="tmSetExit">Exit to Games</button>
            <div class="tm-set-hint">P pause · R restart · F fullscreen</div>
          </div>
        </div>
      </div>
    </section>
  </div></div>`;

  TMG.epoch++;
  const myEpoch = TMG.epoch;
  TMG._loopEpoch = myEpoch;
  TMG.cleanup = [];
  TMG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const tmOnLocal = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    TMG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };

  // Exactly one loop per mount: cancel any straggler from a previous view
  // before starting this session's chain.
  if (TMG.raf) { cancelAnimationFrame(TMG.raf); TMG.raf = 0; }

  tmReset();
  // Best score is read fresh from storage on every mount (a previous
  // session in this tab may have raised it; TMG.best is mount-scoped).
  TMG.best = +(localStorage.getItem('tmBest') || 0) || TMG.best;
  const okInit = tmInit(document.getElementById('tmCanvas'));
  if (!okInit) {
    // Surface init failure honestly instead of a permanently blank stage.
    document.getElementById('viewRoot').innerHTML =
      `<div class="content"><div class="content-inner">
         <section class="ft-panel ft-game tm-wide" id="tmPanel"
              aria-label="Timber Man Ultra failed to start">
           <header class="ft-panel-h">
             <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
             <div class="ft-panel-t">
               <div class="ft-name">Timber Man Ultra could not start</div>
               <div class="ft-blurb">This browser refused to create a 2D canvas context. Try reloading the page, or use a different browser.</div>
             </div>
           </header>
         </section>
       </div></div>`;
    return;
  }
  // Paint persisted audio prefs into the settings card (sliders/toggles)
  // at mount, and again whenever the card opens.
  if (typeof TMG._auSync === 'function') TMG._auSync();
  tmOverlay('ready');
  tmHud();

  // Any real user gesture inside the game may unlock audio (autoplay
  // policy): one delegated capture-phase pointerdown covers every button,
  // pad and canvas click without touching each handler.
  tmOnLocal(document.body, 'pointerdown', () => {
    if (TMG.epoch !== myEpoch) return;
    tmAudioUnlock();
  }, true);
  tmOnLocal(document, 'keydown', () => {
    if (TMG.epoch !== myEpoch) return;
    tmAudioUnlock();
  }, true);

  /* ---- PHASE 6 · keyboard ---------------------------------------------
     Escape is owned by the shared session controller (ftSessionBind):
     one press leaves fullscreen AND returns to Fun Time. */
  const GAME_KEYS = new Set(['ArrowLeft','ArrowRight','a','d','A','D',
    'p','P','r','R','f','F','Escape']);
  tmOnLocal(document, 'keydown', e => {
    if (TMG.epoch !== myEpoch) return;
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (e.key === 'ArrowLeft' || e.key === 'a' || e.key === 'A') {
      if (!setOpen) { if (TMG.state === 'playing') tmChop('left'); else if (TMG.state === 'ready' || TMG.state === 'over') tmStart(); }
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowRight' || e.key === 'd' || e.key === 'D') {
      if (!setOpen) { if (TMG.state === 'playing') tmChop('right'); else if (TMG.state === 'ready' || TMG.state === 'over') tmStart(); }
      e.preventDefault(); return;
    }
    if (e.key === 'p' || e.key === 'P') {
      if (setOpen) setClose(); else tmTogglePause();
      e.preventDefault(); return;
    }
    if (e.key === 'r' || e.key === 'R') { tmRestart(); e.preventDefault(); return; }
    if (e.key === 'f' || e.key === 'F') { tmToggleFullscreen(); e.preventDefault(); return; }
    if (GAME_KEYS.has(e.key)) e.preventDefault();
  });

  // Held keys must not survive a blur or tab switch.
  tmOnLocal(window, 'blur', () => { if (TMG.state === 'playing' && !setOpen) tmTogglePause(); });
  tmOnLocal(document, 'visibilitychange', () => {
    if (document.hidden) {
      if (TMG.state === 'playing') tmTogglePause();
      if (setOpen) setClose();
    }
  });

  /* ---- PHASE 6 · settings overlay --------------------------------------
     Compact gear in the top-right HUD opens a full settings card:
     Resume / Restart / Fullscreen / music+effects / Exit. Opening it
     PAUSES gameplay; closing resumes only if it was auto-paused. */
  const setEl = tm$('tmSettings');
  let setOpen = false, setPausedByOpen = false;

  function setPaintFs() {
    const b = tm$('tmSetFs');
    if (b) b.textContent = (typeof ftIsFullscreen === 'function' && ftIsFullscreen())
      ? 'Leave Fullscreen' : 'Enter Fullscreen';
  }
  function setClose() {
    if (!setEl) return;
    setEl.classList.add('hidden');
    setOpen = false;
    // Resume only when the settings card itself paused the round.
    if (setPausedByOpen && TMG.state === 'paused') tmTogglePause();
    setPausedByOpen = false;
  }
  function setOpenFn() {
    if (!setEl) return;
    tmAudioUnlock();
    setPausedByOpen = TMG.state === 'playing';
    if (TMG.state === 'playing') tmTogglePause();   // pause gameplay
    setEl.classList.remove('hidden');
    setOpen = true;
    setPaintFs();
    if (TMG._auSync) TMG._auSync();
    tmSfx('ui');
  }
  TMG._setClose = setClose;                         // teardown safety

  tmOnLocal(tm$('tmGearBtn'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    setOpen ? setClose() : setOpenFn();
  });
  tmOnLocal(tm$('tmSetClose'), 'click', () => { if (TMG.epoch === myEpoch) setClose(); });
  tmOnLocal(tm$('tmSetResume'), 'click', () => { if (TMG.epoch === myEpoch) setClose(); });
  tmOnLocal(tm$('tmSetRestart'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    setClose();
    setPausedByOpen = false;                        // restart owns the state
    tmRestart();
    tmStart();
  });
  tmOnLocal(tm$('tmSetFs'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    tmToggleFullscreen();
    setTimeout(setPaintFs, 60);
  });
  tmOnLocal(tm$('tmSetExit'), 'click', () => { location.hash = '#funtime'; });
  // Settings closes on outside pointer press (but not on its own children).
  tmOnLocal(document, 'pointerdown', e => {
    if (TMG.epoch !== myEpoch || !setOpen) return;
    if (setEl.contains(e.target) || tm$('tmGearBtn').contains(e.target)) return;
    setClose();
  }, true);

  /* ---- audio controls (inside the settings card) -----------------------
     State lives in TMA.settings (persisted under 'tm-audio-v1'); the UI is
     a plain read/write of that object. Changes apply immediately through
     click-free gain ramps. */
  const auSyncUi = () => {
    const a = TMA.settings;
    const set = (id, v) => { const el = tm$(id); if (el) el.value = String(Math.round(v * 100)); };
    const txt = (id, v) => { const el = tm$(id); if (el) el.textContent = Math.round(v * 100) + '%'; };
    set('tmAuMusicV', a.mv); txt('tmAuMusicVal', a.mv);
    set('tmAuSfxV', a.sv);   txt('tmAuSfxVal', a.sv);
    const tg = (id, on) => { const el = tm$(id); if (el) el.setAttribute('aria-pressed', String(!!on)); };
    tg('tmAuMusicT', a.mu); tg('tmAuSfxT', a.sx);
  };
  TMG._auSync = auSyncUi;

  // volume sliders — apply live, persist every change (cheap writes)
  const auSlider = (inputId, key, valId) => {
    const el = tm$(inputId); if (!el) return;
    tmOnLocal(el, 'input', () => {
      TMA.settings[key] = Math.max(0, Math.min(1, Number(el.value) / 100));
      tmAudioApplyVolumes(); tmAudioSavePrefs(); auSyncUi();
    });
  };
  auSlider('tmAuMusicV', 'mv', 'tmAuMusicVal');
  auSlider('tmAuSfxV', 'sv', 'tmAuSfxVal');

  // toggles — instant mute via the gain graph (no clicks, no restart)
  tmOnLocal(tm$('tmAuMusicT'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    TMA.settings.mu = TMA.settings.mu ? 0 : 1;
    tmAudioApplyVolumes(); tmAudioSavePrefs(); auSyncUi();
    tmSfx('ui');
  });
  tmOnLocal(tm$('tmAuSfxT'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    TMA.settings.sx = TMA.settings.sx ? 0 : 1;
    tmAudioApplyVolumes(); tmAudioSavePrefs(); auSyncUi();
    tmSfx('ui');                                    // plays only if now on
  });

  // Fullscreen icon button (top-right HUD).
  tmOnLocal(tm$('tmFsBtn'), 'click', () => tmToggleFullscreen());
  if (typeof ftFsSupported === 'function' && !ftFsSupported()) {
    const b = tm$('tmFsBtn'); if (b) b.style.display = 'none';
  }
  /* Reflect genuine fullscreen state onto the body class + button label
     (same convention as Snakes/Ludo). This is what makes the BUTTON path
     apply the .tm-fs layout; without it the canvas stayed letterboxed
     after entering fullscreen via the button or F key. */
  const tmFsReflect = () => {
    if (typeof ftFsReflect === 'function') ftFsReflect('tm', 'tm-fs', null);
    setPaintFs();
    tmResize();
  };
  tmOnLocal(document, 'fullscreenchange', tmFsReflect);
  tmOnLocal(document, 'webkitfullscreenchange', tmFsReflect);

  // Overlay buttons: Start / Resume / Chop Again + Back to Games.
  tmOnLocal(tm$('tmOvBtn'), 'click', () => {
    const kind = tm$('tmOvBtn').dataset.kind;
    if (kind === 'again') { tmRestart(); tmStart(); }
    else tmStart();
  });
  tmOnLocal(tm$('tmOvExit'), 'click', () => { location.hash = '#funtime'; });
  // Tapping anywhere on the ready/game-over overlay also starts/resumes —
  // the same gesture arcade cabinets use. (The buttons still work: this
  // handler only fires for taps that did NOT land on a control.)
  tmOnLocal(tm$('tmOverlay'), 'pointerdown', e => {
    if (TMG.epoch !== myEpoch) return;
    if (e.target.closest('button')) return;         // controls own their taps
    const kind = tm$('tmOvBtn').dataset.kind;
    if (kind === 'resume') tmTogglePause();
    else if (kind === 'start' || kind === 'again') {
      if (kind === 'again') { tmRestart(); }
      tmStart();
    }
  });

  /* ---- PHASE 6 · pointer chopping (desktop + mobile) --------------------
     The stage is split into left/right halves. Any pointer type chops the
     half it lands in: desktop mouse clicks, tablet pens and touch all use
     the same path — large comfortable targets by construction, with
     continuous repeat support while held (mobile) and correct release
     handling via pointerup/pointercancel. */
  const stageWrap = tm$('tmStageWrap');
  const chopFromEvent = e => {
    if (e.pointerType === 'mouse' && e.button !== 0) return false;
    const r = stageWrap.getBoundingClientRect();
    return (e.clientX - r.left < r.width / 2) ? 'left' : 'right';
  };
  tmOnLocal(stageWrap, 'pointerdown', e => {
    if (TMG.epoch !== myEpoch) return;
    // UI layers own their own events.
    if (e.target.closest('.tm-ibtn, .tm-overlay, .tm-settings, button')) return;
    e.preventDefault();
    tmAudioUnlock();
    if (TMG.state === 'ready' || TMG.state === 'over') { tmStart(); return; }
    if (TMG.state === 'paused') { tmTogglePause(); return; }
    tmChop(chopFromEvent(e));
  });
  // Continuous input: hold-to-chop repeats at a comfortable arcade rate.
  // The interval handle is registered in TMG.cleanup so tmTeardown() (any
  // route change / remount) always clears it — a leaked interval would
  // keep chopping into the NEXT session's global TMG state.
  TMG.holdTimer = 0;
  const holdStop = () => {
    if (TMG.holdTimer) { clearInterval(TMG.holdTimer); TMG.holdTimer = 0; }
  };
  TMG.cleanup.push(holdStop);                       // teardown safety net
  tmOnLocal(stageWrap, 'pointerup', holdStop);
  tmOnLocal(stageWrap, 'pointercancel', holdStop);
  tmOnLocal(stageWrap, 'pointerleave', holdStop);
  tmOnLocal(window, 'blur', holdStop);
  tmOnLocal(stageWrap, 'pointerdown', e => {
    if (TMG.epoch !== myEpoch) return;
    if (e.target.closest('.tm-ibtn, .tm-overlay, .tm-settings, button')) return;
    if (e.pointerType === 'touch' && TMG.state === 'playing') {
      holdStop();
      const side0 = chopFromEvent(e);
      TMG.holdTimer = setInterval(() => {
        if (TMG.epoch !== myEpoch || TMG.state !== 'playing') { holdStop(); return; }
        tmChop(side0);
      }, 240);
    }
  });
  tmOnLocal(stageWrap, 'contextmenu', e => e.preventDefault());

  // ---- resize + loop ----
  tmOnLocal(window, 'resize', () => tmResize());

  TMG.lastT = performance.now();
  // Mount-time chain: guarded by the `if (TMG.raf)` cancel above, this is
  // the only live RAF handle for this session.
  TMG.raf = requestAnimationFrame(tmFrame);

  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, dashboard chrome hidden while playing.
  if (typeof ftSessionAfterMount === 'function') ftSessionAfterMount('tm', 'tm-fs');
  if (typeof ftSessionBind === 'function') ftSessionBind(tmOnLocal, () => tmResize());

  // Draw one static frame immediately so the canvas is never blank.
  tmDraw(performance.now());
}

function tmRestart() {
  tmBgmStop();                     // music from the old round dies now
  TMG.deathT = 0;
  tmReset();
  TMG.state = 'ready';
  tmOverlay('ready');
  tmSfx('ui');
}

/* ---- Fullscreen (shared FTFS convention) ---------------------------------- */
function tmToggleFullscreen() {
  if (typeof ftToggleFullscreen === 'function') {
    ftToggleFullscreen('tm', 'tm-fs', null);
  }
}

/* ---- Teardown -------------------------------------------------------------- */
function tmTeardown() {
  TMG.epoch++;                     // invalidates the running loop
  TMG._loopEpoch = -1;             // no session: any late frame self-kills
  if (TMG.raf) { cancelAnimationFrame(TMG.raf); TMG.raf = 0; }
  for (const off of TMG.cleanup) { try { off(); } catch { /* gone */ } }
  TMG.cleanup = [];
  TMG.state = 'ready';
  TMG.particles = []; TMG.floats = [];
  TMG.ctx = null; TMG.canvas = null;
  TMG.w = 0; TMG.h = 0;
  TMG._auSync = null;
  TMG._setClose = null;
  TM_MOTES.length = 0;             // background particle pool
  TMG.deathT = 0;
  tmAudioDispose();                // music timer + context closed ONCE
  ftExitFullscreenQuiet();
  document.body.classList.remove('tm-fs');
  ftSessionRestore();               // restore dashboard chrome
}

/* Expose for the console shell (router + tests). Mirrors chess-game.js. */
window.renderTmView = renderTmView;
window.tmTeardown = tmTeardown;
window.TMG = TMG;
window.TMA = TMA;
window.TM_LEVELS = TM_LEVELS;
