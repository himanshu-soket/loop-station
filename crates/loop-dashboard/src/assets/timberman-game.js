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
.tm-wide{max-width:900px}
.tm-hud{display:flex;gap:10px;flex-wrap:wrap;align-items:center;padding:0 22px}
.tm-stat{display:inline-flex;align-items:center;gap:8px;border:1px solid var(--line2);border-radius:10px;
  padding:7px 14px;background:rgba(0,0,0,.32);font:600 12px Sora;color:#aab6c8}
.tm-stat .k{text-transform:uppercase;letter-spacing:.12em;font-size:10px;color:#7f8ca1}
.tm-stat .v{font:800 17px "JetBrains Mono",monospace;color:#eaf2ff;min-width:2ch;text-align:right}
.tm-stat.time .v{color:var(--acc)}
.tm-stat.best .v{color:#ffd166}
#tmTimeBar{position:relative;height:10px;margin:0 22px;border-radius:99px;background:rgba(0,0,0,.45);
  border:1px solid var(--line2);overflow:hidden}
#tmTimeFill{position:absolute;inset:0;width:100%;border-radius:99px;
  background:linear-gradient(90deg,#5ef2c0,#3ecf9a);transition:width .12s linear}
#tmTimeFill.warn{background:linear-gradient(90deg,#ff8fa3,#ff4d6d)}
#tmTimeFill.crit{background:linear-gradient(90deg,#ff4d6d,#c9184a);animation:tmpulse .5s ease-in-out infinite alternate}
@keyframes tmpulse{to{opacity:.55}}
.tm-stage{position:relative;margin:8px 22px;border:1px solid var(--line2);border-radius:14px;overflow:hidden;
  background:linear-gradient(180deg,#0b1322 0%,#0e1a2e 60%,#12233b 100%);user-select:none;-webkit-user-select:none;touch-action:manipulation}
#tmCanvas{display:block;width:100%;height:auto}
.tm-overlay{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;
  background:radial-gradient(ellipse at center,rgba(6,10,18,.62) 0%,rgba(6,10,18,.88) 100%);
  backdrop-filter:blur(3px);z-index:5;text-align:center;padding:18px}
.tm-overlay.hidden{display:none}
.tm-ov-inner{max-width:520px;width:100%}
.tm-ov-title{font:800 clamp(26px,4vw,40px) Sora;letter-spacing:-.02em;color:#f2f7ff;margin-bottom:8px;
  text-shadow:0 0 30px rgba(94,242,192,.25)}
.tm-ov-sub{font:500 14px Sora;color:#9fb0c7;line-height:1.65;margin-bottom:18px}
.tm-ov-sub b{color:#e5edf9}
.tm-ov-keys{display:inline-flex;gap:8px;margin:0 0 16px;flex-wrap:wrap;justify-content:center}
.tm-key{border:1px solid var(--line2);border-bottom-width:3px;border-radius:8px;padding:6px 12px;
  font:700 12px "JetBrains Mono",monospace;color:#cdd9ea;background:rgba(0,0,0,.35)}
.tm-ov-stats{display:flex;gap:14px;justify-content:center;margin-bottom:16px;flex-wrap:wrap}
.tm-ov-stat{border:1px solid var(--line2);border-radius:12px;padding:10px 18px;background:rgba(0,0,0,.4)}
.tm-ov-stat .k{display:block;font:700 10px Sora;letter-spacing:.14em;text-transform:uppercase;color:#7f8ca1;margin-bottom:4px}
.tm-ov-stat .v{font:800 22px "JetBrains Mono",monospace;color:#fff}
.tm-ov-stat.hl .v{color:var(--acc)}
.tm-ov-btn{appearance:none;border:0;border-radius:12px;padding:13px 34px;cursor:pointer;
  font:800 15px Sora;letter-spacing:.02em;color:#06251b;background:linear-gradient(135deg,#5ef2c0,#37c493);
  box-shadow:0 8px 26px rgba(94,242,192,.28);transition:transform .12s,box-shadow .12s}
.tm-ov-btn:hover{transform:translateY(-2px);box-shadow:0 12px 32px rgba(94,242,192,.38)}
.tm-ov-btn:active{transform:translateY(0)}
.tm-ov-best{margin-top:14px;font:700 12px Sora;color:#ffd166;letter-spacing:.06em}
.tm-float{position:absolute;font:800 15px "JetBrains Mono",monospace;color:var(--acc);pointer-events:none;
  text-shadow:0 2px 8px rgba(0,0,0,.6);animation:tmfloat .8s ease-out forwards;z-index:4}
@keyframes tmfloat{to{transform:translateY(-46px);opacity:0}}
.tm-shake{animation:tmshake .3s}
@keyframes tmshake{0%,100%{transform:translateX(0)}25%{transform:translateX(-7px)}75%{transform:translateX(7px)}}
.tm-controls{display:flex;gap:10px;flex-wrap:wrap;padding:0 22px}
/* ---- audio settings popover (same surface language as .sn-audio-pop) -- */
#tmPanel{position:relative}
.tm-audio-pop{position:absolute;top:64px;right:18px;z-index:6;width:280px;
  padding:12px 14px;display:flex;flex-direction:column;gap:9px;
  background:rgba(10,16,28,.96);border:1px solid var(--line2);border-radius:12px;
  box-shadow:0 12px 34px rgba(0,0,0,.45);backdrop-filter:blur(4px)}
.tm-audio-pop.hidden{display:none}
.tm-au-row{display:flex;align-items:center;gap:10px}
.tm-au-head{justify-content:space-between;margin-bottom:2px}
.tm-au-title{font:700 11px Sora;letter-spacing:.14em;color:#aab6c8}
.tm-au-x{border:none;background:transparent;color:#7f8ca1;cursor:pointer;
  font-size:13px;line-height:1;padding:4px 6px;border-radius:6px}
.tm-au-x:hover{color:var(--ink);background:rgba(255,255,255,.06)}
.tm-au-l{flex:0 0 74px;font:600 11.5px Sora;color:#9fb0c7}
.tm-au-slider{flex:1;min-width:0;accent-color:var(--acc);height:18px}
.tm-au-val{flex:0 0 34px;text-align:right;font:700 11px "JetBrains Mono",monospace;color:#e5edf9}
.tm-au-toggles{gap:8px;margin-top:2px}
.tm-au-toggle{flex:1;display:inline-flex;align-items:center;justify-content:center;gap:7px;
  border:1px solid var(--line2);border-radius:9px;padding:7px 10px;background:transparent;
  color:#9fb0c7;font:700 11.5px Sora;cursor:pointer;
  transition:color .12s,border-color .12s,background .12s}
.tm-au-toggle:hover{color:var(--ink);border-color:var(--acc)}
.tm-au-dot{width:8px;height:8px;border-radius:50%;background:#4a5568;transition:background .12s}
.tm-au-toggle[aria-pressed="true"]{color:var(--acc);
  background:rgba(94,242,192,.09);border-color:rgba(94,242,192,.45)}
.tm-au-toggle[aria-pressed="true"] .tm-au-dot{background:var(--acc)}
@media (max-width:520px){.tm-audio-pop{top:auto;bottom:14px;right:12px;left:12px;width:auto}}
/* ---- level chip + progress -------------------------------------------- */
.tm-stat.lvl .v{color:#8ecdf5}
#tmLvlRow{display:flex;align-items:center;gap:10px;margin:6px 22px 0;
  font:700 10px Sora;letter-spacing:.12em;color:#7f8ca1;text-transform:uppercase}
#tmLvlName{color:#8ecdf5;flex:0 0 auto}
#tmLvlBar{flex:1;height:5px;border-radius:99px;background:rgba(0,0,0,.45);
  border:1px solid var(--line2);overflow:hidden;position:relative}
#tmLvlFill{position:absolute;inset:0;width:0%;border-radius:99px;
  background:linear-gradient(90deg,#6ea8ff,#8ecdf5);transition:width .25s ease}
#tmLvlNext{flex:0 0 auto;font:700 10px "JetBrains Mono",monospace;letter-spacing:0;color:#7f8ca1}
/* ---- low-time urgency -------------------------------------------------- */
.tm-stat.time.low .v{color:#ff8fa3;animation:tmblink .5s ease-in-out infinite alternate}
@keyframes tmblink{to{opacity:.45}}
/* ---- level-up toast ---------------------------------------------------- */
.tm-toast{position:absolute;left:50%;top:18%;transform:translateX(-50%);z-index:4;
  pointer-events:none;text-align:center;animation:tmtoast 1.4s ease-out forwards}
.tm-toast .t1{font:800 clamp(20px,3vw,30px) Sora;letter-spacing:.04em;color:#8ecdf5;
  text-shadow:0 0 24px rgba(110,168,255,.55)}
.tm-toast .t2{font:700 11px Sora;letter-spacing:.22em;color:#9fb0c7;text-transform:uppercase;margin-top:2px}
@keyframes tmtoast{0%{opacity:0;transform:translateX(-50%) translateY(14px) scale(.92)}
  12%{opacity:1;transform:translateX(-50%) translateY(0) scale(1)}
  78%{opacity:1}100%{opacity:0;transform:translateX(-50%) translateY(-22px) scale(1.03)}}
.tm-help{display:flex;gap:16px;flex-wrap:wrap;padding:2px 24px 6px;font:500 11.5px Sora;color:#7f8ca1}
.tm-touch{display:none;grid-template-columns:1fr 1fr;gap:12px;padding:0 22px 20px}
.tm-pad{appearance:none;border:1px solid var(--line2);border-radius:14px;background:rgba(0,0,0,.4);
  color:#dbe4f0;font:800 20px Sora;padding:22px 0;cursor:pointer;touch-action:none}
.tm-pad:active{background:rgba(94,242,192,.14);border-color:var(--acc)}
@media (pointer:coarse){.tm-touch{display:grid}.tm-help{display:none}}
/* Fullscreen / immersive layout: the stage owns the viewport. */
body.tm-fs #tmPanel,body.ft-playing.ft-immersive #tmPanel{
  max-width:none;width:100vw;height:100vh;margin:0;border-radius:0;border:none;overflow:hidden;
  padding:clamp(10px,2vh,22px) clamp(12px,3vw,36px);box-sizing:border-box;justify-content:flex-start;
  background:var(--bg,#0a0d13)}
body.tm-fs #tmPanel .ft-panel-h,body.ft-playing.ft-immersive #tmPanel .ft-panel-h{display:none}
body.tm-fs #tmStageWrap,body.ft-playing.ft-immersive #tmStageWrap{flex:1;display:flex;flex-direction:column;min-height:0}
body.tm-fs #tmCanvas,body.ft-playing.ft-immersive #tmCanvas{flex:1;width:100%;height:100%;object-fit:contain}
body.tm-fs .tm-hud,body.ft-playing.ft-immersive .tm-hud{padding:0}
body.tm-fs #tmTimeBar,body.ft-playing.ft-immersive #tmTimeBar{margin:0 2px}
body.tm-fs .tm-controls,body.ft-playing.ft-immersive .tm-controls{padding:0 2px}
body.tm-fs .tm-touch,body.ft-playing.ft-immersive .tm-touch{padding:0 2px 10px}
/* Inline (non-FS) play: centred panel within the main column. */
body.ft-playing #tmPanel{margin:0 auto;width:100%;max-width:min(900px,100%)}
/* Fun Time selector card art (namespaced tm-card-*). */
.ft-card-art.tm{
  background:
    radial-gradient(circle at 30% 20%,rgba(94,242,192,.12),transparent 55%),
    linear-gradient(180deg,rgba(166,124,82,.10),rgba(7,9,13,.6));
}
.tm-card-scene{position:relative;width:min(250px,80%);height:clamp(96px,15vw,132px);display:block;overflow:hidden}
.tm-card-trunk{
  position:absolute;left:50%;bottom:0;transform:translateX(-50%);
  width:34px;height:88%;
  background:linear-gradient(90deg,#7a5230 0%,#a06b3c 45%,#6b4526 100%);
  border-radius:6px 6px 0 0;
}
.tm-card-branch{
  position:absolute;width:58px;height:12px;border-radius:8px;background:#8a5c33;
}
.tm-card-branch.l{left:calc(50% - 62px);top:34%;transform:rotate(-8deg)}
.tm-card-branch.r{left:calc(50% + 4px);top:56%;transform:rotate(8deg)}
.tm-card-man{
  position:absolute;right:16%;bottom:6px;width:30px;height:34px;
  filter:drop-shadow(0 0 8px rgba(94,242,192,.35));
}
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
  trunkCount: 7,            // visible trunk height
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
   tmSoundButton paints the master button from settings; tmAudioSyncUi
   additionally refreshes the popover controls via the view's bound
   sync closure (TMG._auSync). Both are safe pre-mount (no DOM → no-op). */
function tmSoundButton() {
  const btn = document.getElementById('tmSoundBtn');
  if (!btn) return;
  const anyOn = TMA.settings.mu || TMA.settings.sx;
  btn.setAttribute('aria-pressed', String(anyOn));
  btn.classList.toggle('on', anyOn);
  const lbl = document.getElementById('tmSoundLbl');
  if (lbl) lbl.textContent = anyOn ? 'sound on' : 'muted';
}
function tmAudioSyncUi() {
  tmSoundButton();
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
  TMG.side = side;
  TMG.chopAnim = performance.now();
  TMG.lastChopAt = TMG.chopAnim;

  // Branch collision: the segment AT the lumberjack's head level (index 1
  // — index 0 is the stump below) must not hold a branch on his side.
  const headSeg = TMG.trunk[1];
  if (headSeg && headSeg.branch === side) {
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
  tmSpawnChips(side);
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
  const pb = tm$('tmPauseBtn');
  if (pb) pb.textContent = TMG.state === 'paused' ? 'Resume' : 'Pause';
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
  if (!kind) { ov.classList.add('hidden'); return; }
  ov.classList.remove('hidden');
  const title = tm$('tmOvTitle'), sub = tm$('tmOvSub'),
        stats = tm$('tmOvStats'), btn = tm$('tmOvBtn'),
        sc = tm$('tmRsScore'), best = tm$('tmOvBest');
  if (kind === 'ready') {
    title.textContent = 'Timber Man Ultra';
    sub.innerHTML = 'Chop the tree. Dodge the branches.<br><b>← / A</b> chop left · <b>→ / D</b> chop right · <b>P</b> pause<br>The 60-second clock starts on your first chop.';
    stats.classList.add('hidden');
    btn.textContent = 'Start Chopping';
    btn.dataset.kind = 'start';
    if (best) best.textContent = TMG.best > 0 ? `★ best ${TMG.best}` : '';
  } else if (kind === 'paused') {
    title.textContent = 'Paused';
    sub.innerHTML = 'Take a breath.';
    stats.classList.add('hidden');
    btn.textContent = 'Resume';
    btn.dataset.kind = 'resume';
  } else if (kind === 'over') {
    title.textContent = newBest ? 'New Best!' : 'Timber Down!';
    sub.innerHTML = reason === 'time'
      ? 'The clock ran out.'
      : 'A branch got you.';

    stats.classList.remove('hidden');
    if (sc) sc.textContent = String(TMG.score);
    btn.textContent = 'Chop Again';
    btn.dataset.kind = 'again';
    if (best) best.textContent = newBest
      ? `★ new best ${TMG.best}!`
      : (TMG.best > 0 ? `★ best ${TMG.best}` : '');
  }
}

function tmHud() {
  const s = tm$('tmScore'); if (s) s.textContent = String(TMG.score);
  const b = tm$('tmBestV'); if (b) b.textContent = String(TMG.best);
  const t = tm$('tmTime'); if (t) t.textContent = tmTimeText();
  const fill = tm$('tmTimeFill');
  if (fill) {
    const pct = Math.max(0, Math.min(100, TMG.timeMs / TMG.startTimeMs * 100));
    fill.style.width = pct + '%';
    fill.classList.toggle('warn', pct <= 40 && pct > 18);
    fill.classList.toggle('crit', pct <= 18);
  }
  // Low-time urgency on the time stat itself.
  const ts = tm$('tmTimeStat');
  if (ts) ts.classList.toggle('low', TMG.timerStarted && TMG.state === 'playing' && TMG.timeMs <= 10000);
  // Level chip + progress toward the next level.
  const lv = tm$('tmLvlV');
  if (lv) lv.textContent = String(TMG.level + 1);
  const ln = tm$('tmLvlName');
  if (ln) ln.textContent = tmLevelCfg().name;
  const lf = tm$('tmLvlFill');
  if (lf) {
    const cur = TM_LEVELS[TMG.level];
    const nxt = TM_LEVELS[TMG.level + 1];
    let pct = 100;
    if (nxt) pct = Math.max(0, Math.min(100,
      (TMG.score - cur.at) / (nxt.at - cur.at) * 100));
    lf.style.width = pct + '%';
  }
  const nx = tm$('tmLvlNext');
  if (nx) {
    const nxt = TM_LEVELS[TMG.level + 1];
    nx.textContent = nxt ? `${nxt.at - TMG.score} to ${nxt.name}` : 'MAX';
  }
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
function tmSpawnChips(side) {
  if (TMG.reducedMotion) return;
  const cx = TMG.w / 2, cy = TMG.h * 0.52;
  const dir = side === 'left' ? 1 : -1;
  for (let i = 0; i < 7; i++) {
    TMG.particles.push({
      x: cx + dir * (TMG.w * 0.13),
      y: cy + (Math.random() - 0.5) * TMG.h * 0.08,
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
  const rect = c.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) { tmDraw(performance.now()); return; }
  TMG.dpr = Math.min(window.devicePixelRatio || 1, 2);
  TMG.w = Math.max(200, rect.width);
  TMG.h = Math.max(200, rect.height);
  c.width = Math.round(TMG.w * TMG.dpr);
  c.height = Math.round(TMG.h * TMG.dpr);
  TMG.ctx.setTransform(TMG.dpr, 0, 0, TMG.dpr, 0, 0);
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
  const W = TMG.w, H = TMG.h;

  ctx.clearRect(0, 0, W, H);

  // Camera shake: strong on game over, subtle micro-pulse on every chop.
  ctx.save();
  let shakeK = 0;
  if (TMG.shake > 0) shakeK = Math.max(shakeK, TMG.shake / 340);
  if (TMG.chopShake > 0) shakeK = Math.max(shakeK, (TMG.chopShake / 120) * 0.28);
  if (shakeK > 0) {
    ctx.translate((Math.random() - 0.5) * 10 * shakeK,
                  (Math.random() - 0.5) * 10 * shakeK);
  }

  // --- sky + parallax hills ---
  const sky = ctx.createLinearGradient(0, 0, 0, H);
  sky.addColorStop(0, '#0b1322');
  sky.addColorStop(1, '#12233b');
  ctx.fillStyle = sky;
  ctx.fillRect(-20, -20, W + 40, H + 40);

  tmDrawHills(ctx, W, H);
  tmDrawGround(ctx, W, H);

  // --- tree ---
  const segH = tmSegH(H);
  const trunkW = Math.max(46, Math.min(86, W * 0.11));
  const cx = W / 2;
  const baseY = H * 0.86;
  tmDrawTree(ctx, cx, baseY, trunkW, segH, now);

  // --- lumberjack ---
  tmDrawMan(ctx, cx, baseY, trunkW, segH, now);

  // --- particles ---
  for (let i = TMG.particles.length - 1; i >= 0; i--) {
    const p = TMG.particles[i];
    p.x += p.vx; p.y += p.vy; p.vy += 0.18; p.rot += p.vr; p.life -= 0.025;
    if (p.life <= 0) { TMG.particles.splice(i, 1); continue; }
    ctx.save();
    ctx.globalAlpha = Math.max(0, p.life);
    ctx.translate(p.x, p.y);
    ctx.rotate(p.rot);
    ctx.fillStyle = '#c99a5b';
    ctx.fillRect(-p.size / 2, -p.size / 4, p.size, p.size / 2);
    ctx.restore();
  }

  ctx.restore();

  // Red hit-flash (drawn unshaken, over everything).
  if (TMG.flashRed > 0) {
    ctx.fillStyle = `rgba(255,77,109,${(TMG.flashRed / 320) * 0.35})`;
    ctx.fillRect(0, 0, W, H);
  }
}

function tmSegH(H) { return Math.max(44, Math.min(84, H * 0.105)); }

function tmDrawHills(ctx, W, H) {
  ctx.fillStyle = 'rgba(20,40,66,.85)';
  ctx.beginPath();
  ctx.moveTo(0, H * 0.8);
  ctx.quadraticCurveTo(W * 0.2, H * 0.62, W * 0.45, H * 0.78);
  ctx.quadraticCurveTo(W * 0.7, H * 0.9, W, H * 0.68);
  ctx.lineTo(W, H); ctx.lineTo(0, H); ctx.closePath(); ctx.fill();
  ctx.fillStyle = 'rgba(16,32,54,.9)';
  ctx.beginPath();
  ctx.moveTo(0, H * 0.88);
  ctx.quadraticCurveTo(W * 0.3, H * 0.74, W * 0.6, H * 0.86);
  ctx.quadraticCurveTo(W * 0.85, H * 0.94, W, H * 0.82);
  ctx.lineTo(W, H); ctx.lineTo(0, H); ctx.closePath(); ctx.fill();
}

function tmDrawGround(ctx, W, H) {
  ctx.fillStyle = '#16321f';
  ctx.fillRect(-20, H * 0.86, W + 40, H * 0.14 + 20);
  ctx.fillStyle = '#1d4229';
  ctx.fillRect(-20, H * 0.86, W + 40, 6);
}

function tmDrawTree(ctx, cx, baseY, trunkW, segH, now) {
  // Trunk slide-down: after a chop the whole stack eases from one segment
  // higher to its resting position (slideT 0 → 1). Reduced-motion skips it.
  const slide = (TMG.reducedMotion || TMG.slideT >= 1) ? 1 : TMG.slideT;
  const ease = 1 - Math.pow(1 - slide, 3);            // ease-out cubic
  const offY = (1 - ease) * segH;                     // px still to travel

  // Segments: index 0 sits on the ground; the lumberjack stands beside it.
  for (let i = 0; i < TMG.trunk.length; i++) {
    const seg = TMG.trunk[i];
    const y = baseY - i * segH + offY;
    // Trunk slice with rounded bark texture.
    const g = ctx.createLinearGradient(cx - trunkW / 2, 0, cx + trunkW / 2, 0);
    g.addColorStop(0, '#7a5230');
    g.addColorStop(0.45, '#a06b3c');
    g.addColorStop(1, '#6b4526');
    ctx.fillStyle = g;
    tmRoundRect(ctx, cx - trunkW / 2, y - segH, trunkW, segH - 2, 6);
    ctx.fill();
    // Bark line.
    ctx.strokeStyle = 'rgba(0,0,0,.22)';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(cx - trunkW * 0.18, y - segH + 4);
    ctx.lineTo(cx - trunkW * 0.18, y - 6);
    ctx.stroke();

    // Branch on this segment.
    if (seg.branch) {
      const dir = seg.branch === 'left' ? -1 : 1;
      const by = y - segH * 0.72;
      const len = trunkW * 1.55;
      ctx.save();
      ctx.translate(cx + dir * trunkW / 2, by);
      // Branch arm.
      const bg = ctx.createLinearGradient(0, 0, dir * len, 0);
      bg.addColorStop(0, '#8a5c33');
      bg.addColorStop(1, '#5f3d1f');
      ctx.fillStyle = bg;
      tmRoundRect(ctx, dir < 0 ? -len : 0, -7, len, 14, 7);
      ctx.fill();
      // Leaves cluster.
      ctx.fillStyle = '#2e7d4f';
      ctx.beginPath();
      ctx.ellipse(dir * len * 0.92, -4, 20, 13, dir * 0.3, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = '#3b9e63';
      ctx.beginPath();
      ctx.ellipse(dir * len * 0.8, -12, 14, 9, dir * 0.25, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    }
  }
  // Canopy at the very top.
  const topY = baseY - TMG.trunk.length * segH + offY;
  ctx.fillStyle = '#256b43';
  ctx.beginPath();
  ctx.ellipse(cx, topY + 6, trunkW * 1.5, trunkW * 0.95, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#2f8a56';
  ctx.beginPath();
  ctx.ellipse(cx - trunkW * 0.5, topY - 4, trunkW * 0.9, trunkW * 0.6, -0.2, 0, Math.PI * 2);
  ctx.fill();
  ctx.beginPath();
  ctx.ellipse(cx + trunkW * 0.55, topY - 2, trunkW * 0.85, trunkW * 0.55, 0.25, 0, Math.PI * 2);
  ctx.fill();
}

function tmDrawMan(ctx, cx, baseY, trunkW, segH, now) {
  const dir = TMG.side === 'left' ? -1 : 1;
  const x = cx + dir * (trunkW / 2 + 26);
  const feetY = baseY + 4;
  const scale = Math.max(0.8, Math.min(1.15, TMG.h / 560));

  // Axe swing: 0..1 progress since the last chop.
  const since = now - TMG.lastChopAt;
  const swing = TMG.state === 'playing' && since < 220
    ? Math.sin((since / 220) * Math.PI) : 0;

  ctx.save();
  ctx.translate(x, feetY);
  ctx.scale(scale * (dir < 0 ? -1 : 1), scale);

  // Legs.
  ctx.strokeStyle = '#2c3a55'; ctx.lineWidth = 7; ctx.lineCap = 'round';
  ctx.beginPath(); ctx.moveTo(0, -26); ctx.lineTo(-7, 0); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(0, -26); ctx.lineTo(8, 0); ctx.stroke();

  // Body (plaid-ish shirt).
  ctx.fillStyle = '#d64545';
  tmRoundRect(ctx, -11, -58, 22, 34, 8); ctx.fill();
  ctx.fillStyle = 'rgba(255,255,255,.18)';
  ctx.fillRect(-11, -50, 22, 3); ctx.fillRect(-11, -41, 22, 3);

  // Head + helmet.
  ctx.fillStyle = '#f2c19a';
  ctx.beginPath(); ctx.arc(0, -68, 10, 0, Math.PI * 2); ctx.fill();
  ctx.fillStyle = '#ffb703';
  ctx.beginPath(); ctx.arc(0, -70, 10.5, Math.PI, 0); ctx.fill();
  ctx.fillRect(-11, -71, 22, 4);

  // Arms + axe (swings toward the trunk).
  const ax = 14 + swing * 10;
  const ay = -52 + swing * 16;
  ctx.strokeStyle = '#f2c19a'; ctx.lineWidth = 6;
  ctx.beginPath(); ctx.moveTo(4, -52); ctx.lineTo(ax, ay); ctx.stroke();
  // Axe handle.
  ctx.strokeStyle = '#8a5c33'; ctx.lineWidth = 5;
  ctx.beginPath(); ctx.moveTo(ax, ay); ctx.lineTo(ax + 16, ay - 14); ctx.stroke();
  // Axe head.
  ctx.fillStyle = '#cfd8e3';
  ctx.save();
  ctx.translate(ax + 16, ay - 14);
  ctx.rotate(0.6 - swing * 0.9);
  tmRoundRect(ctx, -4, -9, 14, 18, 3); ctx.fill();
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
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Timber Man Ultra</div>
    </div>

    <section class="ft-panel ft-game tm-wide" id="tmPanel" aria-label="Timber Man Ultra arcade game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Timber Man Ultra</div>
          <div class="ft-blurb">Chop fast, dodge branches, race the clock. Every chop buys time — but the refund shrinks as you climb.</div>
        </div>
      </header>

      <div class="tm-hud" role="status" aria-live="off">
        <span class="tm-stat"><span class="k">score</span><span class="v" id="tmScore">0</span></span>
        <span class="tm-stat time" id="tmTimeStat"><span class="k">time</span><span class="v" id="tmTime">60.0</span></span>
        <span class="tm-stat lvl"><span class="k">level</span><span class="v" id="tmLvlV">1</span></span>
        <span class="tm-stat best"><span class="k">best</span><span class="v" id="tmBestV">0</span></span>
      </div>

      <div id="tmTimeBar" aria-hidden="true"><span id="tmTimeFill"></span></div>
      <div id="tmLvlRow" aria-hidden="true">
        <span id="tmLvlName">EASY</span>
        <span id="tmLvlBar"><span id="tmLvlFill"></span></span>
        <span id="tmLvlNext"></span>
      </div>

      <div class="tm-stage" id="tmStageWrap">
        <canvas id="tmCanvas" aria-label="Timber Man play field"
                role="img">Your browser does not support the canvas element.</canvas>
        <div class="tm-overlay" id="tmOverlay" role="dialog" aria-modal="false" aria-label="Game status">
          <div class="tm-ov-inner">
            <div class="tm-ov-title" id="tmOvTitle">Timber Man Ultra</div>
            <div class="tm-ov-sub" id="tmOvSub"></div>
            <div class="tm-ov-stats hidden" id="tmOvStats">
              <div class="tm-ov-stat hl"><span class="k">score</span><span class="v" id="tmRsScore">0</span></div>
            </div>
            <button type="button" class="tm-ov-btn" id="tmOvBtn" data-kind="start">Start Chopping</button>
            <div class="tm-ov-best" id="tmOvBest"></div>
          </div>
        </div>
      </div>

      <div class="tm-controls" role="group" aria-label="Game controls">
        <button type="button" class="ft-btn" id="tmPauseBtn">Pause</button>
        <button type="button" class="ft-btn" id="tmRestartBtn">Restart</button>
        <button type="button" class="ft-btn si-sound" id="tmSoundBtn" aria-pressed="true"
                aria-haspopup="dialog" aria-expanded="false">
          <span class="si-snd-ico" aria-hidden="true">♪</span><span id="tmSoundLbl">sound on</span>
        </button>
        <button type="button" class="ft-btn" id="tmFsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn si-exit" id="tmExit">Back to Games</button>
      </div>

      <!-- audio settings popover: music/effects toggles + volume sliders.
           Persists via localStorage ('tm-audio-v1'); outside-click close. -->
      <div class="tm-audio-pop hidden" id="tmAudioPop" role="dialog"
           aria-label="Audio settings" aria-modal="false">
        <div class="tm-au-row tm-au-head">
          <span class="tm-au-title">AUDIO</span>
          <button type="button" class="tm-au-x" id="tmAudioClose"
                  aria-label="Close audio settings">✕</button>
        </div>
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
        <div class="tm-au-row tm-au-toggles">
          <button type="button" class="tm-au-toggle" id="tmAuMusicT"
                  aria-pressed="true"><span class="tm-au-dot" aria-hidden="true"></span>Music</button>
          <button type="button" class="tm-au-toggle" id="tmAuSfxT"
                  aria-pressed="true"><span class="tm-au-dot" aria-hidden="true"></span>Effects</button>
        </div>
      </div>

      <div class="tm-help">
        <span>←/A chop left</span><span>→/D chop right</span>
        <span>P pause</span><span>R restart</span><span>F fullscreen</span>
      </div>

      <!-- Touch controls: two large half-screen pads. -->
      <div class="tm-touch" role="group" aria-label="Touch chop controls">
        <button type="button" class="tm-pad" id="tmPadL" aria-label="Chop left">⬅ chop left</button>
        <button type="button" class="tm-pad" id="tmPadR" aria-label="Chop right">chop right ➡</button>
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
  const okInit = tmInit(document.getElementById('tmCanvas'));
  if (!okInit) {
    // Surface init failure honestly instead of a permanently blank stage.
    document.getElementById('viewRoot').innerHTML =
      `<div class="content"><div class="content-inner">
         <div class="ft-game-top">
           <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
           <div class="ft-game-title">Timber Man Ultra</div>
         </div>
         <section class="ft-panel ft-game tm-wide" id="tmPanel" aria-label="Timber Man Ultra failed to start">
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
  tmSoundButton();
  tmAudioSyncUi();
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

  // ---- keyboard ----
  // Escape is owned by the shared session controller (ftSessionBind):
  // one press leaves fullscreen AND returns to Fun Time.
  const GAME_KEYS = new Set(['ArrowLeft','ArrowRight','a','d','A','D',
    'p','P','r','R','f','F','Escape']);
  tmOnLocal(document, 'keydown', e => {
    if (TMG.epoch !== myEpoch) return;
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (e.key === 'ArrowLeft' || e.key === 'a' || e.key === 'A') {
      if (TMG.state === 'playing') { tmChop('left'); }
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowRight' || e.key === 'd' || e.key === 'D') {
      if (TMG.state === 'playing') { tmChop('right'); }
      e.preventDefault(); return;
    }
    if (e.key === 'p' || e.key === 'P') { tmTogglePause(); e.preventDefault(); return; }
    if (e.key === 'r' || e.key === 'R') { tmRestart(); e.preventDefault(); return; }
    if (e.key === 'f' || e.key === 'F') { tmToggleFullscreen(); e.preventDefault(); return; }
    if (GAME_KEYS.has(e.key)) e.preventDefault();
  });

  // Held keys must not survive a blur or tab switch.
  tmOnLocal(window, 'blur', () => { if (TMG.state === 'playing') tmTogglePause(); });
  tmOnLocal(document, 'visibilitychange', () => {
    if (document.hidden && TMG.state === 'playing') tmTogglePause();
  });

  // ---- persistent controls ----
  tmOnLocal(tm$('tmPauseBtn'), 'click', () => tmTogglePause());
  tmOnLocal(tm$('tmRestartBtn'), 'click', () => tmRestart());

  /* ---- audio settings popover ------------------------------------------
     The sound button is the entry point to the full settings panel.
     State lives in TMA.settings (persisted); the UI is a plain read/write
     of that object, so refresh/restart both restore exactly what was
     saved. Changes apply immediately through click-free gain ramps. */
  const auPop = tm$('tmAudioPop');
  const auSyncUi = () => {
    const a = TMA.settings;
    const anyOn = a.mu || a.sx;
    const b = tm$('tmSoundBtn');
    if (b) {
      b.setAttribute('aria-pressed', String(anyOn));
      b.classList.toggle('on', anyOn);
      const lbl = tm$('tmSoundLbl');
      if (lbl) lbl.textContent = anyOn ? 'sound on' : 'muted';
    }
    const set = (id, v) => { const el = tm$(id); if (el) el.value = String(Math.round(v * 100)); };
    const txt = (id, v) => { const el = tm$(id); if (el) el.textContent = Math.round(v * 100) + '%'; };
    set('tmAuMusicV', a.mv); txt('tmAuMusicVal', a.mv);
    set('tmAuSfxV', a.sv);   txt('tmAuSfxVal', a.sv);
    const tg = (id, on) => { const el = tm$(id); if (el) el.setAttribute('aria-pressed', String(!!on)); };
    tg('tmAuMusicT', a.mu); tg('tmAuSfxT', a.sx);
  };
  TMG._auSync = auSyncUi;
  const auOpen = () => {
    if (!auPop) return;
    auPop.classList.remove('hidden');
    tm$('tmSoundBtn').setAttribute('aria-expanded', 'true');
    tmSfx('ui');
  };
  const auClose = () => {
    if (!auPop) return;
    auPop.classList.add('hidden');
    const b = tm$('tmSoundBtn');
    if (b) b.setAttribute('aria-expanded', 'false');
  };

  tmOnLocal(tm$('tmSoundBtn'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    tmAudioUnlock();
    if (auPop && auPop.classList.contains('hidden')) auOpen();
    else auClose();
  });
  tmOnLocal(tm$('tmAudioClose'), 'click', () => {
    if (TMG.epoch !== myEpoch) return;
    auClose();
  });

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

  // dismiss on outside click
  tmOnLocal(document, 'pointerdown', e => {
    if (TMG.epoch !== myEpoch) return;
    if (!auPop || auPop.classList.contains('hidden')) return;
    if (auPop.contains(e.target) || tm$('tmSoundBtn').contains(e.target)) return;
    auClose();
  }, true);

  tmOnLocal(tm$('tmFsBtn'), 'click', () => tmToggleFullscreen());
  if (typeof ftFsSupported === 'function' && !ftFsSupported()) {
    const b = tm$('tmFsBtn'); if (b) b.style.display = 'none';
  }
  /* Reflect genuine fullscreen state onto the body class + button label
     (same convention as Snakes/Ludo). This is what makes the Fullscreen
     BUTTON path apply the .tm-fs layout; without it the canvas stayed
     letterboxed after entering fullscreen via the button or F key. */
  const tmFsReflect = () => {
    if (typeof ftFsReflect === 'function') ftFsReflect('tm', 'tm-fs', '#tmFsBtn');
    tmResize();
  };
  tmOnLocal(document, 'fullscreenchange', tmFsReflect);
  tmOnLocal(document, 'webkitfullscreenchange', tmFsReflect);
  tmOnLocal(tm$('tmExit'), 'click', () => { location.hash = '#funtime'; });

  // Overlay button: Start / Resume / Chop Again.
  tmOnLocal(tm$('tmOvBtn'), 'click', () => {
    const kind = tm$('tmOvBtn').dataset.kind;
    if (kind === 'again') { tmRestart(); tmStart(); }
    else tmStart();
  });

  // ---- touch/mouse pads ----
  const bindTap = (id, side) => {
    const el = tm$(id);
    if (!el) return;
    const press = e => {
      e.preventDefault();
      if (TMG.state === 'ready' || TMG.state === 'over') { tmStart(); return; }
      if (TMG.state === 'paused') { tmTogglePause(); return; }
      tmChop(side);
    };
    tmOnLocal(el, 'pointerdown', press);
    tmOnLocal(el, 'contextmenu', e => e.preventDefault());
  };
  bindTap('tmPadL', 'left');
  bindTap('tmPadR', 'right');

  // Desktop mouse: click the left/right half of the stage to chop.
  tmOnLocal(tm$('tmStageWrap'), 'pointerdown', e => {
    if (TMG.epoch !== myEpoch) return;
    if (e.pointerType === 'touch') return;          // touch uses the pads
    const pad = e.target.closest('.tm-pad, .tm-overlay, button');
    if (pad) return;                                 // UI elements own their clicks
    if (TMG.state === 'ready' || TMG.state === 'over') { tmStart(); return; }
    if (TMG.state === 'paused') { tmTogglePause(); return; }
    const r = tm$('tmStageWrap').getBoundingClientRect();
    tmChop(e.clientX - r.left < r.width / 2 ? 'left' : 'right');
  });

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
  tmReset();
  TMG.state = 'ready';
  const pb = tm$('tmPauseBtn');
  if (pb) pb.textContent = 'Pause';
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
