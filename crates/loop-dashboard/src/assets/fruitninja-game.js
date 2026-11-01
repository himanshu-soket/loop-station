/* ============================================================
   FUN TIME · GAME 8 — FRUIT NINJA
   Route: #funtime/fn · Namespace: FN / fn* functions / .fn- CSS.

   A complete arcade fruit-slicing game on HTML5 canvas:
     • procedural fruit artwork (10 fruits + golden bonus, gradients +
       paths, no emoji, no external assets) with real cut faces,
     • parabolic physics with per-fruit velocity / gravity / spin,
     • real blade mechanics: full pointer-path tracking, segment↔circle
       intersection tests, swipe-speed gating, per-swipe combo bonuses,
     • two modes — Classic (3 lives; misses & bombs cost a life) and
       Arcade (60s countdown, bonus fruit, faster ramp),
     • time-based difficulty (PHASE 1): one elapsed-play clock that
       freezes on pause drives wave rate, wave size, bomb odds and
       launch speed through a smoothstep curve — Classic ramps over
       ~80 s, Arcade across its whole 60 s countdown. The previous
       Arcade gap curve ran BACKWARDS (fastest at second zero), which
       this replaces; scoring, lives, collisions and game-over logic
       are untouched,
     • bombs with lit fuses, explosions, screen shake, penalties,
     • juice droplets, splash rings, floating scores, glowing tapered
       blade trail, critical-slice bonuses,
     • Web Audio API synthesis for every effect (no audio files),
     • PHASE 2B procedural background music on the same context: an
       Am–F–C–G arcade groove at 120 BPM (sub bass, chord pads, kick/
       snare/hats, pentatonic lead in bars 5–8), look-ahead scheduled
       for seamless looping; fades on pause, tears down on game over,
     • PHASE 2C polished slice & combo voices on the SFX bus: every cut
       gets a varied juicy swish+splat+thock (pitch never repeats), and
       combo milestones 3×/4×/5+ ring distinct rising phrases; a voice
       budget + bus compressor keep rapid slicing clean, never loud,
     • best scores persisted per mode in localStorage.

   Lifecycle mirrors the other Fun Time games exactly:
     • one state object (FNG) with an epoch guard that invalidates
       stale async continuations,
     • every listener registered through fnOn() so fnTeardown()
       removes them all on any route change,
     • automatic fullscreen from the card click via the shared FTSESS
       controller (ftSessionLaunch → ftSessionAfterMount), with the
       honest CSS immersive fallback when native is denied,
     • PHASE 3 responsive canvas: fnResize() measures the stage (never
       the canvas itself), letterboxes the field to ≤16:10 inside it at
       device resolution, and a ResizeObserver + fullscreenchange refit
       on every layout change — inline or fullscreen, nothing clips,
     • PHASE 3B responsive blade input: cached canvas rect (never a
       per-move getBoundingClientRect), full-rate pointermove with
       coalesced sub-frame samples, single-pointer ownership, passive
       listeners, zero throttle/debounce — identical slicing semantics,
     • Esc leaves the game and returns to #funtime in ONE press,
     • dashboard chrome hidden while playing (body.ft-playing),
     • zero DOM/CSS leakage between mounts (namespaced + torn down).

   States: 'menu' → 'playing' (opens with a GET READY grace beat)
           ⇄ 'paused' → 'over' → 'menu' | 'playing'.
   ============================================================ */

/* ---- Injected styles (namespaced; no global selectors touched) ---- */
(function fnInjectCss() {
  if (document.getElementById('fn-style')) return;
  const st = document.createElement('style');
  st.id = 'fn-style';
  st.textContent = `
.fn-wide{max-width:980px}
.fn-hud{display:flex;gap:8px;flex-wrap:wrap;align-items:center;padding:0 18px}
.fn-stat{display:inline-flex;align-items:center;gap:6px;border:1px solid var(--line2);border-radius:8px;
  padding:4px 10px;background:rgba(0,0,0,.32);font:600 11px Sora;color:#aab6c8}
.fn-stat .k{text-transform:uppercase;letter-spacing:.08em;font-size:9.5px;color:#7f8ca1}
.fn-stat .v{font:700 13.5px "JetBrains Mono",monospace;color:#eaf2ff;min-width:2ch;text-align:right}
.fn-stat.score .v{color:var(--acc)}
.fn-stat.best .v{color:#ffd166}
.fn-stat.time .v{color:#ffd166}
.fn-stat.time.warn .v{color:#ff6b81}
.fn-stat .hearts{display:inline-flex;gap:3px;min-width:0}
.fn-stat .hearts i{font-style:normal;font-size:13px;line-height:1;color:#ff5d73;
  text-shadow:0 0 8px rgba(255,93,115,.45);transition:opacity .2s,color .2s}
.fn-stat .hearts i.off{opacity:.18;color:#8794a8;text-shadow:none}
.fn-mode-arcade .fn-stat.lives{display:none}
.fn-mode-classic .fn-stat.time{display:none}
.fn-controls{display:flex;gap:8px;flex-wrap:wrap;align-items:center;padding:0 18px}
.fn-controls .ft-btn{padding:6px 12px;font-size:11px;border-radius:8px}
.fn-controls .fn-vol{display:inline-flex;align-items:center;gap:7px;padding:6px 10px;font-size:11px;border-radius:8px}
.fn-controls .fn-vol span{opacity:.7;white-space:nowrap}
.fn-controls .fn-vol input[type=range]{width:62px;height:14px;margin:0;padding:0;border:0;background:transparent;accent-color:#7fe8b8;cursor:pointer}
.fn-controls .fn-vol.muted{opacity:.45}
@media (max-width:720px){.fn-controls .fn-vol input[type=range]{width:46px}}
.fn-help{display:flex;gap:14px;flex-wrap:wrap;padding:2px 18px 4px;font:500 11px Sora;color:#7f8ca1}
.fn-stage{position:relative;margin:6px 18px 12px;border:1px solid var(--line2);border-radius:14px;
  overflow:hidden;user-select:none;-webkit-user-select:none;touch-action:none;cursor:crosshair;
  background:#0b1322}
#fnCanvas{display:block;width:100%;height:100%}
/* Inline play: fixed 16/10 stage. Fullscreen / immersive: the stage takes
   all remaining vertical space (HUD above, controls below) and fnResize()
   letterboxes the canvas inside it at ≤16:10, centred, so nothing ever
   clips at any viewport. The canvas's exact box is set inline by
   fnResize() — CSS never guesses the world height. (The previous rule
   used aspect-ratio:1000/var(--fn-wh,625), but no JS ever wrote that
   variable, so fullscreen silently fell back to a hardcoded ratio and
   could clip; it is removed as superseded.) */
.fn-stage-inner{position:relative;width:100%;aspect-ratio:16/10}
/* FULLSCREEN FIX: the stage-inner drops its inline 16:10 ratio and takes
   ALL remaining vertical space; fnResize() writes the exact canvas box
   inline (filling the whole area in fullscreen), so no black margins. */
body.fn-fs .fn-stage-inner,body.ft-playing.ft-immersive .fn-stage-inner{
  aspect-ratio:auto;flex:1;min-height:0;display:flex;align-items:center;justify-content:center;
  overflow:hidden}
body.fn-fs #fnCanvas,body.ft-playing.ft-immersive #fnCanvas{margin:auto}
.fn-overlay{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;
  background:radial-gradient(ellipse at center,rgba(6,10,18,.66) 0%,rgba(6,10,18,.9) 100%);
  backdrop-filter:blur(3px);z-index:5;text-align:center;padding:18px}
.fn-overlay.hidden{display:none}
.fn-ov-inner{max-width:520px;width:100%}
.fn-ov-title{font:800 clamp(26px,4vw,40px) Sora;letter-spacing:-.02em;color:#f2f7ff;margin-bottom:8px;
  text-shadow:0 0 30px rgba(94,242,192,.25)}
.fn-ov-sub{font:500 13.5px Sora;color:#9fb0c7;line-height:1.65;margin-bottom:18px}
.fn-ov-sub b{color:#e5edf9}
.fn-modes{display:flex;gap:12px;justify-content:center;flex-wrap:wrap;margin-bottom:16px}
.fn-mode{appearance:none;border:1px solid var(--line2);border-radius:14px;cursor:pointer;
  background:rgba(0,0,0,.42);padding:16px 22px;min-width:172px;text-align:left;
  transition:border-color .15s,transform .15s,box-shadow .15s}
.fn-mode:hover{border-color:rgba(94,242,192,.5);transform:translateY(-2px)}
.fn-mode:focus-visible{outline:2px solid var(--acc);outline-offset:2px}
.fn-mode .t{display:block;font:800 16px Sora;color:#f2f7ff;margin-bottom:5px}
.fn-mode .d{display:block;font:500 11.5px Sora;color:#93a3b8;line-height:1.5}
.fn-mode .ic{display:block;font-size:22px;line-height:1;margin-bottom:8px}
.fn-ov-stats{display:flex;gap:12px;justify-content:center;margin-bottom:16px;flex-wrap:wrap}
.fn-ov-stat{border:1px solid var(--line2);border-radius:12px;padding:9px 16px;background:rgba(0,0,0,.4)}
.fn-ov-stat .k{display:block;font:700 9.5px Sora;letter-spacing:.12em;text-transform:uppercase;color:#7f8ca1;margin-bottom:3px}
.fn-ov-stat .v{font:800 20px "JetBrains Mono",monospace;color:#fff}
.fn-ov-stat.hl .v{color:var(--acc)}
.fn-ov-row{display:flex;gap:10px;justify-content:center;flex-wrap:wrap}
.fn-ov-btn{appearance:none;border:0;border-radius:12px;padding:12px 30px;cursor:pointer;
  font:800 14.5px Sora;letter-spacing:.02em;color:#06251b;background:linear-gradient(135deg,#5ef2c0,#37c493);
  box-shadow:0 8px 26px rgba(94,242,192,.28);transition:transform .12s,box-shadow .12s}
.fn-ov-btn:hover{transform:translateY(-2px);box-shadow:0 12px 32px rgba(94,242,192,.38)}
.fn-ov-btn:active{transform:translateY(0)}
.fn-ov-btn.ghost{background:transparent;color:#cdd9ea;border:1px solid var(--line2);box-shadow:none}
.fn-ov-btn.ghost:hover{color:#fff;border-color:var(--ink-faint)}
.fn-ov-best{margin-top:14px;font:700 12px Sora;color:#ffd166;letter-spacing:.06em}
/* Fullscreen / immersive layout: the stage owns the viewport — the whole
   available screen, not a constrained column. */
body.fn-fs #fnPanel,body.ft-playing.ft-immersive #fnPanel{
  max-width:none;width:100vw;height:100vh;margin:0;border-radius:0;border:none;overflow:hidden;
  padding:clamp(10px,2vh,22px) clamp(12px,3vw,36px);box-sizing:border-box;justify-content:flex-start;
  background:var(--bg,#0a0d13)}
body.fn-fs #fnPanel .ft-panel-h,body.ft-playing.ft-immersive #fnPanel .ft-panel-h{display:none}
body.fn-fs .fn-stage,body.ft-playing.ft-immersive .fn-stage{margin:6px 2px 10px;flex:1;display:flex;flex-direction:column;min-height:0}
body.fn-fs .fn-hud,body.ft-playing.ft-immersive .fn-hud{padding:0}
body.fn-fs .fn-controls,body.ft-playing.ft-immersive .fn-controls{padding:0 2px}
body.fn-fs .fn-help,body.ft-playing.ft-immersive .fn-help{display:none}
/* Inline (non-FS) play: centred panel within the main column. */
body.ft-playing #fnPanel{margin:0 auto;width:100%;max-width:min(980px,100%)}
/* Fun Time selector card art (namespaced fn-card-*). */
.ft-card-art.fn{
  background:
    radial-gradient(circle at 30% 20%,rgba(94,242,192,.12),transparent 55%),
    linear-gradient(180deg,rgba(255,93,115,.10),rgba(7,9,13,.6));
}
.fn-card-scene{position:relative;width:min(240px,74%);height:clamp(104px,15vw,140px);display:block}
.fn-card-melon{position:absolute;left:50%;top:54%;transform:translate(-50%,-50%) rotate(-14deg);
  width:clamp(84px,12vw,112px);aspect-ratio:1;border-radius:50%;
  background:radial-gradient(circle at 35% 30%,#ff6b6b 0%,#e63946 55%,#c1121f 100%);
  border:7px solid #2d6a4f;box-shadow:0 0 0 3px #1b4332,inset 0 -10px 18px rgba(0,0,0,.25)}
.fn-card-melon i{position:absolute;width:5px;height:8px;border-radius:50%;background:#231207;
  transform:rotate(20deg)}
.fn-card-slash{position:absolute;left:8%;top:12%;width:118%;height:5px;border-radius:99px;
  transform:rotate(-32deg);transform-origin:left center;
  background:linear-gradient(90deg,transparent,#fff 30%,#d0fff0 55%,transparent);
  box-shadow:0 0 14px rgba(208,255,240,.8);opacity:.95}
.fn-card-spark{position:absolute;right:4%;top:6%;width:10px;height:10px;border-radius:50%;
  background:#fff;box-shadow:0 0 16px 4px rgba(255,255,255,.85);opacity:.9}
@media (prefers-reduced-motion:reduce){.fn-card-slash{opacity:.6}}
`;
  document.head.appendChild(st);
})();

/* ---- Audio: WebAudio synthesis, no external assets -----------------
   Same envelope pattern as tmBeep(), plus a shared noise buffer for
   swishes/explosions. Initialised lazily on the first user gesture so
   autoplay policy never blocks it; disposed in fnTeardown().

   PHASE 2A: fnAudioInit() is now the single entry point for context
   creation. It reuses an already-running context (never constructs a
   second one), resumes suspended contexts (autoplay unlock), and keeps
   the master gain in sync with FNA.volume × FNA.on. The tone/noise
   synths below are unchanged.                                       */
const FNA = {
  ctx: null, master: null,
  on: true, volume: 0.8,          // on = mute gate; volume = master level 0..1
  noiseBuf: null,
  unlocked: false,                // set once a user gesture has opened the ctx
  sfx: { volume: 1, gain: null, comp: null, live: 0 }, // PHASE 2B bus + 2C budget
  bgm: {                          // PHASE 2B: music bus + scheduler state
    volume: 0.34,                 // independent music level under the master
    started: false, timer: 0,     // one scheduler interval, never two
    step: 0, loop: 0, nextT: 0,   // 16th-note grid position + anchor time
    gain: null, comp: null,       // fade bus → compressor → master
  },
};

/** Master gain actually applied to the graph: 0 when muted. */
function fnAudioMasterGain() {
  return FNA.on ? FNA.volume : 0;
}

/** Apply the current mute/volume state to the live master gain node. */
function fnAudioApplyGain() {
  if (FNA.master && FNA.ctx) {
    try {
      FNA.master.gain.setTargetAtTime(fnAudioMasterGain(), FNA.ctx.currentTime, 0.015);
    } catch { /* ctx mid-close */ }
  }
}

/**
 * Create (once) or reuse the AudioContext, then resume it if the browser
 * suspended it under the autoplay policy. Safe to call from any user
 * gesture and from the sound toggle; duplicate calls are no-ops.
 * @returns {boolean} true when a usable context exists.
 */
function fnAudioInit() {
  // Reuse: one context per view lifecycle, never a second instance.
  if (FNA.ctx) {
    if (FNA.ctx.state === 'suspended') FNA.ctx.resume().catch(() => {});
    fnAudioApplyGain();
    return true;
  }
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return false;
  try {
    FNA.ctx = new AC();
    FNA.master = FNA.ctx.createGain();
    FNA.master.gain.value = fnAudioMasterGain();
    FNA.master.connect(FNA.ctx.destination);
    // PHASE 2B: dedicated SFX bus under the master gain. Every existing
    // effect routes through it, so an independent SFX level (and later a
    // separate toggle) is one setTargetAtTime away — no synth changes.
    FNA.sfx.gain = FNA.ctx.createGain();
    FNA.sfx.gain.gain.value = FNA.sfx.volume;
    // PHASE 2C: SFX compressor — rapid consecutive slices stack, and the
    // bus-level limiter (−14 dB, 8:1, fast release) keeps stacked bursts
    // from ever getting loud or clipping, without touching per-voice levels.
    FNA.sfx.comp = FNA.ctx.createDynamicsCompressor();
    FNA.sfx.comp.threshold.value = -14;
    FNA.sfx.comp.knee.value = 6;
    FNA.sfx.comp.ratio.value = 8;
    FNA.sfx.comp.attack.value = 0.003;
    FNA.sfx.comp.release.value = 0.12;
    FNA.sfx.comp.connect(FNA.master);
    FNA.sfx.gain.connect(FNA.sfx.comp);
    // PHASE 2B: music bus — fade gain → soft compressor (peaks never
    // clip the destination when music + effects stack) → master. The
    // compressor is transparent at normal levels (threshold −18 dB,
    // gentle 3:1, fast release) and only catches stacked transients.
    FNA.bgm.gain = FNA.ctx.createGain();
    FNA.bgm.gain.gain.value = 0;            // silent until faded in
    FNA.bgm.comp = FNA.ctx.createDynamicsCompressor();
    FNA.bgm.comp.threshold.value = -18;
    FNA.bgm.comp.knee.value = 12;
    FNA.bgm.comp.ratio.value = 3;
    FNA.bgm.comp.attack.value = 0.004;
    FNA.bgm.comp.release.value = 0.26;
    FNA.bgm.comp.connect(FNA.master);
    FNA.bgm.gain.connect(FNA.bgm.comp);
    // 1s of white noise, reused by every swish/explosion.
    const len = Math.floor(FNA.ctx.sampleRate);
    FNA.noiseBuf = FNA.ctx.createBuffer(1, len, FNA.ctx.sampleRate);
    const ch = FNA.noiseBuf.getChannelData(0);
    for (let i = 0; i < len; i++) ch[i] = Math.random() * 2 - 1;
    if (FNA.ctx.state === 'suspended') FNA.ctx.resume().catch(() => {});
    fnAudioApplyGain();
    return true;
  } catch { FNA.ctx = null; FNA.master = null; FNA.noiseBuf = null; return false; }
}

/**
 * Autoplay-policy unlock: called on every pointerdown/keydown until it
 * succeeds once (idempotent via FNA.unlocked). Creates + resumes the
 * context inside the gesture's call stack — the only place browsers
 * allow it.
 */
function fnAudioUnlock() {
  if (FNA.unlocked) return;
  if (fnAudioInit()) FNA.unlocked = true;
}

function fnTone(freq, dur, vol, type, slideTo) {
  if (!FNA.on || !FNA.ctx || !freq) return;
  const t = FNA.ctx.currentTime;
  const osc = FNA.ctx.createOscillator();
  const gain = FNA.ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(Math.max(20, slideTo), t + dur);
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(vol, t + 0.008);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(FNA.sfx.gain || FNA.master);
  osc.start(t); osc.stop(t + dur + 0.02);
}

function fnNoise(dur, vol, f0, f1, type) {
  if (!FNA.on || !FNA.ctx || !FNA.noiseBuf) return;
  const t = FNA.ctx.currentTime;
  const src = FNA.ctx.createBufferSource();
  src.buffer = FNA.noiseBuf;
  src.loop = true;
  const flt = FNA.ctx.createBiquadFilter();
  flt.type = type || 'bandpass';
  flt.Q.value = 0.9;
  flt.frequency.setValueAtTime(f0, t);
  flt.frequency.exponentialRampToValueAtTime(Math.max(30, f1), t + dur);
  const gain = FNA.ctx.createGain();
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(vol, t + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(flt); flt.connect(gain); gain.connect(FNA.sfx.gain || FNA.master);
  src.start(t); src.stop(t + dur + 0.02);
}

/* ---- PHASE 2B: Background music --------------------------------------
   Procedural arcade soundtrack on the SAME AudioContext as the effects
   (no second context, no files, no network). Design goals:
     • energetic but pleasant: minor-key groove (Am) with a bright
       pentatonic lead, warm sub bass, offbeat hats — steady 120 BPM,
     • non-irritating over long sessions: the lead only plays bars 5–8
       of each 8-bar phrase, entries are sparse/syncopated so nothing
       hammers, and every voice has soft attack/release envelopes,
     • smooth transitions: chord pads glide bar-to-bar; the arrangement
       naturally rotates A/B halves through the 128-step loop,
     • seamless looping: a look-ahead scheduler keeps ~0.35 s of music
       scheduled ahead of the audio clock, so the wrap from step 127 → 0
       is inaudible — there is no gap and no click.

   Signal flow: voices → bgm.gain (fade bus) → compressor → master.
   The compressor (−18 dB threshold, 3:1, fast release) guarantees
   music + effects never clip the output when they stack.

   Lifecycle: fnBgmStart() on first play, fnBgmSetPlaying(false) while
   paused, fade-out + full node teardown on game over / route leave.
   Restart cycles are stable: one interval, one generation counter.  */

/** Music constants: 120 BPM; one 16th-note = spb/4 seconds. */
const FN_BGM = {
  bpm: 120,
  spb: 0.5,                        // seconds per beat (60/120)
  // Am progression, one chord per bar over 8 bars: Am F C G ×2 — the
  // classic uplifting loop; voiced low-mid so it stays warm.
  chords: [
    [220.00, 261.63, 329.63],      // Am  (A3 C4 E4)
    [174.61, 220.00, 261.63],      // F   (F3 A3 C4)
    [130.81, 196.00, 261.63],      // C   (C3 G3 C4)
    [196.00, 246.94, 293.66],      // G   (G3 B3 D4)
    [220.00, 261.63, 329.63],      // Am
    [174.61, 220.00, 261.63],      // F
    [261.63, 329.63, 392.00],      // C   (C4 E4 G4) — lift on the last pair
    [196.00, 246.94, 293.66],      // G
  ],
  // A-minor pentatonic lead pool (A4 up): bright, always consonant over
  // the progression, so improvised-sounding patterns never sour.
  leadPool: [440.00, 523.25, 587.33, 659.25, 783.99, 880.00],
  // Chord roots for the bass line (one octave below the voicing).
  roots: [110.00, 87.31, 65.41, 98.00, 110.00, 87.31, 130.81, 98.00],
};

/** True when the music system has a live bus chain. */
function fnBgmReady() {
  return !!(FNA.ctx && FNA.bgm.gain && FNA.bgm.comp);
}

/** Current music output level (fade-bus target): 0 unless playing. */
function fnBgmTarget() {
  return (FNA.on && FNA.bgm.started && FNA.bgm.active !== false) ? FNA.bgm.volume : 0;
}

/** Smoothly move the music fade bus to its target level. */
function fnBgmApplyGain() {
  if (!fnBgmReady()) return;
  try {
    FNA.bgm.gain.gain.setTargetAtTime(fnBgmTarget(), FNA.ctx.currentTime, 0.12);
  } catch { /* ctx mid-close */ }
}

/** One synth voice. attack/release in ms; optional freq glide. All
 *  voices connect into the fade bus; WebAudio frees them once stopped. */
function fnBgmVoice(freq, t, dur, vol, type, attack, release, glideTo, glideT) {
  const ctx = FNA.ctx;
  const osc = ctx.createOscillator();
  const g = ctx.createGain();
  osc.type = type || 'triangle';
  osc.frequency.setValueAtTime(freq, t);
  if (glideTo) {
    osc.frequency.exponentialRampToValueAtTime(Math.max(20, glideTo), t + (glideT || dur));
  }
  // Soft attack / linear sustain / exponential release: no clicks.
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(vol, t + attack / 1000);
  g.gain.setValueAtTime(vol, t + Math.max(attack / 1000, dur - release / 1000));
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g); g.connect(FNA.bgm.gain);
  osc.start(t); osc.stop(t + dur + 0.05);
}

/** Percussive hat: high-passed noise burst into the fade bus. */
function fnBgmHat(t, vol) {
  const ctx = FNA.ctx;
  const src = ctx.createBufferSource();
  src.buffer = FNA.noiseBuf;
  const flt = ctx.createBiquadFilter();
  flt.type = 'highpass';
  flt.frequency.value = 7000;
  const g = ctx.createGain();
  g.gain.setValueAtTime(vol, t);
  g.gain.exponentialRampToValueAtTime(0.0001, t + 0.04);
  src.connect(flt); flt.connect(g); g.connect(FNA.bgm.gain);
  src.start(t); src.stop(t + 0.06);
}

/** Percussive kick: sine drop 150→45 Hz. */
function fnBgmKick(t, vol) {
  fnBgmVoice(150, t, 0.16, vol, 'sine', 2, 60, 45, 0.14);
}

/** Snare-ish rim: short triangle blip with a downward tick. */
function fnBgmSnare(t, vol) {
  fnBgmVoice(190, t, 0.09, vol, 'triangle', 2, 50, 150, 0.07);
}

/** Schedule ONE 16th-step of music at audio time t. step counts up
 *  forever; the pattern is step % 128 (8 bars × 16 sixteenths). */
function fnBgmStep(step, t) {
  const s = step % 128;                 // 8 bars × 16 sixteenths
  const bar = (s / 16) | 0;             // 0..7
  const sixteenth = s % 16;             // 0..15
  const B = FN_BGM.spb;                 // seconds per beat
  const st = B / 4;                     // seconds per 16th

  // --- drums: kick on 1 & 3, snare on 2 & 4, hats on the offbeats ---
  if (sixteenth === 0) fnBgmKick(t, 0.50);
  if (sixteenth === 8) fnBgmKick(t, 0.30);
  if (sixteenth === 4 || sixteenth === 12) fnBgmSnare(t, 0.22);
  if (sixteenth % 2 === 1) fnBgmHat(t, 0.10);   // offbeat 8ths
  if (sixteenth === 14) fnBgmHat(t, 0.16);      // little fill tick

  // --- chord pad: retrigger every bar, warm triangle stack ---
  if (sixteenth === 0) {
    const ch = FN_BGM.chords[bar];
    fnBgmVoice(ch[0], t, B * 3.6, 0.10, 'triangle', 30, 400);
    fnBgmVoice(ch[1], t, B * 3.6, 0.085, 'triangle', 30, 0);
    fnBgmVoice(ch[2], t, B * 3.3, 0.07, 'sine', 40, 0);   // top note soft
  }

  // --- bass: root on the downbeat, octave pop on the 'and' of 2 & 4 ---
  if (sixteenth === 0) fnBgmVoice(FN_BGM.roots[bar], t, B * 0.9, 0.30, 'sine', 5, 90);
  if (sixteenth === 6 || sixteenth === 14) {
    fnBgmVoice(FN_BGM.roots[bar] * 2, t, B * 0.4, 0.20, 'triangle', 5, 60);
  }

  // --- lead: pentatonic melody in bars 5–8 only, sparse syncopation so
  //     it breathes; deterministic by step so it feels composed, not RNG. ---
  if (bar >= 4 && sixteenth % 2 === 0) {
    if ((step * 13 + bar) % 5 !== 0) {
      const idx = (step * 7 + bar * 3) % FN_BGM.leadPool.length;
      fnBgmVoice(FN_BGM.leadPool[idx], t, st * 3, 0.11, 'square', 8, 70);
    }
  }
}

/** Look-ahead scheduler tick: keep ~0.35 s of steps scheduled. */
function fnBgmSchedule() {
  if (!fnBgmReady() || !FNA.bgm.started) return;
  const horizon = FNA.ctx.currentTime + 0.35;
  while (FNA.bgm.nextT < horizon) {
    fnBgmStep(FNA.bgm.step, FNA.bgm.nextT);
    FNA.bgm.step++;
    FNA.bgm.nextT += FN_BGM.spb / 4;
  }
}

/** Start the music: idempotent — safe on every fnStart(). */
function fnBgmStart() {
  if (!fnAudioInit()) return;           // also resumes a suspended ctx
  if (FNA.bgm.started) return;          // already running: no dup instances
  FNA.bgm.started = true;
  FNA.bgm.active = true;
  FNA.bgm.step = 0;
  FNA.bgm.nextT = FNA.ctx.currentTime + 0.1;
  fnBgmApplyGain();                     // fade in
  if (FNA.bgm.timer) clearInterval(FNA.bgm.timer);
  FNA.bgm.timer = setInterval(fnBgmSchedule, 120);
  fnBgmSchedule();                      // schedule immediately
}

/** Pause/resume WITHOUT stopping the scheduler: the music fades to
 *  silence and back (0.12 s time-constant); grid position preserved. */
function fnBgmSetPlaying(playing) {
  if (!FNA.bgm.started) return;
  if (playing === FNA.bgm.active) return;
  FNA.bgm.active = playing;
  fnBgmApplyGain();
}

/** Stop the music and tear down every node/timer cleanly. */
function fnBgmStop() {
  if (FNA.bgm.timer) { clearInterval(FNA.bgm.timer); FNA.bgm.timer = 0; }
  FNA.bgm.started = false;
  FNA.bgm.active = false;
  // Fade out fast, then disconnect the bus after the tail has passed.
  if (fnBgmReady()) {
    try {
      FNA.bgm.gain.gain.setTargetAtTime(0, FNA.ctx.currentTime, 0.08);
      const g = FNA.bgm.gain, comp = FNA.bgm.comp;
      setTimeout(() => { try { g.disconnect(); comp.disconnect(); } catch { /* gone */ } }, 500);
    } catch { /* ctx mid-close */ }
  }
}

function fnSfx(kind) {
  switch (kind) {
    // 'slice' and 'combo' were replaced in PHASE 2C by fnSfxSlice() /
    // fnSfxCombo() — richer, pitch-varied, budget-gated voices on the
    // SFX bus. Every other effect below is unchanged.
    case 'splat':  fnNoise(0.12, 0.16, 750, 210, 'lowpass'); break;
    case 'boom':   fnNoise(0.55, 0.5, 950, 55, 'lowpass');
                   fnTone(150, 0.5, 0.4, 'sine', 36);
                   fnTone(92, 0.4, 0.25, 'triangle', 30); break;
    case 'crit':   fnTone(1568, 0.09, 0.15, 'triangle', 1046); break;
    case 'bonus':  fnTone(880, 0.08, 0.14, 'triangle');
                   setTimeout(() => fnTone(1318, 0.12, 0.14, 'triangle'), 80); break;
    case 'miss':   fnTone(330, 0.14, 0.13, 'sawtooth', 165); break;
    case 'over':   [392, 311, 233, 155].forEach((f, i) =>
                     setTimeout(() => fnTone(f, 0.16, 0.14, 'sawtooth'), i * 130)); break;
    case 'win':    [523, 659, 784, 1047].forEach((f, i) =>
                     setTimeout(() => fnTone(f, 0.12, 0.13), i * 100)); break;
    case 'ui':     fnTone(880, 0.04, 0.07); break;
    case 'fuse':   fnNoise(0.06, 0.05, 5200, 3800); break;
  }
}

function fnAudioDispose() {
  fnBgmStop();                       // PHASE 2B: music timer + bus first
  if (FNA.ctx) { try { FNA.ctx.close(); } catch { /* already closed */ } }
  FNA.ctx = null; FNA.master = null; FNA.noiseBuf = null;
  FNA.sfx.gain = null; FNA.sfx.comp = null; FNA.sfx.live = 0;  // PHASE 2B/2C
  FNA.bgm.gain = null; FNA.bgm.comp = null;
  // Next mount must be able to unlock again after its first gesture.
  FNA.unlocked = false;
}

/** PHASE 2A: set master volume (0..1). Safe before init. */
function fnAudioSetVolume(v) {
  FNA.volume = Math.min(1, Math.max(0, +v || 0));
  fnAudioApplyGain();
}

/** PHASE 2B: independent music level (0..1), under the master gate.
 *  Safe before init (value applies once the bus exists). */
function fnBgmSetVolume(v) {
  FNA.bgm.volume = Math.min(1, Math.max(0, +v || 0));
  fnBgmApplyGain();
}

/** PHASE 2B: independent effects level (0..1), under the master gate.
 *  The bus exists from Phase 2B onward; the value is stored either way
 *  so a pre-init call is honored when the ctx opens. */
function fnSfxSetVolume(v) {
  FNA.sfx.volume = Math.min(1, Math.max(0, +v || 0));
  if (FNA.sfx.gain && FNA.ctx) {
    try { FNA.sfx.gain.gain.setTargetAtTime(FNA.sfx.volume, FNA.ctx.currentTime, 0.015); }
    catch { /* ctx mid-close */ }
  }
}

/* ---- PHASE 2C: slicing & combo voices --------------------------------
   Polished arcade SFX on the existing Phase 2A/2B architecture — no new
   AudioContext, no engine rewrite. Everything routes through the shared
   FNA.sfx.gain bus (→ sfx compressor → master), so the Sound On/Off
   gate and fnSfxSetVolume() both apply automatically.

   Node budget: a hard cap of FN_SFX_MAX live voices. Each voice
   self-decrements on ended, so rapid consecutive slices stay responsive
   (never queued/dropped below the cap) while an uncontrolled pile of
   oscillators can never build up. The cap is generous (24) — it exists
   for safety, not throttling: normal play uses ~4–8 voices at once. */
const FN_SFX_MAX = 24;

/** Budget-gated pitched voice into the SFX bus. Returns false when the
 *  budget is exhausted (callers just skip — gameplay is unaffected).
 *  PHASE 2D: optional `delay` (seconds) schedules the voice on the
 *  audio clock — sample-accurate sequences with zero setTimeout timers. */
function fnSfxVoice(freq, dur, vol, type, slideTo, detuneCents, delay) {
  if (!FNA.on || !FNA.ctx || !FNA.sfx.gain) return false;
  if (FNA.sfx.live >= FN_SFX_MAX) return false;      // voice budget
  const t = FNA.ctx.currentTime + (delay || 0);
  const osc = FNA.ctx.createOscillator();
  const g = FNA.ctx.createGain();
  osc.type = type || 'sine';
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(Math.max(20, slideTo), t + dur);
  if (detuneCents) osc.detune.setValueAtTime(detuneCents, t);
  // Short click-free envelope: fast attack, quick exponential release.
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(vol, t + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g); g.connect(FNA.sfx.gain);
  FNA.sfx.live++;
  osc.onended = () => { FNA.sfx.live--; };           // self-cleaning
  osc.start(t); osc.stop(t + dur + 0.02);
  return true;
}

/** Budget-gated noise burst into the SFX bus (same cap, same cleanup).
 *  PHASE 2D: optional `delay` (seconds) — audio-clock scheduling. */
function fnSfxNoise(dur, vol, f0, f1, type, delay) {
  if (!FNA.on || !FNA.ctx || !FNA.noiseBuf || !FNA.sfx.gain) return false;
  if (FNA.sfx.live >= FN_SFX_MAX) return false;      // voice budget
  const t = FNA.ctx.currentTime + (delay || 0);
  const src = FNA.ctx.createBufferSource();
  src.buffer = FNA.noiseBuf;
  const flt = FNA.ctx.createBiquadFilter();
  flt.type = type || 'bandpass';
  flt.Q.value = 0.9;
  flt.frequency.setValueAtTime(f0, t);
  flt.frequency.exponentialRampToValueAtTime(Math.max(30, f1), t + dur);
  const g = FNA.ctx.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(vol, t + 0.005);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(flt); flt.connect(g); g.connect(FNA.sfx.gain);
  FNA.sfx.live++;
  src.onended = () => { FNA.sfx.live--; };           // self-cleaning
  src.start(t); src.stop(t + dur + 0.02);
  return true;
}

/* ---- PHASE 2C: slice & combo sound design -----------------------------
   SLICE — crisp and juicy, with per-cut pitch variation so consecutive
   slices never sound identical:
     • a fast downward noise sweep (blade 'shhk') whose start frequency
       and length vary ±20% per cut,
     • a wet 'splat' body: low-passed noise thump, also varied,
     • a short sub 'thock' tone whose pitch varies ±3 semitones.
   Levels are conservative (≤0.30) and every voice is < 150 ms, so rapid
   slicing stays satisfying and never harsh. Stacking safety comes from
   the bus compressor added in this phase, not from lowering levels.

   COMBO MILESTONES — increasingly rewarding, one distinct call per
   milestone crossing (3 / 4 / 5+), driven by the EXISTING combo logic
   (FNG.combo after increment, crit flag at ≥3): scoring, windows and
   points are untouched:
     • 3× : bright two-note rising fifth (E5→B5) — "nice"
     • 4× : three-note rising arpeggio (C5 E5 G5) — "great"
     • 5+ : four-note fanfare with an octave top (D5 F#5 A5 D6) plus a
       shimmer — the jackpot sound; each further fruit in the same swipe
       re-triggers only the top sparkle, not the whole fanfare.        */

/** Deterministic-but-varied pitch helper: ratio in [1-spread, 1+spread]. */
function fnSfxVar(spread) {
  return 1 + (Math.random() * 2 - 1) * spread;
}

/** The polished slice sound. Called on EVERY successful fruit cut. */
function fnSfxSlice() {
  const v = fnSfxVar(0.20);
  // blade swish: fast high→low noise sweep, varied length & brightness
  fnSfxNoise(0.055 + Math.random() * 0.03, 0.30, 3200 * v, 700 / v, 'bandpass');
  // wet splat body: low-passed thump under the swish
  fnSfxNoise(0.09 + Math.random() * 0.04, 0.22, 900 * v, 180, 'lowpass');
  // sub 'thock': grounding tone, ±3 semitones per cut
  fnSfxVoice(175 * v, 0.07, 0.16, 'sine', 82 * v);
}

/** Combo milestone sounds (3 / 4 / 5+). `n` is the CURRENT combo count
 *  (after increment). Only the crossing plays the full phrase. */
function fnSfxCombo(n) {
  if (n === 3) {                       // "nice" — rising fifth
    fnSfxVoice(659.25, 0.10, 0.20, 'triangle');
    fnSfxVoice(987.77, 0.14, 0.20, 'triangle');
  } else if (n === 4) {                // "great" — rising arpeggio
    [523.25, 659.25, 783.99].forEach((f, i) =>
      fnSfxVoice(f, 0.10, 0.19, 'triangle', null, 0));
    fnSfxVoice(1567.98, 0.08, 0.10, 'sine');          // sparkle top
  } else if (n >= 5) {                 // "jackpot" — fanfare + shimmer
    [587.33, 739.99, 932.33, 1174.66].forEach((f, i) =>
      fnSfxVoice(f, 0.11, 0.20, 'triangle'));
    fnSfxVoice(2349.32, 0.10, 0.09, 'sine');          // octave shimmer
  }
}

/* ---- PHASE 2D: game-event sounds --------------------------------------
   Polished arcade SFX for the big moments, built on the Phase 2C voice
   helpers (same bus, same budget, same cleanup). All sequences use the
   audio-clock `delay` parameter — sample-accurate phrasing with ZERO
   setTimeout timers to leak or dedupe.

   Every sound here routes: voices → FNA.sfx.gain → sfx compressor →
   master. Sound On/Off and fnSfxSetVolume() apply automatically; the
   Phase 2B music bus is untouched and stays independently controlled.

     • BOOM      — bomb explosion: punchy low impact + noise burst
     • LIFELOST  — descending two-tone penalty (not irritating)
     • OVER      — memorable descending arcade ending (once per run)
     • RECORD    — celebratory flourish, only on a GENUINE new best
     • COUNTDOWN — 3-2-1 ticks + GO cue at every game start
     • UI        — subtle click for menu actions (existing 'ui' case,
                   now richer: soft tick + airy chime)                  */

/** Bomb explosion: punchy short low impact + subtle noise burst.
 *  Distinct from slicing — no pitched blade components at all. */
function fnSfxBoom() {
  fnSfxNoise(0.22, 0.34, 1400, 60, 'lowpass');            // body burst
  fnSfxVoice(110, 0.30, 0.40, 'sine', 38);                // deep impact
  fnSfxVoice(70, 0.24, 0.26, 'triangle', 30, 0, 0.02);    // sub tail
}

/** Life lost / miss penalty: clear descending tone pair, gentle enough
 *  not to irritate. Used when Classic loses a life. */
function fnSfxLifeLost() {
  fnSfxVoice(392.00, 0.12, 0.18, 'triangle', 311.13);     // G4 → Eb4
  fnSfxVoice(311.13, 0.16, 0.16, 'triangle', 233.08, 0, 0.12); // Eb4 → Bb3
}

/** Game over: memorable descending arcade ending. Plays exactly once
 *  per transition (fnGameOver sets state='over' first, and every caller
 *  guards on it — re-entry is same-frame impossible). */
function fnSfxOver() {
  // four descending notes, then a low final thud
  [392.00, 329.63, 261.63, 174.61].forEach((f, i) =>
    fnSfxVoice(f, 0.16, 0.20, 'triangle', null, 0, i * 0.13));
  fnSfxVoice(87.31, 0.40, 0.30, 'sine', 55, 0, 0.52);     // final low drop
}

/** New high score: positive celebratory flourish. Triggered ONLY on a
 *  genuine record (score > previous best before the update). */
function fnSfxRecord() {
  // rising major arpeggio + octave sparkle + quick shimmer roll
  [523.25, 659.25, 783.99, 1046.50].forEach((f, i) =>
    fnSfxVoice(f, 0.14, 0.20, 'triangle', null, 0, i * 0.09));
  fnSfxVoice(1318.51, 0.16, 0.16, 'sine', null, 0, 0.36);
  fnSfxVoice(1567.98, 0.18, 0.14, 'sine', null, 0, 0.44);
}

/** Game start countdown: three short ticks and a distinct GO cue.
 *  Audio-clock scheduled — immune to restart races and render storms;
 *  each fnStart() call supersedes any earlier one naturally because all
 *  voices are < 1.6 s and budget-gated. */
function fnSfxCountdown() {
  fnSfxVoice(880.00, 0.07, 0.16, 'square', null, 0, 0.00);   // tick 1
  fnSfxVoice(880.00, 0.07, 0.16, 'square', null, 0, 0.45);   // tick 2
  fnSfxVoice(880.00, 0.07, 0.16, 'square', null, 0, 0.90);   // tick 3
  fnSfxVoice(1318.51, 0.22, 0.24, 'triangle', 1760, 0, 1.35); // GO!
}

/** Subtle menu click: soft tick + airy chime. */
function fnSfxUi() {
  fnSfxVoice(1320, 0.03, 0.10, 'sine');
  fnSfxVoice(1980, 0.05, 0.06, 'sine', null, 0, 0.03);
}

function fnSoundButton() {
  const btn = document.getElementById('fnSoundBtn');
  if (!btn) return;
  btn.setAttribute('aria-pressed', String(FNA.on));
  btn.classList.toggle('on', FNA.on);
  const lbl = document.getElementById('fnSoundLbl');
  if (lbl) lbl.textContent = FNA.on ? 'sound on' : 'sound off';
}

/* ---- PHASE 2E: audio controls integration -----------------------------
   Two independent sliders (music ♫ / effects ✦) under the existing
   master Sound On/Off gate. Levels persist in localStorage, remounts
   restore them, and all changes ride the existing smooth setTargetAtTime
   ramps — no engine rebuild, buses/compressors/budget untouched.

   Volume architecture (unchanged from 2B/2C):
     master (FNA.on × FNA.volume)
       ├── bgm bus  (bgm.volume) → bgm compressor
       └── sfx bus  (sfx.volume) → sfx compressor → voice budget      */

/** Load saved audio prefs once per mount; fall back to defaults. */
function fnAudioLoadPrefs() {
  try {
    const m = parseFloat(localStorage.getItem('fnVolMusic'));
    const s = parseFloat(localStorage.getItem('fnVolSfx'));
    if (Number.isFinite(m)) FNA.bgm.volume = Math.min(1, Math.max(0, m));
    if (Number.isFinite(s)) FNA.sfx.volume = Math.min(1, Math.max(0, s));
  } catch { /* storage unavailable */ }
}

/** Sync every control's visual state from FNA (mute + slider levels). */
function fnAudioSyncControls() {
  fnSoundButton();
  const wrapM = document.getElementById('fnVolMusic');
  const wrapS = document.getElementById('fnVolSfx');
  const m = document.getElementById('fnBgmVol');
  const s = document.getElementById('fnSfxVol');
  if (m) m.value = String(Math.round(FNA.bgm.volume * 100));
  if (s) s.value = String(Math.round(FNA.sfx.volume * 100));
  // Mute mirrors onto the slider wrappers: dimmed when sound is off.
  if (wrapM) wrapM.classList.toggle('muted', !FNA.on);
  if (wrapS) wrapS.classList.toggle('muted', !FNA.on);
}

/** Master toggle now also refreshes the slider visuals. */
function fnSoundToggle() {
  FNA.on = !FNA.on;
  if (FNA.on && !fnAudioInit()) FNA.on = false;
  fnAudioApplyGain();               // mute/unmute reaches the graph immediately
  fnBgmApplyGain();                 // PHASE 2B: music follows the master gate
  fnAudioSyncControls();            // PHASE 2E: sliders mirror the mute state
}

/* ---- Fruit catalog --------------------------------------------------
   Every fruit is drawn procedurally: a body painter used for both whole
   fruits and their sliced halves. `draw(ctx, r)` paints the whole body
   centred on (0,0) with radius r; `cut` describes the exposed face when
   halved (flesh gradient, seed layout, rim colour). No emoji, no images.
   `pts` = base score; `r` = collision radius in world units.           */
const FN_FRUITS = {
  watermelon: {
    r: 46, pts: 2, juice: '#ff5d6e',
    draw(ctx, r) {
      // dark striped rind
      const g = ctx.createRadialGradient(-r * .3, -r * .3, r * .1, 0, 0, r);
      g.addColorStop(0, '#2f8f5b'); g.addColorStop(.75, '#1f6b43'); g.addColorStop(1, '#17553a');
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
      // stripes follow the sphere's curvature
      ctx.strokeStyle = 'rgba(12,42,26,.85)'; ctx.lineWidth = r * .13;
      for (let i = -2; i <= 2; i++) {
        ctx.beginPath();
        ctx.ellipse(i * r * .38, 0, r * .16, r * .96, 0, 0, Math.PI * 2);
        ctx.stroke();
      }
      // sheen
      const hl = ctx.createRadialGradient(-r * .38, -r * .42, 0, -r * .38, -r * .42, r * .7);
      hl.addColorStop(0, 'rgba(255,255,255,.28)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#1f6b43', flesh: [['#ff7d8d', 0], ['#ff4d6d', .55], ['#e63950', 1]],
      seeds: { n: 9, color: '#231207' }, core: 'rgba(255,255,255,.22)',
    },
  },
  apple: {
    r: 34, pts: 1, juice: '#ffe08a',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .35, -r * .35, r * .1, 0, 0, r * 1.05);
      g.addColorStop(0, '#ff6b57'); g.addColorStop(.6, '#e63946'); g.addColorStop(1, '#b02a37');
      ctx.fillStyle = g;
      ctx.beginPath();
      // two-lobe apple silhouette
      ctx.moveTo(0, -r * .92);
      ctx.bezierCurveTo(r * .45, -r * 1.02, r * 1.02, -r * .5, r * .92, r * .18);
      ctx.bezierCurveTo(r * .84, r * .82, r * .38, r * 1.02, 0, r * .88);
      ctx.bezierCurveTo(-r * .38, r * 1.02, -r * .84, r * .82, -r * .92, r * .18);
      ctx.bezierCurveTo(-r * 1.02, -r * .5, -r * .45, -r * 1.02, 0, -r * .92);
      ctx.fill();
      // stem + leaf
      ctx.strokeStyle = '#6b4423'; ctx.lineWidth = r * .11; ctx.lineCap = 'round';
      ctx.beginPath(); ctx.moveTo(0, -r * .86); ctx.quadraticCurveTo(r * .06, -r * 1.2, r * .16, -r * 1.32); ctx.stroke();
      ctx.fillStyle = '#3f9e4d';
      ctx.beginPath();
      ctx.ellipse(r * .42, -r * 1.14, r * .34, r * .16, -.5, 0, Math.PI * 2); ctx.fill();
      const hl = ctx.createRadialGradient(-r * .3, -r * .4, 0, -r * .3, -r * .4, r * .6);
      hl.addColorStop(0, 'rgba(255,255,255,.35)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#e63946', flesh: [['#fff7d6', 0], ['#ffe8a3', .5], ['#ffd166', 1]],
      seeds: { n: 2, color: '#5c3a1e' }, core: 'rgba(214,158,46,.4)',
    },
  },
  orange: {
    r: 33, pts: 1, juice: '#ffb23e',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .32, -r * .32, r * .1, 0, 0, r);
      g.addColorStop(0, '#ffc04d'); g.addColorStop(.65, '#ff9838'); g.addColorStop(1, '#e07b1f');
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
      // dimpled skin: subtle stipple
      ctx.fillStyle = 'rgba(255,255,255,.16)';
      for (let i = 0; i < 14; i++) {
        const a = i * 2.399, rr = r * Math.sqrt((i + .5) / 14) * .9;
        ctx.beginPath(); ctx.arc(Math.cos(a) * rr, Math.sin(a) * rr, r * .045, 0, Math.PI * 2); ctx.fill();
      }
      // tiny green calyx
      ctx.fillStyle = '#3f9e4d';
      ctx.beginPath();
      for (let i = 0; i < 5; i++) {
        const a = -Math.PI / 2 + i * (Math.PI * 2 / 5);
        ctx.lineTo(Math.cos(a) * r * .16, Math.sin(a) * r * .16 - r * .9);
      }
      ctx.closePath(); ctx.fill();
      const hl = ctx.createRadialGradient(-r * .34, -r * .36, 0, -r * .34, -r * .36, r * .62);
      hl.addColorStop(0, 'rgba(255,255,255,.4)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#ff9838', flesh: [['#ffd97a', 0], ['#ffb23e', .5], ['#ff9838', 1]],
      segments: true, core: 'rgba(255,220,140,.5)',
    },
  },
  lemon: {
    r: 30, pts: 1, juice: '#ffe97a',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .3, -r * .3, r * .1, 0, 0, r);
      g.addColorStop(0, '#fff59b'); g.addColorStop(.6, '#ffe23e'); g.addColorStop(1, '#e0b81f');
      ctx.fillStyle = g;
      ctx.beginPath();
      // lemon: ellipse with nubbed tips
      ctx.moveTo(-r * 1.08, 0);
      ctx.quadraticCurveTo(-r * 1.02, -r * .72, 0, -r * .74);
      ctx.quadraticCurveTo(r * 1.02, -r * .72, r * 1.08, 0);
      ctx.quadraticCurveTo(r * 1.02, r * .72, 0, r * .74);
      ctx.quadraticCurveTo(-r * 1.02, r * .72, -r * 1.08, 0);
      ctx.fill();
      const hl = ctx.createRadialGradient(-r * .3, -r * .34, 0, -r * .3, -r * .34, r * .6);
      hl.addColorStop(0, 'rgba(255,255,255,.45)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.ellipse(0, 0, r * 1.05, r * .72, 0, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#ffe23e', flesh: [['#fffbc4', 0], ['#fff59b', .5], ['#ffe97a', 1]],
      segments: true, core: 'rgba(255,250,200,.55)',
    },
  },
  kiwi: {
    r: 28, pts: 1, juice: '#b6e388',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .3, -r * .3, r * .1, 0, 0, r);
      g.addColorStop(0, '#9c7a4d'); g.addColorStop(.7, '#7a5c36'); g.addColorStop(1, '#5f4728');
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.ellipse(0, 0, r, r * 1.08, 0, 0, Math.PI * 2); ctx.fill();
      // fuzzy skin hint
      ctx.strokeStyle = 'rgba(60,42,20,.5)'; ctx.lineWidth = 1;
      for (let i = 0; i < 10; i++) {
        const a = i * (Math.PI * 2 / 10);
        ctx.beginPath();
        ctx.moveTo(Math.cos(a) * r * .96, Math.sin(a) * r * 1.03);
        ctx.lineTo(Math.cos(a) * r * 1.03, Math.sin(a) * r * 1.1);
        ctx.stroke();
      }
      const hl = ctx.createRadialGradient(-r * .3, -r * .3, 0, -r * .3, -r * .3, r * .55);
      hl.addColorStop(0, 'rgba(255,255,255,.25)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.ellipse(0, 0, r, r * 1.08, 0, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#7a5c36', flesh: [['#d8f3b6', 0], ['#a4d977', .6], ['#8ac926', 1]],
      seeds: { n: 14, ring: true, color: '#231207' }, core: 'rgba(255,255,255,.75)',
    },
  },
  banana: {
    r: 34, pts: 1, juice: '#ffe97a',
    draw(ctx, r) {
      // crescent along a curved spine
      ctx.save();
      const path = new Path2D();
      path.moveTo(-r * 1.05, -r * .55);
      path.quadraticCurveTo(0, r * 1.15, r * 1.05, -r * .55);
      path.quadraticCurveTo(r * .55, -r * .78, r * .3, -r * .68);
      path.quadraticCurveTo(0, r * .58, -r * .3, -r * .68);
      path.quadraticCurveTo(-r * .55, -r * .78, -r * 1.05, -r * .55);
      const g = ctx.createLinearGradient(0, -r, 0, r);
      g.addColorStop(0, '#fff1a8'); g.addColorStop(.55, '#ffe23e'); g.addColorStop(1, '#e0b81f');
      ctx.fillStyle = g; ctx.fill(path);
      // tip shading
      ctx.fillStyle = 'rgba(90,60,10,.5)';
      ctx.beginPath(); ctx.arc(-r * 1.02, -r * .56, r * .09, 0, Math.PI * 2); ctx.fill();
      ctx.beginPath(); ctx.arc(r * 1.02, -r * .56, r * .09, 0, Math.PI * 2); ctx.fill();
      const hl = ctx.createLinearGradient(0, -r * .5, 0, r * .4);
      hl.addColorStop(0, 'rgba(255,255,255,.5)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl; ctx.fill(path);
      ctx.restore();
    },
    cut: {
      rim: '#ffe23e', flesh: [['#fffde0', 0], ['#fff1a8', .6], ['#ffe97a', 1]],
      core: 'rgba(200,170,80,.35)', slim: true,
    },
  },
  strawberry: {
    r: 27, pts: 1, juice: '#ff6b81',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .25, -r * .3, r * .08, 0, 0, r * 1.1);
      g.addColorStop(0, '#ff7d8d'); g.addColorStop(.55, '#e63950'); g.addColorStop(1, '#b3243c');
      ctx.fillStyle = g;
      ctx.beginPath();
      // heart-ish berry body
      ctx.moveTo(0, r * 1.02);
      ctx.bezierCurveTo(-r * .95, r * .45, -r * .82, -r * .55, 0, -r * .62);
      ctx.bezierCurveTo(r * .82, -r * .55, r * .95, r * .45, 0, r * 1.02);
      ctx.fill();
      // seeds
      ctx.fillStyle = 'rgba(255,235,180,.9)';
      for (let i = 0; i < 9; i++) {
        const a = (i / 9) * Math.PI * 2 + .4;
        const rr = r * (.28 + (i % 3) * .2);
        ctx.beginPath();
        ctx.ellipse(Math.cos(a) * rr, Math.sin(a) * rr * .8, r * .05, r * .08, a, 0, Math.PI * 2);
        ctx.fill();
      }
      // leafy calyx
      ctx.fillStyle = '#3f9e4d';
      for (let i = 0; i < 5; i++) {
        const a = -Math.PI / 2 + (i - 2) * .5;
        ctx.beginPath();
        ctx.ellipse(Math.cos(a) * r * .34, -r * .58 + Math.sin(a) * r * .18, r * .3, r * .13, a, 0, Math.PI * 2);
        ctx.fill();
      }
      const hl = ctx.createRadialGradient(-r * .3, -r * .4, 0, -r * .3, -r * .4, r * .55);
      hl.addColorStop(0, 'rgba(255,255,255,.4)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#e63950', flesh: [['#ffd1d9', 0], ['#ff8fa3', .55], ['#ff6b81', 1]],
      core: 'rgba(255,255,255,.35)',
    },
  },
  pineapple: {
    r: 40, pts: 2, juice: '#ffe08a',
    draw(ctx, r) {
      // barrel body
      const g = ctx.createLinearGradient(-r * .55, 0, r * .55, 0);
      g.addColorStop(0, '#b07818'); g.addColorStop(.45, '#ffcf5c'); g.addColorStop(1, '#a86e15');
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.ellipse(0, r * .12, r * .55, r * .82, 0, 0, Math.PI * 2); ctx.fill();
      // diamond lattice
      ctx.strokeStyle = 'rgba(120,80,15,.6)'; ctx.lineWidth = 1.4;
      for (let y = -r * .6; y <= r * .85; y += r * .27) {
        ctx.beginPath(); ctx.moveTo(-r * .55, y); ctx.lineTo(r * .55, y - r * .2); ctx.stroke();
        ctx.beginPath(); ctx.moveTo(-r * .55, y); ctx.lineTo(r * .55, y + r * .2); ctx.stroke();
      }
      // spiky crown
      ctx.fillStyle = '#3f9e4d';
      for (let i = -2; i <= 2; i++) {
        ctx.beginPath();
        ctx.moveTo(i * r * .22 - r * .1, -r * .6);
        ctx.quadraticCurveTo(i * r * .34, -r * 1.28, i * r * .5, -r * .72);
        ctx.closePath(); ctx.fill();
      }
      const hl = ctx.createRadialGradient(-r * .2, -r * .3, 0, -r * .2, -r * .3, r * .7);
      hl.addColorStop(0, 'rgba(255,255,255,.3)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.ellipse(0, r * .12, r * .55, r * .82, 0, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#ffcf5c', flesh: [['#fff3b0', 0], ['#ffe08a', .55], ['#ffcf5c', 1]],
      core: 'rgba(214,158,46,.5)', slim: true,
    },
  },
  coconut: {
    r: 36, pts: 2, juice: '#f5ede0',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .3, -r * .3, r * .1, 0, 0, r);
      g.addColorStop(0, '#8a6a48'); g.addColorStop(.6, '#6b4f33'); g.addColorStop(1, '#4e3a24');
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
      // fibrous husk strokes
      ctx.strokeStyle = 'rgba(40,28,16,.55)'; ctx.lineWidth = 1.3;
      for (let i = 0; i < 12; i++) {
        const a = i * (Math.PI * 2 / 12) + .2;
        ctx.beginPath();
        ctx.moveTo(Math.cos(a) * r * .35, Math.sin(a) * r * .35);
        ctx.lineTo(Math.cos(a) * r * .94, Math.sin(a) * r * .94);
        ctx.stroke();
      }
      // three germination pores
      ctx.fillStyle = '#3a2a18';
      [[-.3, -.42], [.3, -.42], [0, -.12]].forEach(([x, y]) => {
        ctx.beginPath(); ctx.arc(x * r, y * r, r * .08, 0, Math.PI * 2); ctx.fill();
      });
      const hl = ctx.createRadialGradient(-r * .32, -r * .34, 0, -r * .32, -r * .34, r * .6);
      hl.addColorStop(0, 'rgba(255,255,255,.22)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#6b4f33', flesh: [['#ffffff', 0], ['#f5ede0', .5], ['#e8dcc8', 1]],
      core: 'rgba(120,90,60,.35)',
    },
  },
  peach: {
    r: 32, pts: 1, juice: '#ffc9a3',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .3, -r * .3, r * .1, 0, 0, r);
      g.addColorStop(0, '#ffd0a3'); g.addColorStop(.5, '#ff9e6d'); g.addColorStop(1, '#e8734f');
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.moveTo(0, -r * .9);
      ctx.bezierCurveTo(r * .6, -r * .95, r * .95, -r * .3, r * .82, r * .3);
      ctx.bezierCurveTo(r * .7, r * .85, r * .3, r * 1.02, 0, r * .9);
      ctx.bezierCurveTo(-r * .3, r * 1.02, -r * .7, r * .85, -r * .82, r * .3);
      ctx.bezierCurveTo(-r * .95, -r * .3, -r * .6, -r * .95, 0, -r * .9);
      ctx.fill();
      // crease
      ctx.strokeStyle = 'rgba(200,100,60,.45)'; ctx.lineWidth = r * .06;
      ctx.beginPath(); ctx.moveTo(0, -r * .88); ctx.quadraticCurveTo(r * .12, 0, 0, r * .9); ctx.stroke();
      // leaf pair
      ctx.fillStyle = '#3f9e4d';
      ctx.beginPath(); ctx.ellipse(-r * .4, -r * .98, r * .3, r * .13, .55, 0, Math.PI * 2); ctx.fill();
      ctx.beginPath(); ctx.ellipse(r * .4, -r * .98, r * .3, r * .13, -.55, 0, Math.PI * 2); ctx.fill();
      const hl = ctx.createRadialGradient(-r * .3, -r * .36, 0, -r * .3, -r * .36, r * .58);
      hl.addColorStop(0, 'rgba(255,255,255,.42)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#ff9e6d', flesh: [['#ffe8d1', 0], ['#ffc9a3', .55], ['#ffab7d', 1]],
      core: 'rgba(200,110,70,.45)', stone: true,
    },
  },
  pomegranate: {
    r: 33, pts: 2, juice: '#ff4d6d',
    draw(ctx, r) {
      const g = ctx.createRadialGradient(-r * .3, -r * .3, r * .1, 0, 0, r);
      g.addColorStop(0, '#e8543f'); g.addColorStop(.6, '#c1121f'); g.addColorStop(1, '#8d0b17');
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.moveTo(0, -r * .78);
      ctx.bezierCurveTo(r * .72, -r * .9, r * .95, -r * .2, r * .88, r * .3);
      ctx.bezierCurveTo(r * .8, r * .85, r * .35, r * 1.0, 0, r * .92);
      ctx.bezierCurveTo(-r * .35, r * 1.0, -r * .8, r * .85, -r * .88, r * .3);
      ctx.bezierCurveTo(-r * .95, -r * .2, -r * .72, -r * .9, 0, -r * .78);
      ctx.fill();
      // crown
      ctx.fillStyle = '#8d0b17';
      ctx.beginPath();
      for (let i = 0; i < 4; i++) {
        const x = (i - 1.5) * r * .16;
        ctx.moveTo(x, -r * .78);
        ctx.lineTo(x + r * .08, -r * 1.02);
        ctx.lineTo(x + r * .16, -r * .78);
      }
      ctx.fill();
      const hl = ctx.createRadialGradient(-r * .3, -r * .34, 0, -r * .3, -r * .34, r * .6);
      hl.addColorStop(0, 'rgba(255,255,255,.32)'); hl.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = hl;
      ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
    },
    cut: {
      rim: '#c1121f', flesh: [['#ffe3e8', 0], ['#ff8fa3', .5], ['#ff4d6d', 1]],
      seeds: { n: 12, color: '#c1121f', juicy: true }, core: 'rgba(255,255,255,.3)',
    },
  },
};

/* Golden bonus fruit (Arcade mode): rare, +50 pts and a burst of sparkles. */
const FN_BONUS = {
  r: 36, pts: 50, juice: '#ffd166',
  draw(ctx, r) {
    // star-fruit silhouette rendered as a glowing golden orb with rays
    const g = ctx.createRadialGradient(-r * .25, -r * .25, r * .05, 0, 0, r);
    g.addColorStop(0, '#fff6cc'); g.addColorStop(.5, '#ffd166'); g.addColorStop(1, '#e0a521');
    ctx.fillStyle = g;
    ctx.beginPath();
    for (let i = 0; i < 10; i++) {
      const a = -Math.PI / 2 + i * (Math.PI / 5);
      const rr = i % 2 === 0 ? r * 1.06 : r * .62;
      ctx.lineTo(Math.cos(a) * rr, Math.sin(a) * rr);
    }
    ctx.closePath(); ctx.fill();
    // inner sparkle
    ctx.fillStyle = 'rgba(255,255,255,.65)';
    ctx.beginPath(); ctx.arc(-r * .18, -r * .2, r * .16, 0, Math.PI * 2); ctx.fill();
    // pulsing halo (time-driven from outside)
    ctx.strokeStyle = 'rgba(255,209,102,.55)'; ctx.lineWidth = 2;
    ctx.beginPath(); ctx.arc(0, 0, r * 1.18, 0, Math.PI * 2); ctx.stroke();
  },
  cut: {
    rim: '#ffd166', flesh: [['#fff9e0', 0], ['#ffe8a3', .55], ['#ffd166', 1]],
    core: 'rgba(255,255,255,.6)', bonus: true,
  },
};

/* ---- State ---------------------------------------------------------- */
const FNG = {
  epoch: 0, cleanup: [],
  canvas: null, ctx: null,
  w: 0, h: 0, dpr: 1,

  state: 'menu',            // menu | playing | paused | over
  mode: 'classic',          // classic | arcade
  score: 0, lives: 3, combo: 0, bestCombo: 0,
  timeMs: 0,                // arcade countdown
  graceMs: 0,               // GET READY beat before the first wave
  elapsedMs: 0,             // PHASE 1: play-time clock (pauses with pause)
  spawnTimer: 0, waveGap: 0, waveCount: 0,
  fruits: [], halves: [], particles: [], splats: [], floats: [],
  trail: [],                // blade points {x,y,t}
  swipeHits: [],            // fruit ids cut during the CURRENT swipe
  swipeLen: 0,              // px travelled this swipe (speed gate)
  shakeT: 0, shakeMag: 0,   // screen shake state
  flashT: 0,                // bomb white-flash
  raf: 0, lastT: 0, _loopEpoch: -1,
  reducedMotion: false,
  best: {
    classic: +(localStorage.getItem('fnBestClassic') || 0),
    arcade: +(localStorage.getItem('fnBestArcade') || 0),
  },
};

function fn$(id) { return document.getElementById(id); }
function fnOn(target, ev, fn, opts) {
  target.addEventListener(ev, fn, opts);
  FNG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
}

/* ---- Spawning --------------------------------------------------------
   Waves launch from below the bottom edge at spread-out x positions.
   Each fruit gets a real parabolic arc: enough upward velocity to peak
   in the upper half of the field, plus horizontal drift toward the
   centre so nothing escapes off-screen instantly.                   */
const FN_WORLD_W = 1000;   // fixed world width; canvas scales to fit

function fnSpawnX() {
  // cluster around thirds of the field, occasionally an edge lob
  const roll = Math.random();
  if (roll < .22) return 80 + Math.random() * 140;
  if (roll < .44) return FN_WORLD_W - 220 + Math.random() * 140;
  return FN_WORLD_W * .3 + Math.random() * FN_WORLD_W * .4;
}

function fnLaunchSpeed(h) {
  // v such that apex ≈ 62–82% of field height: v² = 2·g·apex
  const g = FN_G;
  // PHASE 1: harder levels lob slightly faster (higher, quicker arcs).
  const boost = 1 + fnDiff() * 0.16;
  const apex = h * (.62 + Math.random() * .2) * boost;
  return Math.sqrt(2 * g * Math.min(apex, h * .96));
}

/** Spawn one entity (fruit or bomb). */
function fnSpawn(kindName, isBonus, isBomb) {
  const def = isBomb ? null : (isBonus ? FN_BONUS : FN_FRUITS[kindName]);
  const r = isBomb ? 30 : def.r;
  const x = fnSpawnX();
  const h = FNG.wh || 620;
  const vy = -fnLaunchSpeed(h);
  // drift toward centre: sign depends on which half it launched from
  const towardCentre = (x < FN_WORLD_W / 2 ? 1 : -1);
  const vx = towardCentre * (40 + Math.random() * 150) * (Math.random() < .18 ? -.5 : 1);
  FNG.fruits.push({
    id: ++FNG._id, kind: kindName, def, bonus: !!isBonus, bomb: !!isBomb,
    x, y: h + r + 24,
    vx, vy,
    rot: Math.random() * Math.PI * 2,
    vr: (Math.random() - .5) * 5,
    r, sliced: false, missed: false,
    fuse: Math.random() * Math.PI * 2,   // fuse-spark phase
  });
}
FNG._id = 0;

/* ---- Difficulty (PHASE 1: time-based balancing) -----------------------
   One elapsed-play clock (FNG.elapsedMs) drives every difficulty knob:
   it only advances while state === 'playing', so pausing freezes the
   ramp exactly like the arcade countdown does. Wave count is kept as a
   secondary seed for variety but never the primary difficulty input.

   fnDiff() returns the normalised difficulty d ∈ [0,1]:
     • Classic ramps over ~80 s of play (FN_DIFF_CLASSIC_MS).
     • Arcade ramps across its whole 60 s countdown.
   A smoothstep easing (slow → fast → slow) replaces the old linear
   ramp so the opening stays gentle and the ceiling arrives smoothly
   instead of snapping between difficulty steps.

   FIX: the previous Arcade gap curve was inverted — it computed
   `1 - progress` where progress grew with elapsed time, making wave 1
   the FASTEST of the run (~0–260 ms gaps) and the final seconds the
   slowest. It now scales the same way as everything else: fast waves
   only late in the run.                                          */
const FN_DIFF_CLASSIC_MS = 80000;   // Classic: full difficulty at 80 s
const FN_DIFF_ARCADE_MS = 60000;    // Arcade: full difficulty at the buzzer

/** Normalised difficulty 0→1 from the elapsed play clock. */
function fnDiff() {
  const span = FNG.mode === 'arcade' ? FN_DIFF_ARCADE_MS : FN_DIFF_CLASSIC_MS;
  const t = Math.min(1, Math.max(0, FNG.elapsedMs / span));
  return t * t * (3 - 2 * t);       // smoothstep: gentle start, no snap
}

/** Difficulty curve: gap between waves shrinks as time passes. */
function fnWaveGapMs() {
  const base = FNG.mode === 'arcade' ? 1050 : 1250;
  const floor = FNG.mode === 'arcade' ? 380 : 450;   // fastest allowed gap
  const d = fnDiff();
  return Math.round((base + (floor - base) * d) * (0.86 + Math.random() * .28));
}

/** Build one wave: 1–5 fruits (+bomb odds by difficulty), staggered. */
function fnSpawnWave() {
  FNG.waveCount++;
  // PHASE 1: difficulty is time-based (frozen on pause), not wave-count.
  const diff = fnDiff();
  const nMax = FNG.mode === 'arcade' ? 5 : 4;
  // wave size grows smoothly with difficulty: early waves stay small,
  // late waves reach (but never exceed) the original per-mode maximum
  const n = 1 + Math.floor(Math.random() * nMax * (0.6 + 0.4 * diff));
  const bombChance = (FNG.mode === 'arcade' ? .16 : .10) + diff * .1;
  const bonusChance = FNG.mode === 'arcade' ? .07 : 0;
  const kinds = Object.keys(FN_FRUITS);
  const schedule = [];
  for (let i = 0; i < n; i++) {
    const roll = Math.random();
    let kind = kinds[Math.floor(Math.random() * kinds.length)], bomb = false, bonus = false;
    if (roll < bombChance) bomb = true;
    else if (roll < bombChance + bonusChance) bonus = true;
    schedule.push({ kind, bomb, bonus, delay: i * (90 + Math.random() * 160) });
  }
  for (const s of schedule) {
    const myEpoch = FNG.epoch;
    setTimeout(() => {
      if (myEpoch !== FNG.epoch || FNG.state !== 'playing') return;
      fnSpawn(s.kind, s.bonus, s.bomb);
    }, s.delay);
  }
}

/* ---- Physics constants ---------------------------------------------- */
const FN_G = 1500;            // px/s² gravity in world units
const FN_TRAIL_LIFE = 220;    // ms a blade point stays alive
// LATENCY FIX: 320 → 180 world px/s. The old gate demanded an exaggerated
// flick; a quick natural swipe now registers instantly. Slow resting
// drags are still ignored (the gate itself is unchanged in behaviour).
const FN_MIN_SWIPE_SPEED = 180; // world px/s below which the blade won't cut

/* ---- Blade: segment↔circle intersection ----------------------------- */

/** Squared distance from point p to segment ab. */
function fnSegDist2(px, py, ax, ay, bx, by) {
  const dx = bx - ax, dy = by - ay;
  const len2 = dx * dx + dy * dy;
  let t = len2 ? ((px - ax) * dx + (py - ay) * dy) / len2 : 0;
  t = Math.max(0, Math.min(1, t));
  const cx = ax + t * dx, cy = ay + t * dy;
  return (px - cx) * (px - cx) + (py - cy) * (py - cy);
}

/** Did the swipe segment [a→b] cross the fruit's circle? */
function fnSegHitsCircle(ax, ay, bx, by, fx, fy, fr) {
  return fnSegDist2(fx, fy, ax, ay, bx, by) <= fr * fr;
}

/** LATENCY FIX: swept blade test. `prev` is the last processed sample,
 *  `cur` the newest one. Between two input samples (or between the last
 *  sample of one frame and the first of the next) a very fast swipe can
 *  jump clean over a fruit without either endpoint being near it — the
 *  classic "fast swipe misses" bug. We therefore ALSO advance the fruit
 *  along its own velocity for the sample interval and test the capsule
 *  swept by the segment: if the blade crossed the fruit's path at ANY
 *  instant between the two samples, it cuts. Pure math, no allocation,
 *  called only while state === 'playing'. */
function fnBladeCrossed(prev, cur, f, scale) {
  // blade segment in world units
  const ax = prev.x * scale, ay = prev.y * scale;
  const bx = cur.x * scale, by = cur.y * scale;
  // fruit position at both instants (it moved between the samples too)
  const dt = Math.max(0, (cur.t - prev.t)) / 1000;      // s
  const fx1 = f.x, fy1 = f.y;
  const fx2 = f.x + f.vx * dt, fy2 = f.y + f.vy * dt;
  // quick reject: bounding box of the swept blade vs the swept fruit
  const pad = f.r + 2;
  if (Math.max(ax, bx) + pad < Math.min(fx1, fx2)
   || Math.min(ax, bx) - pad > Math.max(fx1, fx2)
   || Math.max(ay, by) + pad < Math.min(fy1, fy2)
   || Math.min(ay, by) - pad > Math.max(fy1, fy2)) return false;
  // exact: distance from each fruit position to the blade segment
  return fnSegHitsCircle(ax, ay, bx, by, fx1, fy1, f.r)
      || fnSegHitsCircle(ax, ay, bx, by, fx2, fy2, f.r);
}

/** Process a fresh blade sample: test the newest segment against every
 *  live fruit. Speed-gated so a resting pointer never slices.
 *  LATENCY FIX:
 *  • the canvas→world scale is computed ONCE per sample (not per fruit),
 *  • the speed gate uses the same world-space distance as the collision
 *    test, so a fast diagonal/horizontal/vertical flick is never wrongly
 *    rejected by a CSS-vs-world unit mismatch,
 *  • the swept capsule test (fnBladeCrossed) catches fruits that a very
 *    fast swipe would otherwise jump clean over between two samples,
 *  • fruits are only iterated while actually playing; the loop is a plain
 *    reverse for-loop over a small array — no closures, no allocation. */
function fnBladeSample(x, y) {
  // LATENCY FIX: everything here runs on the performance.now() clock —
  // event.timeStamp is deliberately NOT used because some engines report
  // epoch-based stamps that would corrupt the speed gate and the trail
  // ageing. A sample that duplicates one already processed (pointerrawupdate
  // fires before pointermove's coalesced list) produces a zero-length
  // segment: it fails the speed gate and is inert, so no explicit dedupe
  // bookkeeping is needed in this hot path.
  const now = performance.now();
  const prev = FNG.trail[FNG.trail.length - 1];
  FNG.trail.push({ x, y, t: now });

  if (FNG.state !== 'playing') return;       // blade inert outside play
  if (!prev) return;
  const scale = FNG.w ? FN_WORLD_W / FNG.w : 1;   // canvas CSS px → world
  const ax = prev.x * scale, ay = prev.y * scale;
  const bx = x * scale, by = y * scale;
  const dist = Math.hypot(bx - ax, by - ay);
  FNG.swipeLen += dist;
  // speed gate in world px/s (same units as dist)
  const dtMs = Math.max(1, now - prev.t);
  if ((dist / dtMs) * 1000 < FN_MIN_SWIPE_SPEED) return;   // too slow → no cut

  const cur = { x, y, t: now };              // for the swept test
  for (let i = FNG.fruits.length - 1; i >= 0; i--) {
    const f = FNG.fruits[i];
    if (f.sliced) continue;
    if (fnBladeCrossed(prev, cur, f, scale)) {
      if (f.bomb) { fnBombHit(f); continue; }
      fnSliceFruit(f, ax, ay, bx, by);
    }
  }
}

/** Cut angle for a slice: perpendicular to travel looks natural. */
function fnSliceAngle(ax, ay, bx, by) {
  return Math.atan2(by - ay, bx - ax);
}

/** Slice ONE fruit: award points, split into halves, spray juice. */
function fnSliceFruit(f, ax, ay, bx, by) {
  f.sliced = true;
  const ang = fnSliceAngle(ax, ay, bx, by);
  const isBonus = f.bonus;
  const base = isBonus ? FN_BONUS.pts : (f.def ? f.def.pts : 1);

  // combo tracking: consecutive cuts inside a rolling 900ms blade window
  const nowMs = performance.now();
  if (nowMs - (FNG.lastCutT || 0) > 900) FNG.combo = 0;
  FNG.lastCutT = nowMs;
  if (!FNG.swipeHits.includes(f.id)) FNG.swipeHits.push(f.id);
  FNG.combo++;
  FNG.bestCombo = Math.max(FNG.bestCombo, FNG.combo);

  let pts = base;
  let crit = false;
  if (!isBonus && FNG.combo >= 3) {
    // every fruit after the 2nd in a single swipe is a combo slice
    pts = base + (FNG.combo - 1) * 2;
    crit = true;
  }

  FNG.score += pts;
  fnAddFloat(f.x, f.y - f.r * .4, '+' + pts, crit ? '#ffd166' : '#d0fff0', crit);
  fnSplitFruit(f, ang);
  fnJuice(f, ang);
  fnSfxSlice();                       // PHASE 2C: juicy varied slice
  if (crit) {
    // PHASE 2C: milestone sounds keyed to the existing combo counter.
    // Only the crossing (3/4/5+) plays its phrase; 6, 7, … re-trigger
    // just the top sparkle so long swipes escalate without noise.
    fnSfxCombo(FNG.combo >= 5 ? 5 : FNG.combo);
    if (FNG.combo > 5) fnSfxVoice(2349.32, 0.09, 0.08, 'sine');
    fnAddFloat(f.x, f.y - f.r - 18, FNG.combo + ' COMBO', '#ffd166', true);
  }
  if (isBonus) fnSfx('bonus');

  fnHudUpdate();
  // remove from live list
  const i = FNG.fruits.indexOf(f);
  if (i >= 0) FNG.fruits.splice(i, 1);
}

/** Split a fruit into two independently-falling halves along `ang`. */
function fnSplitFruit(f, ang) {
  const def = f.bomb ? null : (f.bonus ? FN_BONUS : f.def);
  if (!def) return;
  const perp = ang + Math.PI / 2;
  const push = 90 + Math.random() * 60;
  const shared = {
    def, kind: f.kind, bonus: f.bonus, r: f.r,
    life: 1600, born: performance.now(),
  };
  FNG.halves.push(
    Object.assign({}, shared, {
      x: f.x + Math.cos(perp) * f.r * .28, y: f.y + Math.sin(perp) * f.r * .28,
      vx: f.vx + Math.cos(perp) * push, vy: f.vy + Math.sin(perp) * push - 40,
      rot: f.rot, vr: f.vr + (Math.random() - .5) * 3, side: 0, ang,
    }),
    Object.assign({}, shared, {
      x: f.x - Math.cos(perp) * f.r * .28, y: f.y - Math.sin(perp) * f.r * .28,
      vx: f.vx - Math.cos(perp) * push, vy: f.vy - Math.sin(perp) * push - 40,
      rot: f.rot, vr: f.vr + (Math.random() - .5) * 3, side: 1, ang,
    })
  );
}

/** Juice droplets + splash ring at the cut point. */
function fnJuice(f, ang) {
  const def = f.bomb ? null : (f.bonus ? FN_BONUS : f.def);
  const color = def ? def.juice : '#ff4d6d';
  const n = FNG.reducedMotion ? 6 : 14;
  for (let i = 0; i < n; i++) {
    const a = ang + (Math.random() - .5) * 2.4;
    const sp = 120 + Math.random() * 320;
    FNG.particles.push({
      x: f.x, y: f.y,
      vx: Math.cos(a) * sp, vy: Math.sin(a) * sp - 60,
      r: 2 + Math.random() * 4.5, color,
      life: 500 + Math.random() * 450, born: performance.now(),
      grav: 1,
    });
  }
  FNG.splats.push({ x: f.x, y: f.y, r: f.r * .55, born: performance.now(), life: 420, color });
  if (FNG.particles.length > 400) FNG.particles.splice(0, FNG.particles.length - 400);
}

/** Bomb hit: explosion, shake, flash, penalty. */
function fnBombHit(f) {
  if (FNG.state !== 'playing') return;      // game already ending
  f.sliced = true;
  const i = FNG.fruits.indexOf(f);
  if (i >= 0) FNG.fruits.splice(i, 1);
  fnExplode(f.x, f.y);
  if (FNG.mode === 'classic') {
    FNG.lives--;
    fnHudUpdate();
    if (FNG.lives <= 0) { fnGameOver('bomb'); return; }
  } else {
    // Arcade: bombs cost 10 seconds and reset the combo chain.
    FNG.timeMs = Math.max(0, FNG.timeMs - 10000);
    FNG.combo = 0;
    fnHudUpdate();
  }
  fnAddFloat(f.x, f.y - f.r, FNG.mode === 'classic' ? '−1 ♥' : '−10s', '#ff6b81', true);
}

/** Explosion visuals: shockwave ring + sparks + smoke puffs. */
function fnExplode(x, y) {
  fnSfxBoom();                        // PHASE 2D: punchy explosion SFX
  FNG.shakeT = 420; FNG.shakeMag = 16;
  FNG.flashT = 130;
  const n = FNG.reducedMotion ? 12 : 34;
  for (let i = 0; i < n; i++) {
    const a = Math.random() * Math.PI * 2;
    const sp = 180 + Math.random() * 640;
    FNG.particles.push({
      x, y,
      vx: Math.cos(a) * sp, vy: Math.sin(a) * sp,
      r: 2 + Math.random() * 5,
      color: ['#ffd166', '#ff9838', '#ff6b81', '#fff'][Math.floor(Math.random() * 4)],
      life: 450 + Math.random() * 500, born: performance.now(), grav: .5,
    });
  }
  FNG.splats.push({ x, y, r: 30, born: performance.now(), life: 520, color: '#ffb23e', ring: true });
}

/** Floating score text. */
function fnAddFloat(x, y, text, color, big) {
  FNG.floats.push({ x, y, text, color, big: !!big, born: performance.now(), life: 900 });
  if (FNG.floats.length > 24) FNG.floats.shift();
}

/* ---- Rendering ------------------------------------------------------- */

/** Paint a fruit's exposed cut face: flesh gradient, rim, seeds or
 *  segments, centred on (0,0), radius r. `side` 0|1 flips the half. */
function fnPaintCut(ctx, def, r, side) {
  const cut = def.cut;
  ctx.save();
  // clip to a semicircle: the flat cut edge lies along +x after rotation
  ctx.beginPath();
  ctx.arc(0, 0, r, side ? 0 : Math.PI, side ? Math.PI : Math.PI * 2);
  ctx.closePath();
  ctx.clip();
  // flesh
  const g = ctx.createRadialGradient(0, 0, r * .05, 0, 0, r);
  for (const [c, stop] of cut.flesh) g.addColorStop(stop, c);
  ctx.fillStyle = g;
  ctx.fillRect(-r, -r, r * 2, r * 2);
  // segments (citrus): radial wedges
  if (cut.segments) {
    ctx.strokeStyle = 'rgba(255,255,255,.55)'; ctx.lineWidth = r * .07;
    for (let i = 0; i < 6; i++) {
      const a = (i / 6) * Math.PI;
      ctx.beginPath(); ctx.moveTo(0, 0);
      ctx.lineTo(Math.cos(a) * r, Math.sin(a) * r); ctx.stroke();
    }
  }
  // seeds
  if (cut.seeds && cut.seeds.n) {
    const n = cut.seeds.n;
    ctx.fillStyle = cut.seeds.color;
    for (let i = 0; i < n; i++) {
      let sx, sy;
      if (cut.seeds.ring) {          // kiwi: ring of seeds around the core
        const a = (i / n) * Math.PI * 2;
        sx = Math.cos(a) * r * .62; sy = Math.sin(a) * r * .62;
      } else {                        // melon/apple: two neat rows
        const row = i % 2 ? 1 : -1;
        const col = Math.floor(i / 2);
        sx = (-.45 + col * .3) * r; sy = row * r * .34;
      }
      ctx.beginPath();
      ctx.ellipse(sx, sy, r * .05, r * .085, 0, 0, Math.PI * 2); ctx.fill();
    }
  }
  // pale core
  if (cut.core) {
    ctx.fillStyle = cut.core;
    ctx.beginPath(); ctx.ellipse(0, 0, r * .3, r * .3, 0, 0, Math.PI * 2); ctx.fill();
  }
  ctx.restore();
  // rim stroke along the flat edge + arc
  ctx.strokeStyle = cut.rim; ctx.lineWidth = Math.max(2, r * .09);
  ctx.beginPath();
  ctx.arc(0, 0, r - ctx.lineWidth / 2, side ? 0 : Math.PI, side ? Math.PI : Math.PI * 2);
  ctx.stroke();
  ctx.beginPath(); ctx.moveTo(-r, 0); ctx.lineTo(r, 0); ctx.stroke();
}

/** Draw one whole fruit (rotated at its position). */
function fnDrawFruit(ctx, f) {
  ctx.save();
  ctx.translate(f.x, f.y);
  ctx.rotate(f.rot);
  if (f.bomb) { fnDrawBomb(ctx, f); ctx.restore(); return; }
  const def = f.bonus ? FN_BONUS : f.def;
  def.draw(ctx, f.r);
  ctx.restore();
}

/** Bomb: matte black sphere, lit fuse with sparks, hazard glint. */
function fnDrawBomb(ctx, f) {
  const r = f.r;
  ctx.save();
  // body
  const g = ctx.createRadialGradient(-r * .3, -r * .35, r * .1, 0, 0, r);
  g.addColorStop(0, '#4a4f5a'); g.addColorStop(.55, '#23272f'); g.addColorStop(1, '#101318');
  ctx.fillStyle = g;
  ctx.beginPath(); ctx.arc(0, 0, r, 0, Math.PI * 2); ctx.fill();
  // hazard glints
  ctx.fillStyle = 'rgba(255,209,102,.14)';
  ctx.beginPath(); ctx.arc(-r * .32, -r * .36, r * .22, 0, Math.PI * 2); ctx.fill();
  // skull mark
  ctx.fillStyle = 'rgba(255,255,255,.85)';
  ctx.font = `800 ${Math.round(r * .62)}px Sora`;
  ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  ctx.fillText('☠', 0, r * .06);
  // fuse
  ctx.strokeStyle = '#8a6a48'; ctx.lineWidth = r * .1; ctx.lineCap = 'round';
  ctx.beginPath();
  ctx.moveTo(r * .25, -r * .82);
  ctx.quadraticCurveTo(r * .55, -r * 1.15, r * .38, -r * 1.32);
  ctx.stroke();
  // spark: flickers with time
  const sp = (Math.sin(performance.now() / 60 + f.fuse) + 1) / 2;
  ctx.fillStyle = `rgba(255,${180 + sp * 60},80,${.7 + sp * .3})`;
  ctx.beginPath(); ctx.arc(r * .38, -r * 1.34, r * (.12 + sp * .1), 0, Math.PI * 2); ctx.fill();
  ctx.strokeStyle = 'rgba(255,209,102,.5)'; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.arc(r * .38, -r * 1.34, r * (.2 + sp * .16), 0, Math.PI * 2); ctx.stroke();
  ctx.restore();
}

/** Draw a falling half: rotate so the cut edge stays aligned with the
 *  original slice angle, then paint body-half + cut face. */
function fnDrawHalf(ctx, hf) {
  const def = hf.def;
  ctx.save();
  ctx.translate(hf.x, hf.y);
  ctx.rotate(hf.rot - hf.ang);
  // outer body silhouette clipped to this side
  ctx.save();
  ctx.beginPath();
  ctx.arc(0, 0, hf.r, hf.side ? 0 : Math.PI, hf.side ? Math.PI : Math.PI * 2);
  ctx.closePath();
  ctx.clip();
  def.draw(ctx, hf.r);
  ctx.restore();
  // exposed face
  fnPaintCut(ctx, def, hf.r, hf.side);
  ctx.restore();
}

/** Blade trail: tapered polyline with additive glow. */
function fnDrawTrail(ctx) {
  const now = performance.now();
  const pts = FNG.trail.filter(p => now - p.t < FN_TRAIL_LIFE);
  if (pts.length < 2) return;
  const scale = FNG.w / FN_WORLD_W;          // world → CSS px
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  for (let pass = 0; pass < 2; pass++) {
    ctx.beginPath();
    ctx.moveTo(pts[0].x * scale, pts[0].y * scale);
    for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x * scale, pts[i].y * scale);
    ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    if (pass === 0) {           // glow underlay
      ctx.strokeStyle = 'rgba(94,242,192,.28)';
      ctx.lineWidth = 13;
    } else {                    // bright core, tapering with age
      ctx.strokeStyle = 'rgba(230,255,248,.95)';
      ctx.lineWidth = 3.5;
    }
    ctx.stroke();
  }
  // tip gleam
  const tip = pts[pts.length - 1];
  ctx.fillStyle = 'rgba(255,255,255,.9)';
  ctx.beginPath(); ctx.arc(tip.x * scale, tip.y * scale, 3.2, 0, Math.PI * 2); ctx.fill();
  ctx.restore();
}

/** Backdrop: dojo-night gradient + vignette, drawn in world units. */
function fnDrawBackdrop(ctx) {
  const w = FN_WORLD_W, h = FNG.wh || 620;
  const g = ctx.createLinearGradient(0, 0, 0, h);
  g.addColorStop(0, '#0d1626'); g.addColorStop(.55, '#0b1322'); g.addColorStop(1, '#101c30');
  ctx.fillStyle = g; ctx.fillRect(0, 0, w, h);
  // faint radial glow behind the action
  const rg = ctx.createRadialGradient(w / 2, h * .42, 40, w / 2, h * .42, w * .62);
  rg.addColorStop(0, 'rgba(94,242,192,.05)'); rg.addColorStop(1, 'rgba(94,242,192,0)');
  ctx.fillStyle = rg; ctx.fillRect(0, 0, w, h);
}

/** Full scene paint. Physics has already stepped. */
function fnDraw(now) {
  const ctx = FNG.ctx; if (!ctx) return;
  const scale = FNG.w / FN_WORLD_W;          // world → CSS px
  ctx.setTransform(FNG.dpr * scale, 0, 0, FNG.dpr * scale, 0, 0);

  // screen shake: offset the world transform
  if (FNG.shakeT > 0) {
    const k = FNG.shakeT / 420 * FNG.shakeMag;
    ctx.translate((Math.random() - .5) * 2 * k, (Math.random() - .5) * 2 * k);
  }

  fnDrawBackdrop(ctx);

  // splash rings & stains (behind entities)
  for (const s of FNG.splats) {
    const t = (now - s.born) / s.life;
    if (t >= 1) continue;
    ctx.save();
    ctx.globalAlpha = (1 - t) * .5;
    if (s.ring) {
      ctx.strokeStyle = s.color; ctx.lineWidth = 3 * (1 - t) + 1;
      ctx.beginPath(); ctx.arc(s.x, s.y, s.r * (0.4 + t * 2.2), 0, Math.PI * 2); ctx.stroke();
    } else {
      ctx.fillStyle = s.color;
      ctx.beginPath(); ctx.arc(s.x, s.y, s.r * (1 + t * .3), 0, Math.PI * 2); ctx.fill();
    }
    ctx.restore();
  }

  // halves first (they fall behind fresh fruit)
  for (const hf of FNG.halves) fnDrawHalf(ctx, hf);
  // live fruit & bombs
  for (const f of FNG.fruits) fnDrawFruit(ctx, f);

  // particles
  for (const p of FNG.particles) {
    const t = (now - p.born) / p.life;
    if (t >= 1) continue;
    ctx.globalAlpha = 1 - t;
    ctx.fillStyle = p.color;
    ctx.beginPath(); ctx.arc(p.x, p.y, p.r * (1 - t * .5), 0, Math.PI * 2); ctx.fill();
  }
  ctx.globalAlpha = 1;

  // blade
  fnDrawTrail(ctx);

  // floating scores
  for (const fl of FNG.floats) {
    const t = (now - fl.born) / fl.life;
    if (t >= 1) continue;
    ctx.save();
    ctx.globalAlpha = 1 - t * t;
    ctx.fillStyle = fl.color;
    ctx.font = `800 ${fl.big ? 26 : 19}px "JetBrains Mono",monospace`;
    ctx.textAlign = 'center';
    ctx.shadowColor = 'rgba(0,0,0,.6)'; ctx.shadowBlur = 6;
    ctx.fillText(fl.text, fl.x, fl.y - t * 44);
    ctx.restore();
  }

  // bomb flash
  if (FNG.flashT > 0) {
    ctx.fillStyle = `rgba(255,255,255,${(FNG.flashT / 130) * .55})`;
    ctx.fillRect(-40, -40, FN_WORLD_W + 80, FNG.wh + 80);
  }

  // GET READY grace text
  if (FNG.state === 'playing' && FNG.graceMs > 0) {
    const a = Math.min(1, FNG.graceMs / 300);
    ctx.save();
    ctx.globalAlpha = a;
    ctx.fillStyle = '#eaf2ff';
    ctx.font = '800 34px Sora'; ctx.textAlign = 'center';
    ctx.shadowColor = 'rgba(0,0,0,.6)'; ctx.shadowBlur = 10;
    ctx.fillText('GET READY', FN_WORLD_W / 2, FNG.wh * .42);
    ctx.restore();
  }
}

/* ---- Simulation step (fixed world units, dt in seconds) ------------- */
function fnStep(dtMs, now) {
  const dt = dtMs / 1000;
  const h = FNG.wh || 620;                   // WORLD height, not CSS px

  // decay shake/flash/grace regardless of state
  if (FNG.shakeT > 0) FNG.shakeT = Math.max(0, FNG.shakeT - dtMs);
  if (FNG.flashT > 0) FNG.flashT = Math.max(0, FNG.flashT - dtMs);
  if (FNG.graceMs > 0) FNG.graceMs = Math.max(0, FNG.graceMs - dtMs);

  // trail ageing (also while paused → no ghost blade on resume)
  const cutoff = now - FN_TRAIL_LIFE;
  while (FNG.trail.length && FNG.trail[0].t < cutoff) FNG.trail.shift();

  // expire visuals
  FNG.halves = FNG.halves.filter(hf => now - hf.born < hf.life && hf.y < h + 260);
  // (h here is the world height computed above)
  FNG.particles = FNG.particles.filter(p => now - p.born < p.life);
  FNG.splats = FNG.splats.filter(s => now - s.born < s.life);
  FNG.floats = FNG.floats.filter(f => now - f.born < f.life);

  if (FNG.state !== 'playing') return;

  // PHASE 1: difficulty clock — advances ONLY while playing, so pausing
  // freezes the ramp exactly like the arcade countdown.
  FNG.elapsedMs += dtMs;

  // arcade clock
  if (FNG.mode === 'arcade') {
    FNG.timeMs -= dtMs;
    if (FNG.timeMs <= 0) { FNG.timeMs = 0; fnHudUpdate(); fnGameOver('time'); return; }
  }

  // wave scheduler
  FNG.spawnTimer -= dtMs;
  if (FNG.spawnTimer <= 0 && FNG.graceMs <= 0) {
    fnSpawnWave();
    FNG.spawnTimer = fnWaveGapMs();
  }

  // fruit physics + miss detection
  for (let i = FNG.fruits.length - 1; i >= 0; i--) {
    const f = FNG.fruits[i];
    f.vy += FN_G * dt;
    f.x += f.vx * dt;
    f.y += f.vy * dt;
    f.rot += f.vr * dt;
    // missed: fell back below the bottom after being on screen
    if (!f.missed && f.vy > 0 && f.y - f.r > h + 8) {
      f.missed = true;
      FNG.fruits.splice(i, 1);
      if (!f.bomb) fnMiss(f);
    }
  }

  // halves & particles physics
  for (const hf of FNG.halves) {
    hf.vy += FN_G * dt;
    hf.x += hf.vx * dt; hf.y += hf.vy * dt; hf.rot += hf.vr * dt;
  }
  for (const p of FNG.particles) {
    p.vy += FN_G * (p.grav ?? 1) * dt;
    p.x += p.vx * dt; p.y += p.vy * dt;
  }
}

/** A fruit escaped: Classic loses a life; Arcade just breaks the chain. */
function fnMiss(f) {
  if (FNG.state !== 'playing') return;      // game already ending
  if (FNG.mode === 'classic') {
    fnSfxLifeLost();                 // PHASE 2D: descending penalty
  } else {
    fnSfx('miss');                   // Arcade: soft chain-break blip
  }
  fnAddFloat(Math.min(FN_WORLD_W - 60, Math.max(60, f.x)), (FNG.wh || 620) - 70, 'MISS', '#ff6b81', false);
  if (FNG.mode === 'classic') {
    FNG.lives--;
    FNG.combo = 0;
    fnHudUpdate();
    if (FNG.lives <= 0) { fnGameOver('misses'); return; }
  } else {
    FNG.combo = 0;
  }
}

/* ---- Frame loop ------------------------------------------------------ */
function fnFrame(now) {
  if (FNG.epoch !== FNG._loopEpoch) { FNG.raf = 0; return; }
  FNG.raf = requestAnimationFrame(fnFrame);
  const dt = Math.min(50, now - (FNG.lastT || now));
  FNG.lastT = now;

  fnStep(dt, now);
  fnDraw(now);

  // live HUD ticks that change every frame (timer)
  if (FNG.state === 'playing' && FNG.mode === 'arcade') fnHudTime();
}

/* ---- HUD ------------------------------------------------------------- */
function fnHudUpdate() {
  const sc = fn$('fnScore'); if (sc) sc.textContent = String(FNG.score);
  const lv = fn$('fnLives');
  if (lv) {
    let html = '';
    for (let i = 0; i < 3; i++) html += `<i class="${i < FNG.lives ? '' : 'off'}" aria-hidden="true">♥</i>`;
    lv.innerHTML = html;
  }
  const best = fn$('fnBestV');
  if (best) best.textContent = String(Math.max(FNG.best[FNG.mode], FNG.score));
  fnHudTime();
}

function fnHudTime() {
  const t = fn$('fnTime');
  if (!t) return;
  const s = Math.ceil(FNG.timeMs / 1000);
  t.textContent = String(s);
  const stat = t.closest('.fn-stat');
  if (stat) stat.classList.toggle('warn', s <= 10);
}

/* ---- Game flow ------------------------------------------------------- */
function fnStart(mode) {
  FNG.mode = mode || FNG.mode;
  FNG.score = 0;
  FNG.lives = 3;
  FNG.combo = 0; FNG.bestCombo = 0;
  FNG.timeMs = FNG.mode === 'arcade' ? 60000 : 0;
  FNG.graceMs = 1200;                 // GET READY beat before wave 1
  FNG.elapsedMs = 0;                  // PHASE 1: difficulty clock restarts
  FNG.spawnTimer = 0;
  FNG.waveCount = 0;
  FNG.fruits = []; FNG.halves = []; FNG.particles = [];
  FNG.splats = []; FNG.floats = []; FNG.swipeHits = [];
  FNG.trail = []; FNG.swipeLen = 0; FNG.lastCutT = 0;
  FNG.shakeT = 0; FNG.flashT = 0;
  FNG.state = 'playing';
  fnBgmStart();                       // PHASE 2B: music with every run
  fnSfxCountdown();                   // PHASE 2D: 3-2-1 ticks + GO cue
  const panel = fn$('fnPanel');
  if (panel) {
    panel.classList.toggle('fn-mode-arcade', FNG.mode === 'arcade');
    panel.classList.toggle('fn-mode-classic', FNG.mode === 'classic');
  }
  fnHudUpdate();
  fnOverlayHide();
  fnSfxUi();                          // PHASE 2D: subtle start click
}

function fnTogglePause() {
  if (FNG.state === 'playing') {
    FNG.state = 'paused';
    fnBgmSetPlaying(false);           // PHASE 2B: duck music on pause
    fnOverlayShow({
      title: 'Paused',
      sub: 'Take a breath. Your fruit will wait.',
      btn: 'Resume', kind: 'resume',
    });
  } else if (FNG.state === 'paused') {
    FNG.state = 'playing';
    FNG.lastT = performance.now();
    fnBgmSetPlaying(true);            // PHASE 2B: music returns with play
    fnOverlayHide();
  }
  fnPauseButton();
  fnSfxUi();                          // PHASE 2D: subtle pause/resume click
}

function fnPauseButton() {
  const b = fn$('fnPauseBtn');
  if (b) b.textContent = FNG.state === 'paused' ? 'Resume' : 'Pause';
}

function fnGameOver(reason) {
  FNG.state = 'over';
  FNG.trail = [];
  fnBgmStop();                        // PHASE 2B: fade out, clean teardown
  // persist best
  const key = FNG.mode;
  const prevBest = FNG.best[key];     // PHASE 2D: capture BEFORE update
  if (FNG.score > FNG.best[key]) {
    FNG.best[key] = FNG.score;
    localStorage.setItem(key === 'arcade' ? 'fnBestArcade' : 'fnBestClassic', String(FNG.score));
  }
  fnSfxOver();                        // PHASE 2D: arcade ending (once)
  if (FNG.score > 0 && FNG.score > prevBest) {
    fnSfxRecord();                    // PHASE 2D: genuine new best only
  }
  const isRecord = FNG.score > 0 && FNG.score > prevBest;
  const reasonTxt = reason === 'bomb' ? 'A bomb ended the run.'
    : reason === 'misses' ? 'Too many fruits got away.'
    : "Time's up!";
  fnOverlayShow({
    title: 'Game Over',
    sub: `${reasonTxt} Best combo this run: <b>${FNG.bestCombo}×</b>.`,
    stats: [
      ['score', FNG.score, true],
      ['best', FNG.best[key], false],
      ['combo', FNG.bestCombo + '×', false],
    ],
    row: [
      { label: `Play ${FNG.mode === 'arcade' ? 'Arcade' : 'Classic'} Again`, act: 'again', primary: true },
      { label: 'Change Mode', act: 'menu', ghost: true },
    ],
    best: isRecord ? '★ NEW BEST SCORE ★' : '',
  });
  fnHudUpdate();
}

/* ---- Overlay --------------------------------------------------------- */
function fnOverlayShow(o) {
  const ov = fn$('fnOverlay'); if (!ov) return;
  const title = fn$('fnOvTitle'), sub = fn$('fnOvSub');
  const stats = fn$('fnOvStats'), row = fn$('fnOvRow'), best = fn$('fnOvBest');
  if (title) title.textContent = o.title || '';
  if (sub) sub.innerHTML = o.sub || '';
  if (stats) {
    if (o.stats && o.stats.length) {
      stats.classList.remove('hidden');
      stats.innerHTML = o.stats.map(([k, v, hl]) =>
        `<div class="fn-ov-stat${hl ? ' hl' : ''}"><span class="k">${k}</span><span class="v">${v}</span></div>`).join('');
    } else { stats.classList.add('hidden'); stats.innerHTML = ''; }
  }
  if (row) {
    row.innerHTML = (o.row || [{ label: o.btn || 'OK', act: o.kind || 'start', primary: !o.ghost }])
      .map(b => `<button type="button" class="fn-ov-btn${b.ghost ? ' ghost' : ''}" data-act="${b.act}">${b.label}</button>`)
      .join('');
  }
  if (best) best.textContent = o.best || '';
  ov.classList.remove('hidden');
}

function fnOverlayHide() {
  const ov = fn$('fnOverlay');
  if (ov) ov.classList.add('hidden');
}

/* ---- Pointer input ----------------------------------------------------
   PHASE 3B: latency-tuned blade input.
   • The canvas rect is CACHED (refreshed on every resize and at every
     swipe start) instead of read per pointermove — getBoundingClientRect
     forces a layout flush, so the hot path must never call it.
   • pointermove consumes e.getCoalescedEvents() when available: fast
     swipes are sampled at full device rate (sub-frame), so no fruit
     between two frame samples escapes the segment tests. The coalesced
     list ends with the event's own position, so iterating only that
     list never double-processes a point.
   • ONE pointer owns the blade at a time (first down wins until it
     lifts): interleaving a second finger into the active polyline would
     forge phantom segments — and phantom slices. Mouse and touch are
     identical here; Pointer Events unify them.
   • No throttling, debouncing or rAF-deferral anywhere in this path —
     every sample is processed the moment it arrives. `touch-action:none`
     on .fn-stage stops page scroll while swiping; the move listener is
     passive (it never calls preventDefault).
   Slicing semantics are UNCHANGED: complete-path tracking, per-segment
   circle intersection, the FN_MIN_SWIPE_SPEED gate, per-fruit id-gated
   combo detection — bombs included.                                   */
function fnCacheRect() {
  // Canvas-local origin for pointer mapping. Called on resize and at the
  // start of every swipe — never per move (gBCR forces a layout flush).
  FNG._rect = FNG.canvas ? FNG.canvas.getBoundingClientRect() : null;
}

function fnPointerPos(e) {
  // CSS px → canvas-local px (canvas CSS size == backing store / dpr).
  // Cached rect; the live-read fallback only covers a missing cache.
  const r = FNG._rect || (FNG.canvas ? FNG.canvas.getBoundingClientRect() : null);
  return { x: e.clientX - (r ? r.left : 0), y: e.clientY - (r ? r.top : 0) };
}

/** End-of-swipe bookkeeping: reset per-swipe combo counters. */
function fnSwipeEnd() {
  if (FNG.swipeLen > 24) FNG.swipeHits = [];
  FNG.swipeLen = 0;
}

/* ---- Resize -----------------------------------------------------------
   PHASE 3: one source of truth for the canvas box. The stage's available
   area is measured (NEVER the canvas itself — its own box is whatever the
   last fit produced, so measuring it would freeze the size forever). The
   canvas letterboxes to at most 16:10 inside that area, centred by the
   flex .fn-stage-inner; fnResize() then writes the exact px box inline,
   sizes the DPR backing store to match, and syncs FNG.w/h (CSS px) plus
   FNG.wh (world height) that every painter already reads.             */
function fnResize() {
  const c = FNG.canvas;
  if (!c || !FNG.ctx) return;
  // 1. Available area: the stage-inner box (flex-centres the canvas).
  const inner = c.parentElement;                       // .fn-stage-inner
  const availW = Math.max(200, (inner && inner.clientWidth) || 0);
  const availH = Math.max(150, (inner && inner.clientHeight) || 0);
  if (availW < 2 || availH < 2) { fnDraw(performance.now()); return; }
  // 2. Fit. FULLSCREEN FIX: in fullscreen/immersive the canvas fills the
  //    ENTIRE available area — no 16:10 letterboxing, no black margins.
  //    Only the inline stage keeps the classic ≤16:10 aspect.
  const filling = document.body.classList.contains('fn-fs')
    || document.body.classList.contains('ft-immersive');
  let cssW, cssH;
  if (filling) {
    cssW = availW; cssH = availH;
  } else {
    cssW = Math.min(availW, availH * 1.6);
    cssH = cssW / 1.6;
  }
  // No-op guard: skip identical refits (fullscreenchange fires twice etc.)
  const key = (filling ? 'F' : 'I') + Math.round(cssW) + 'x' + Math.round(cssH);
  if (key === FNG._fitKey) return;
  FNG._fitKey = key;
  // 3. CSS box (inline style beats the stylesheet, so this works in both
  //    the inline and fullscreen/immersive layouts without extra rules).
  c.style.width = cssW + 'px';
  c.style.height = cssH + 'px';
  // 4. Backing store at device resolution, capped for perf.
  FNG.dpr = Math.min(window.devicePixelRatio || 1, 2);
  c.width = Math.max(1, Math.round(cssW * FNG.dpr));
  c.height = Math.max(1, Math.round(cssH * FNG.dpr));
  // 5. State every painter/input path reads: CSS px size + world height.
  FNG.w = cssW; FNG.h = cssH;
  // world height tracks the REAL aspect so trajectories fill the field
  // (tall portrait fullscreen now gets a taller world, not clipped arcs)
  FNG.wh = Math.max(420, Math.round(FN_WORLD_W * (cssH / cssW)));
  fnCacheRect();                     // PHASE 3B: pointer origin follows the box
  if (FNG.state !== 'playing') fnDraw(performance.now());
}

/* ---- View ------------------------------------------------------------- */
function renderFnView() {
  document.getElementById('viewRoot').innerHTML = `
  <div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Fruit Ninja</div>
    </div>

    <section class="ft-panel ft-game fn-wide" id="fnPanel" aria-label="Fruit Ninja arcade game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${(typeof ICONS !== 'undefined' && ICONS.funtime) || ''}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Fruit Ninja</div>
          <div class="ft-blurb">Slice everything that flies — just not the bombs. Combos pay extra.</div>
        </div>
      </header>

      <div class="fn-hud" role="status" aria-live="off">
        <span class="fn-stat score"><span class="k">score</span><span class="v" id="fnScore">0</span></span>
        <span class="fn-stat lives"><span class="k">lives</span><span class="hearts" id="fnLives" aria-label="3 lives left"></span></span>
        <span class="fn-stat time hidden-mode"><span class="k">time</span><span class="v" id="fnTime">60</span></span>
        <span class="fn-stat best"><span class="k">best</span><span class="v" id="fnBestV">0</span></span>
      </div>

      <div class="fn-stage" id="fnStageWrap">
        <div class="fn-stage-inner">
          <canvas id="fnCanvas" aria-label="Fruit Ninja play field"
                  role="img">Your browser does not support the canvas element.</canvas>
          <div class="fn-overlay" id="fnOverlay" role="dialog" aria-modal="false" aria-label="Game status">
            <div class="fn-ov-inner">
              <div class="fn-ov-title" id="fnOvTitle">Fruit Ninja</div>
              <div class="fn-ov-sub" id="fnOvSub"></div>
              <div class="fn-modes" id="fnModes">
                <button type="button" class="fn-mode" data-mode="classic">
                  <span class="ic" aria-hidden="true">🍉</span>
                  <span class="t">Classic</span>
                  <span class="d">3 lives. Missed fruit and bombs cost a heart.</span>
                </button>
                <button type="button" class="fn-mode" data-mode="arcade">
                  <span class="ic" aria-hidden="true">⏱</span>
                  <span class="t">Arcade</span>
                  <span class="d">60 seconds of chaos. Bonus fruit, big combos.</span>
                </button>
              </div>
              <div class="fn-ov-stats hidden" id="fnOvStats"></div>
              <div class="fn-ov-row" id="fnOvRow"></div>
              <div class="fn-ov-best" id="fnOvBest"></div>
            </div>
          </div>
        </div>
      </div>

      <div class="fn-controls" role="group" aria-label="Game controls">
        <button type="button" class="ft-btn" id="fnPauseBtn">Pause</button>
        <button type="button" class="ft-btn" id="fnRestartBtn">Restart</button>
        <button type="button" class="ft-btn si-sound" id="fnSoundBtn" aria-pressed="true">
          <span class="si-snd-ico" aria-hidden="true">♪</span><span id="fnSoundLbl">sound on</span>
        </button>
        <label class="fn-vol" id="fnVolMusic" title="Background music volume">
          <span aria-hidden="true">♫</span><input type="range" id="fnBgmVol" min="0" max="100" step="1" value="34" aria-label="Background music volume">
        </label>
        <label class="fn-vol" id="fnVolSfx" title="Sound effects volume">
          <span aria-hidden="true">✦</span><input type="range" id="fnSfxVol" min="0" max="100" step="1" value="100" aria-label="Sound effects volume">
        </label>
        <button type="button" class="ft-btn" id="fnFsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn si-exit" id="fnExit">Back to Games</button>
      </div>

      <div class="fn-help">
        <span>drag to slice</span><span>space pause</span><span>R restart</span><span>F fullscreen</span>
      </div>
    </section>
  </div></div>`;

  FNG.epoch++;
  const myEpoch = FNG.epoch;
  FNG._loopEpoch = myEpoch;
  FNG.cleanup = [];
  FNG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const fnOnLocal = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    FNG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };

  // Exactly one loop per mount: cancel any straggler from a previous view.
  if (FNG.raf) { cancelAnimationFrame(FNG.raf); FNG.raf = 0; }

  FNG.canvas = fn$('fnCanvas');
  FNG.ctx = FNG.canvas.getContext('2d');
  FNG.state = 'menu';
  FNG.fruits = []; FNG.halves = []; FNG.particles = [];
  FNG.splats = []; FNG.floats = []; FNG.trail = [];

  fnResize();
  fnCacheRect();                    // PHASE 3B: pointer origin valid from the first swipe

  // Menu overlay on first mount
  fnOverlayShow({
    title: 'Fruit Ninja',
    sub: 'Drag or swipe to slice. Chain cuts in one swipe for <b>combo bonuses</b> — and whatever you do, <b>don\'t cut the bombs</b>.',
    row: [],
  });
  fn$('fnModes').classList.remove('hidden');

  // ---- mode buttons ----
  fnOnLocal(fn$('fnModes'), 'click', e => {
    const b = e.target.closest('.fn-mode');
    if (!b) return;
    if (FNG.state === 'playing' || FNG.state === 'paused') return;
    fnSfxUi();                        // PHASE 2D: subtle selection click
    fnStart(b.dataset.mode);
    fn$('fnModes').classList.add('hidden');
  });

  // ---- overlay action buttons (start/resume/again/menu) ----
  fnOnLocal(fn$('fnOvRow'), 'click', e => {
    const b = e.target.closest('.fn-ov-btn');
    if (!b) return;
    const act = b.dataset.act;
    if (act === 'start' || act === 'resume') fnTogglePause();
    else if (act === 'again') { fnStart(FNG.mode); fn$('fnModes').classList.add('hidden'); }
    else if (act === 'menu') {
      FNG.state = 'menu';
      fnBgmStop();                    // PHASE 2B: no music on the menu
      fnSfxUi();                      // PHASE 2D: subtle menu click
      fnOverlayShow({
        title: 'Fruit Ninja',
        sub: 'Pick your poison.',
        row: [],
      });
      fn$('fnModes').classList.remove('hidden');
    }
  });

  // ---- blade input on the canvas -------------------------------------
  // PHASE 3B: one pointer owns the blade (first down wins); moves are
  // sampled at full rate incl. coalesced sub-frame events; the rect is
  // cached per swipe, never read per move.
  const stage = fn$('fnStageWrap');
  let bladePointer = null;              // pointerId currently owning the blade
  fnOnLocal(stage, 'pointerdown', e => {
    if (myEpoch !== FNG.epoch) return;
    fnAudioUnlock();                    // PHASE 2A: first gesture opens the ctx
    if (e.target.closest('.fn-overlay')) return;   // overlay owns its clicks
    if (bladePointer !== null && e.pointerId !== bladePointer) return; // second finger must not splice into the active swipe
    e.preventDefault();
    try { stage.setPointerCapture(e.pointerId); } catch { /* fine */ }
    bladePointer = e.pointerId;
    fnCacheRect();                      // fresh origin for this swipe
    FNG.trail = [];
    FNG.swipeLen = 0;
    const p = fnPointerPos(e);
    fnBladeSample(p.x, p.y);
  });
  fnOnLocal(stage, 'pointermove', e => {
    if (myEpoch !== FNG.epoch) return;
    if (!FNG.trail.length) return;                 // only while swiping
    if (bladePointer !== null && e.pointerId !== bladePointer) return;
    // Full-rate sampling: the coalesced list ends with the event's own
    // position, so iterating ONLY it processes each point exactly once.
    const evs = e.getCoalescedEvents ? e.getCoalescedEvents() : null;
    if (evs && evs.length) {
      for (let i = 0; i < evs.length; i++) {
        const p = fnPointerPos(evs[i]);
        fnBladeSample(p.x, p.y);
      }
    } else {
      const p = fnPointerPos(e);
      fnBladeSample(p.x, p.y);
    }
  }, { passive: true });
  // LATENCY FIX: raw pointerrawupdate gives sub-frame blade samples on
  // engines that support it (Chrome/Edge); it fires BEFORE the coalesced
  // pointermove, so very fast flicks are tested at the true device rate.
  // Guarded — Safari/Firefox simply keep the pointermove path above.
  if ('onpointerrawupdate' in window) {
    fnOnLocal(stage, 'pointerrawupdate', e => {
      if (myEpoch !== FNG.epoch) return;
      if (bladePointer === null || e.pointerId !== bladePointer) return;
      if (!FNG.trail.length) return;
      const p = fnPointerPos(e);
      fnBladeSample(p.x, p.y);
    }, { passive: true });
  }
  const endSwipe = e => {
    if (myEpoch !== FNG.epoch) return;
    if (bladePointer !== null && e.pointerId !== bladePointer) return;
    bladePointer = null;
    if (!FNG.trail.length) return;
    fnSwipeEnd();
    FNG.trail = [];
  };
  fnOnLocal(stage, 'pointerup', endSwipe);
  fnOnLocal(stage, 'pointercancel', endSwipe);
  fnOnLocal(stage, 'lostpointercapture', endSwipe);
  fnOnLocal(stage, 'contextmenu', e => e.preventDefault());

  // ---- keyboard ----
  const GAME_KEYS = new Set([' ', 'p', 'P', 'r', 'R', 'f', 'F']);
  fnOnLocal(document, 'keydown', e => {
    if (myEpoch !== FNG.epoch) return;
    fnAudioUnlock();                  // PHASE 2A: keyboard also unlocks audio
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (e.key === ' ' || e.key === 'p' || e.key === 'P') {
      if (FNG.state === 'playing' || FNG.state === 'paused') { fnTogglePause(); e.preventDefault(); }
      return;
    }
    if (e.key === 'r' || e.key === 'R') {
      if (FNG.state !== 'menu') { fnStart(FNG.mode); fn$('fnModes').classList.add('hidden'); }
      e.preventDefault(); return;
    }
    if (e.key === 'f' || e.key === 'F') { fnToggleFullscreen(); e.preventDefault(); return; }
    if (GAME_KEYS.has(e.key)) e.preventDefault();
  });

  // Auto-pause when the tab loses focus mid-game.
  fnOnLocal(window, 'blur', () => { if (FNG.state === 'playing') fnTogglePause(); });
  fnOnLocal(document, 'visibilitychange', () => {
    if (document.hidden && FNG.state === 'playing') fnTogglePause();
  });

  // ---- persistent controls ----
  fnOnLocal(fn$('fnPauseBtn'), 'click', () => {
    if (FNG.state === 'playing' || FNG.state === 'paused') fnTogglePause();
  });
  fnOnLocal(fn$('fnRestartBtn'), 'click', () => {
    if (FNG.state !== 'menu') { fnStart(FNG.mode); fn$('fnModes').classList.add('hidden'); }
  });
  fnOnLocal(fn$('fnSoundBtn'), 'click', () => fnSoundToggle());

  // ---- PHASE 2E: independent volume sliders ----
  fnAudioLoadPrefs();                 // restore saved levels before first sound
  fnAudioSyncControls();              // sliders reflect loaded/mute state
  const bgmSlider = fn$('fnBgmVol');
  if (bgmSlider) {
    fnOnLocal(bgmSlider, 'input', () => {
      fnBgmSetVolume(parseInt(bgmSlider.value, 10) / 100);
      try { localStorage.setItem('fnVolMusic', String(FNA.bgm.volume)); } catch { /* ignore */ }
    });
  }
  const sfxSlider = fn$('fnSfxVol');
  if (sfxSlider) {
    fnOnLocal(sfxSlider, 'input', () => {
      fnSfxSetVolume(parseInt(sfxSlider.value, 10) / 100);
      try { localStorage.setItem('fnVolSfx', String(FNA.sfx.volume)); } catch { /* ignore */ }
    });
  }
  fnOnLocal(fn$('fnFsBtn'), 'click', () => fnToggleFullscreen());
  if (typeof ftFsSupported === 'function' && !ftFsSupported()) {
    const b = fn$('fnFsBtn'); if (b) b.style.display = 'none';
  }
  // FULLSCREEN FIX: reflect genuine fullscreen state onto the body class
  // and the button label through the shared helper, then refit the canvas.
  // Previously the button path requested native fullscreen but NOTHING
  // applied the .fn-fs layout class, so the canvas stayed letterboxed
  // inside a black screen. ftFsReflect also clears every game's fs class
  // on exit, so ESC / button exits restore the inline layout cleanly.
  const fnFsChanged = () => {
    if (typeof ftFsReflect === 'function') ftFsReflect('fn', 'fn-fs', 'fnFsBtn');
    fnResize();
  };
  fnOnLocal(document, 'fullscreenchange', fnFsChanged);
  fnOnLocal(document, 'webkitfullscreenchange', fnFsChanged);
  fnOnLocal(fn$('fnExit'), 'click', () => { location.hash = '#funtime'; });

  // ---- resize + loop ------------------------------------------------
  // PHASE 3: window resize AND a ResizeObserver on the stage. Entering or
  // leaving fullscreen/immersive only flips body classes — no window
  // 'resize' event fires — so the observer is what refits the canvas at
  // the exact moment the layout changes.
  fnOnLocal(window, 'resize', () => fnResize());
  try {
    const ro = new ResizeObserver(() => fnResize());
    ro.observe(stage);
    if (stage.firstElementChild) ro.observe(stage.firstElementChild); // .fn-stage-inner
    FNG.cleanup.push(() => ro.disconnect());
  } catch { /* ancient engine: window-resize path still covers it */ }

  FNG.lastT = performance.now();
  FNG.raf = requestAnimationFrame(fnFrame);

  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, dashboard chrome hidden while playing.
  if (typeof ftSessionAfterMount === 'function') ftSessionAfterMount('fn', 'fn-fs');
  if (typeof ftSessionBind === 'function') ftSessionBind(fnOnLocal, () => fnResize());
  // PHASE 3: the session/fullscreen classes land AFTER mount (they flip
  // body classes), so refit once more once the real layout settles. The
  // button-path fullscreenchange listener above now handles state
  // reflection + refit, so no duplicate resize-only listener is needed.
  requestAnimationFrame(() => { if (myEpoch === FNG.epoch) fnResize(); });
}

/* ---- Fullscreen ------------------------------------------------------- */
function fnToggleFullscreen() {
  if (typeof ftToggleFullscreen === 'function') {
    ftToggleFullscreen('fn', 'fn-fs', null);
  }
}

/* ---- Teardown --------------------------------------------------------- */
function fnTeardown() {
  FNG.epoch++;                     // invalidates the running loop
  FNG._loopEpoch = -1;             // no session: any late frame self-kills
  if (FNG.raf) { cancelAnimationFrame(FNG.raf); FNG.raf = 0; }
  for (const off of FNG.cleanup) { try { off(); } catch { /* gone */ } }
  FNG.cleanup = [];
  FNG.state = 'menu';
  FNG.fruits = []; FNG.halves = []; FNG.particles = [];
  FNG.splats = []; FNG.floats = []; FNG.trail = [];
  FNG.ctx = null; FNG.canvas = null;
  FNG.w = 0; FNG.h = 0;
  FNG._fitKey = '';                 // PHASE 3: next mount must refit, not skip
  FNG._rect = null;                 // PHASE 3B: drop the cached pointer origin
  fnAudioDispose();
  ftExitFullscreenQuiet();
  document.body.classList.remove('fn-fs');
  ftSessionRestore();               // restore dashboard chrome
}

/* Expose for the console shell (router + tests). Mirrors chess-game.js. */
window.renderFnView = renderFnView;
window.fnTeardown = fnTeardown;
window.FNG = FNG;
