//! Timber Man Ultra UI integration checks: the arcade-upgrade contract is
//! present in the served assets — fullscreen letterboxed canvas with a
//! fixed logical playfield, the overlay HUD (score top-left, time+level
//! top-center, best + settings gear top-right), no permanent bottom
//! control bar, keyboard + touch-half chopping with hold-to-repeat,
//! settings overlay that pauses gameplay, audio prefs persistence, and a
//! teardown that never leaks listeners or layout state.
//!
//! Mirrors space_invaders_ui.rs / fruitninja_ui.rs: the asset is fetched
//! through the real router so the served bytes are what ships.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use loop_dashboard::api::{build_app, AppOptions};

async fn app() -> axum::Router {
    let (router, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        support_store: None,
        console_state: None,
        auth_store: None,
    })
    .unwrap();
    router
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let code = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (code, String::from_utf8_lossy(&bytes).into_owned())
}

fn js() -> String {
    include_str!("../src/assets/timberman-game.js").to_string()
}

/// The game script is served as JavaScript through the real router.
#[tokio::test]
async fn tm_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/timberman-game.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(
        body.contains("function renderTmView"),
        "view entrypoint must exist"
    );
    assert!(body.contains("requestAnimationFrame"), "must use RAF");
    // The console shell registers the sub-route and teardown.
    let shell = include_str!("../src/assets/console-app.js").to_string();
    assert!(shell.contains("funtime/tm"), "sub-route must be registered");
    assert!(shell.contains("tmTeardown()"), "teardown must be wired");
}

/// PHASE 1 · fullscreen architecture: the stage owns the panel, the canvas
/// letterboxes at the fixed TM_VW x TM_VH logical aspect with one scale
/// transform (no sprite distortion at any viewport), and the shared FTSESS
/// session controller drives native fullscreen + the honest CSS fallback.
#[test]
fn tm_fullscreen_architecture_contract() {
    let src = js();
    // Fixed logical playfield keeps gameplay math viewport-independent.
    assert!(src.contains("const TM_VW = 480, TM_VH = 640"));
    // One letterbox fit: largest scale that fits BOTH axes, centred.
    assert!(
        src.contains("Math.min(fitW / TM_VW, fitH / TM_VH)"),
        "canvas fit must letterbox both axes"
    );
    // One transform maps logical units onto the device pixels.
    assert!(
        src.contains("cssW * TMG.dpr / TM_VW"),
        "logical->device transform must scale from TM_VW/TM_VH"
    );
    // Session lifecycle wiring (native fullscreen + Esc ownership).
    assert!(src.contains("ftSessionAfterMount('tm', 'tm-fs')"));
    assert!(src.contains("ftSessionBind(tmOnLocal, () => tmResize())"));
    // The immersive fallback is honestly reported (not native).
    assert!(src.contains("ft-immersive"));
    // Fullscreen exit restores the dashboard shell.
    assert!(
        src.contains("document.body.classList.remove('tm-fs')"),
        "teardown must drop the fs class"
    );
    // Resize refits and repaints even while idle.
    assert!(src.contains("if (TMG.state !== 'playing') tmDraw("));
}

/// PHASE 2 · professional HUD: three clusters pinned inside the stage —
/// score top-left, time (+ progress bar) and level chip top-center, best +
/// fullscreen icon + settings gear top-right. Translucent panels, smooth
/// score bump, warn/crit timer feedback. No permanent bottom control bar.
#[test]
fn tm_hud_contract() {
    let src = js();
    for id in [
        "tmScore", "tmTime", "tmLvlChip", "tmBestV", "tmTimeBar", "tmTimeFill",
        "tmGearBtn", "tmFsBtn",
    ] {
        assert!(
            src.contains(&format!("id=\"{id}\"")),
            "HUD element {id} must exist"
        );
    }
    // The three HUD clusters exist as separate grid areas.
    for cls in ["tm-hud-l", "tm-hud-c", "tm-hud-r"] {
        assert!(src.contains(cls), "HUD cluster {cls} must exist");
    }
    // The HUD lives INSIDE the stage (overlay, not a row above it).
    assert!(
        src.contains(".tm-hud{position:absolute;inset:0"),
        "HUD must overlay the stage"
    );
    // Smooth score update: bump pulse only on change.
    assert!(
        src.contains("st.classList.add('bump')"),
        "score bump animation must exist"
    );
    // Timer urgency thresholds.
    assert!(src.contains("pct <= 40 && pct > 18"), "warn band must exist");
    assert!(src.contains("pct <= 18"), "crit band must exist");
    // NO permanent bottom toolbar: the old rows must be gone.
    for gone in [".tm-controls{", ".tm-touch{", "tmPauseBtn", "tmRestartBtn", "tmSoundBtn"] {
        assert!(
            !src.contains(gone),
            "legacy bottom-bar element {gone} must be removed"
        );
    }
}

/// PHASE 3/4 · environment + character: layered forest background
/// (sky/hills/silhouettes/motes/ground), textured trunk, leafed branches,
/// lumberjack with axe swing, side slide, death tumble and hit feedback.
#[test]
fn tm_environment_and_character_contract() {
    let src = js();
    // Environment layers exist and draw back-to-front.
    for f in [
        "function tmDrawSky",
        "function tmDrawHills",
        "function tmDrawBgForest",
        "function tmDrawMotes",
        "function tmDrawGround",
    ] {
        assert!(src.contains(f), "{f} must exist");
    }
    // Restrained particle pool (bounded, not per-frame spawns).
    assert!(src.contains("const TM_MOTES = []"));
    // Trunk texture: bark fissures + highlight stripe.
    assert!(src.contains("Bark texture"), "trunk texture must exist");
    // Branches carry foliage clusters.
    assert!(src.contains("leaves"), "branch foliage must exist");
    // Character: chop swing, side slide, death animation, chips.
    assert!(src.contains("TMG.sideAnim"), "side-slide state must exist");
    assert!(src.contains("TMG.deathT"), "death clock must exist");
    assert!(src.contains("function tmSpawnChips"), "wood chips must exist");
    // Hit feedback: red flash + screen shake.
    assert!(src.contains("flashRed"), "hit flash must exist");
    assert!(src.contains("shake"), "screen shake must exist");
}

/// PHASE 5 · gameplay preserved: chop sides, branch collision at head
/// level, fairness invariant (never two branches in a row on one side),
/// level curve with shrinking time refund, timer starts on first chop,
/// best-score persistence, game-over logic for both death causes.
#[test]
fn tm_gameplay_contract() {
    let src = js();
    // Collision checks the segment at the lumberjack's head level.
    assert!(
        src.contains("const headSeg = TMG.trunk[1]"),
        "collision must test the head-level segment"
    );
    // Fairness invariant appears in BOTH generators (initial + per-chop).
    assert!(
        src.matches("if (nb === lastBranch) nb = nb === 'left' ? 'right' : 'left';").count() >= 1,
        "per-chop fairness flip must exist"
    );
    assert!(
        src.matches("if (b === lastBranch) b = b === 'left' ? 'right' : 'left';").count() >= 1,
        "initial-trunk fairness flip must exist"
    );
    // Five levels with rising thresholds and shrinking refunds.
    assert!(src.contains("const TM_LEVELS = ["));
    assert!(src.contains("name: 'INSANE'"), "all five levels must exist");
    // Timer starts on the first successful chop.
    assert!(
        src.contains("if (!TMG.timerStarted) { TMG.timerStarted = true;"),
        "countdown must begin on the first chop"
    );
    // Best score persists under the established key.
    assert!(src.contains("localStorage.getItem('tmBest')"));
    assert!(src.contains("localStorage.setItem('tmBest'"));
    // Game over distinguishes branch vs time.
    assert!(src.contains("tmGameOver('branch')"));
    assert!(src.contains("tmGameOver('time')"));
    // Score + time refund on every successful chop.
    assert!(src.contains("TMG.score++"));
    assert!(src.contains("tmChopBonus()"));
}

/// PHASE 6 · controls & settings: arrows/WASD chop, P pause, R restart,
/// F fullscreen, text fields never hijacked; pointer halves chop with
/// hold-to-repeat and clean release; the settings overlay pauses gameplay
/// and offers Resume/Restart/Fullscreen/audio/Exit.
#[test]
fn tm_controls_and_settings_contract() {
    let src = js();
    // Keyboard bindings.
    for k in ["'ArrowLeft'", "'ArrowRight'", "'a'", "'d'", "'p'", "'r'", "'f'"] {
        assert!(src.contains(k), "key {k} must be bound");
    }
    // Keys never reach the game while a field is focused.
    assert!(src.contains("t.tagName === 'INPUT'"));
    // Pointer halves: any pointer type chops the half it lands in.
    assert!(
        src.contains("chopFromEvent"),
        "pointer-side resolution must exist"
    );
    // Hold-to-repeat for touch, stopped by up/cancel/leave/blur.
    assert!(src.contains("holdTimer = setInterval"));
    for ev in ["'pointerup'", "'pointercancel'", "'pointerleave'"] {
        assert!(src.contains(ev), "release handler {ev} must exist");
    }
    // Settings overlay exists with the required actions.
    for id in [
        "tmSettings", "tmSetResume", "tmSetRestart", "tmSetFs", "tmSetExit",
        "tmAuMusicV", "tmAuSfxV", "tmAuMusicT", "tmAuSfxT",
    ] {
        assert!(
            src.contains(&format!("id=\"{id}\"")),
            "settings element {id} must exist"
        );
    }
    // Opening settings PAUSES gameplay; closing resumes only if it paused.
    assert!(
        src.contains("setPausedByOpen = TMG.state === 'playing'"),
        "settings must auto-pause"
    );
    // Audio prefs persist under the versioned key.
    assert!(src.contains("TM_AUDIO_KEY = 'tm-audio-v1'"));
    // Held input dies on blur/tab-switch.
    assert!(src.contains("tmOnLocal(window, 'blur'"));
}

/// PHASE 7 · game states: ready/playing/paused/over overlays with the
/// final score + best + Restart + Back to Games on game over, and the
/// level-up toast.
#[test]
fn tm_game_states_contract() {
    let src = js();
    // Overlay handles all four states.
    for kind in ["'ready'", "'paused'", "'over'"] {
        assert!(src.contains(kind), "overlay state {kind} must exist");
    }
    // Game-over screen shows final score AND best.
    assert!(src.contains("id=\"tmRsScore\""));
    assert!(src.contains("id=\"tmRsBest\""));
    // Restart + Back buttons on game over.
    assert!(src.contains("Chop Again"));
    assert!(src.contains("id=\"tmOvExit\""));
    // Level-up toast.
    assert!(src.contains("LEVEL ${TMG.level + 1}"));
    // Touch-zone hints show on coarse pointers in the ready state.
    assert!(src.contains("@media (pointer:coarse)"));
}

/// PHASE 8 · responsiveness: phone + landscape breakpoints compact the
/// HUD, the stage never scrolls the page, and orientation changes refit.
#[test]
fn tm_responsiveness_contract() {
    let src = js();
    // Compact breakpoints for phones and short landscape viewports.
    assert!(src.contains("@media (max-width:560px)"));
    assert!(src.contains("@media (max-height:520px) and (orientation:landscape)"));
    // The game owns gesture scrolling (no accidental page scroll).
    assert!(src.contains("touch-action:none"));
    // Safe-area insets respected in fullscreen (notched phones).
    assert!(src.contains("env(safe-area-inset-left"));
    // Visual balance: HUD padding scales with the viewport.
    assert!(src.contains("clamp(8px,1.6vh,14px)"));
}

/// REGRESSION · teardown drains every listener, kills the RAF chain,
/// closes the audio context exactly once, clears the mote pool and
/// restores the dashboard chrome. The other games' cards survive.
#[test]
fn tm_teardown_and_regression_contract() {
    let src = js();
    // Every listener goes through the registrar that teardown drains.
    assert!(src.contains("TMG.cleanup.push("));
    // Teardown resets the full state incl. the particle pools.
    assert!(src.contains("TM_MOTES.length = 0"));
    // Audio context closed ONCE.
    assert!(src.contains("tmAudioDispose()"));
    // The epoch guard invalidates stale continuations.
    assert!(src.contains("TMG.epoch++"));
    // The other arcade cards must all survive in the shell.
    let shell = include_str!("../src/assets/console-app.js").to_string();
    for needle in [
        "ftCardT3", "ftCardSi", "ftCardHt", "ftCardCr", "ftCardCh", "ftCardSo", "ftCardFn",
        "ftCardLu", "ftCardSn",
    ] {
        assert!(shell.contains(needle), "{needle} must survive");
    }
}
