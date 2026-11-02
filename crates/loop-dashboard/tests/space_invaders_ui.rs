//! Space Invaders UI integration checks: the redesign contract is present
//! in the served assets — responsive fullscreen canvas sizing, the HUD
//! (score/high/wave/lives-as-glyphs), settings inside the pause/ready
//! overlay, keyboard + touch steering (pads AND direct drag), audio prefs
//! persistence, and a teardown that never leaks listeners or layout state.
//!
//! Mirrors the style of cyber_rush_ui.rs / fruitninja_ui.rs: the asset is
//! fetched through the real router so the served bytes are what ships.

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
    include_str!("../src/assets/console-app.js").to_string()
}

fn html() -> String {
    include_str!("../src/assets/console.html").to_string()
}

#[tokio::test]
async fn si_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/console-app.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains("function renderSiView"), "view entrypoint must exist");
    assert!(body.contains("requestAnimationFrame"), "must use RAF");
}

/// STEP 1 · fullscreen architecture: real Fullscreen API on the panel,
/// height-fitted canvas sizing measured against the viewport, and the CSS
/// immersive fallback sharing one layout class.
#[test]
fn si_fullscreen_architecture_contract() {
    let src = js();
    // The panel is the fullscreen element; the class drives its layout.
    assert!(src.contains("'si-fs'"), "fullscreen class must be registered");
    assert!(src.contains("function siIsFullscreen"), "state probe must exist");
    // Height-fitted sizing uses the VIEWPORT budget, not panel.clientHeight.
    assert!(
        src.contains("window.innerHeight - chromeH"),
        "canvas fit must measure the viewport budget"
    );
    // Logical coordinates stay fixed: the transform scales once.
    assert!(
        src.contains("cssW * dpr / SI.W"),
        "logical->device transform must scale from SI.W/SI.H"
    );
    // The immersive fallback is honestly reported (not native).
    assert!(
        src.contains("ft-immersive"),
        "CSS immersive fallback must be wired"
    );
    // Fullscreen exit restores the dashboard shell.
    assert!(
        src.contains("document.body.classList.remove('si-fs')"),
        "teardown must drop the fs class"
    );
}

/// STEP 2 · professional HUD: score/high/wave stats plus lives rendered as
/// ship glyphs with an accessible count, all behind the change cache.
#[test]
fn si_hud_contract() {
    let src = js();
    for id in ["siScore", "siHigh", "siWave", "siLives"] {
        assert!(src.contains(&format!("id=\"{id}\"")), "HUD stat {id} must exist");
    }
    // Lives render as repeated glyphs, capped at MAX_LIVES.
    assert!(
        src.contains("'▲'.repeat(Math.max(0, Math.min(g.lives, SI.MAX_LIVES)))"),
        "lives must render as capped ship glyphs"
    );
    // The exact number stays available to screen readers.
    assert!(
        src.contains("livesEl.setAttribute('aria-label'"),
        "lives element must carry an aria-label"
    );
    assert!(
        src.contains("SI_HUD_CACHE[id] !== v"),
        "HUD must be cached against rewrites"
    );
    let css = html();
    assert!(css.contains(".si-lives-icons"), "lives glyph styling must exist");
    assert!(css.contains(".si-stat.si-wave"), "wave badge styling must exist");
}

/// STEP 3 · settings live in the ready/paused overlay, hidden on game over;
/// music/effects volumes and mutes stay independent and persisted.
#[test]
fn si_overlay_settings_contract() {
    let src = js();
    // Settings block is inside the overlay markup.
    assert!(src.contains("id=\"siOvAudio\""), "overlay settings block must exist");
    assert!(
        src.contains("si-ov-settings"),
        "settings block must use its class"
    );
    // Shown while idle, hidden on game over.
    assert!(
        src.contains("set.hidden = (kind === 'over')"),
        "settings must hide on game over"
    );
    // Independent music/effects controls still exist exactly once each.
    assert_eq!(src.matches("id=\"siMusicVol\"").count(), 1);
    assert_eq!(src.matches("id=\"siSfxVol\"").count(), 1);
    assert_eq!(src.matches("id=\"siMusicMute\"").count(), 1);
    assert_eq!(src.matches("id=\"siSfxMute\"").count(), 1);
    // Prefs persist under the versioned key.
    assert!(src.contains("SI_AUDIO_KEY = 'si.audio.v1'"));
    let css = html();
    assert!(
        css.contains(".si-ov-settings[hidden]{display:none !important}"),
        "hidden settings must never render"
    );
}

/// STEP 4 · desktop keyboard: arrows/WASD steering, P pause, R restart,
/// F fullscreen, Space consumed (never scrolls), and text fields are never
/// hijacked.
#[test]
fn si_keyboard_contract() {
    let src = js();
    // The gameplay key set exists and covers the documented bindings.
    assert!(src.contains("const GAME_KEYS = new Set("));
    for k in ["'ArrowLeft'", "'ArrowRight'", "'a'", "'d'", "'p'", "'r'", "'f'"] {
        assert!(src.contains(k), "key {k} must be bound");
    }
    // Keys never reach the game while a field is focused.
    assert!(
        src.contains("t.tagName === 'INPUT'"),
        "text-field guard must exist"
    );
    // Space must not scroll the page during play.
    assert!(
        src.contains("e.preventDefault();"),
        "gameplay keys must preventDefault"
    );
    // Held keys die on blur/tab-switch (no stuck movement).
    assert!(
        src.contains("siOn(window, 'blur',"),
        "blur must clear held input"
    );
}

/// STEP 5 · touch: pointer pads with held state, PLUS direct drag steering
/// on the canvas at the same capped speed as keys (no teleporting).
#[test]
fn si_touch_contract() {
    let src = js();
    // Pads still exist and paint their held state.
    assert!(src.contains("id=\"siLeft\""), "left pad must exist");
    assert!(src.contains("id=\"siRight\""), "right pad must exist");
    assert!(
        src.contains("el.classList.add('held')"),
        "held pads must stay painted"
    );
    // Direct drag steering maps pointer x into logical units.
    assert!(src.contains("function siSteerTo"), "drag steering must exist");
    assert!(
        src.contains("input.steerX - g.player.w / 2"),
        "drag must target the ship's centre"
    );
    // Drag moves at the SAME capped speed as keys — never teleports.
    assert!(
        src.contains("Math.max(-maxMove, Math.min(maxMove, delta))"),
        "drag steering must be speed-capped"
    );
    // Pointer capture keeps the drag alive; release always stops it.
    assert!(
        src.contains("setPointerCapture"),
        "pointer capture must be used"
    );
    assert!(
        src.contains("SIG.touch.steerX = null"),
        "release must stop following"
    );
    // Mouse pointers are excluded (keys/pads remain the desktop controls).
    assert!(
        src.contains("e.pointerType === 'mouse'"),
        "mouse drags must not steer"
    );
}

/// STEP 6 · visual polish: overlay scrolls when short, pads keep their
/// active state, and every breakpoint still compacts the HUD.
#[test]
fn si_css_polish_contract() {
    let css = html();
    // Overlay content can scroll on short viewports.
    assert!(
        css.contains("overflow-y:auto;overscroll-behavior:contain"),
        "overlay inner must scroll when clipped"
    );
    // Held pads stay visually active.
    assert!(
        css.contains(".si-pad:active,.si-pad.held"),
        "held pad styling must exist"
    );
    // Phone + landscape breakpoints still compact the HUD.
    assert!(css.contains("@media (max-width:560px)"));
    assert!(css.contains("@media (max-height:520px) and (orientation:landscape)"));
}

/// STEP 7 · regression: the shared session plumbing and the other games'
/// cards survive untouched; teardown removes every listener and resets the
/// touch model including the new steerX field.
#[test]
fn si_session_and_teardown_contract() {
    let src = js();
    // Session lifecycle wiring.
    assert!(src.contains("ftSessionAfterMount('si', 'si-fs')"));
    assert!(src.contains("ftSessionBind(siOn, () => siResize())"));
    // Teardown resets the full touch model.
    assert!(
        src.contains("steerX: null"),
        "touch reset must include steerX"
    );
    // Every listener goes through the registrar that teardown drains.
    assert!(src.contains("SIG.cleanup.push("));
    // The other arcade cards must all survive.
    for needle in [
        "ftCardT3", "ftCardHt", "ftCardCr", "ftCardCh", "ftCardTm", "ftCardSo", "ftCardFn",
        "ftCardLu", "ftCardSn",
    ] {
        assert!(src.contains(needle), "{needle} must survive");
    }
}
