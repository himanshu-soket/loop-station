//! Hack the Terminal integration checks: the served assets keep the
//! presentation contract (mission header band, no ASCII separators in the
//! brief, semantic line classes, docked control bar) and the responsive
//! layout rules that make the terminal fill the viewport and scroll
//! internally — the two bugs this UI fixes.

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

fn css() -> String {
    include_str!("../src/assets/console.html").to_string()
}

/// The four missions, their verbs and their scoring bases are untouched.
#[test]
fn mission_contract_is_intact() {
    let src = js();
    for needle in [
        "id: 'access', title: 'Mission 1 · Terminal Access'",
        "id: 'sequence', title: 'Mission 2 · Crack the Sequence'",
        "id: 'decode', title: 'Mission 3 · Decode the Signal'",
        "id: 'firewall', title: 'Mission 4 · Firewall Bypass'",
        "const HT_TIMES = [150, 180, 210, 240];",
        "const HT_BASE = [300, 400, 500, 600];",
    ] {
        assert!(src.contains(needle), "{needle} must survive");
    }
    // Answer verbs still dispatch through htSubmit.
    assert!(src.contains("case 'unlock': case 'submit': case 'decode': case 'bypass'"));
    // Hint still halves the reward.
    assert!(src.contains("HT.hintPenalty = (HT.hintPenalty || 0) + 0.5;"));
    // Timer still ticks every second and freezes while paused.
    assert!(src.contains("if (HT.paused) return;               // frozen while paused"));
}

/// Presentation: the mission brief uses one labelled section rule instead of
/// duplicated ASCII ══ bars, and the panel carries a mission header band.
#[test]
fn mission_presentation_contract() {
    let src = js();
    // No ASCII separator lines are printed anywhere.
    assert!(
        !src.contains("'══════════════════════════════════════', 'l-hd'"),
        "ASCII separator rules must be gone"
    );
    // The mission header band exists and is kept in sync with game state.
    assert!(
        src.contains("id=\"htMhead\""),
        "mission header band must exist"
    );
    assert!(
        src.contains("function htMissionHead()"),
        "header painter must exist"
    );
    assert!(src.contains("LOCK ${idx + 1}/${HT_MISSIONS.length}"));
    // Intel clues render through the styled .ht-clue group.
    assert!(src.matches("class=\"ht-clue\"").count() >= 4);
    // Simulation transcript lines are marked separately from user commands.
    assert!(
        src.contains("'l-sim'"),
        "simulation lines need their own class"
    );
    // The victory summary labels the real maximum (base × 1.4 time bonus).
    assert!(src.contains("HT_BASE.reduce((a, b) => a + b, 0) * 1.4"));
}

/// Layout: the terminal fills the viewport and owns the only scroll — the
/// prompt row can never be pushed off-screen by a long log.
#[test]
fn responsive_layout_contract() {
    let styles = css();
    // .ht-term is a flex column so .ht-out's flex:1 actually applies.
    assert!(
        styles.contains(".ht-term{\n  position:relative;border:1px solid var(--line);border-radius:12px;\n  background:#05070a;overflow:hidden;\n  box-shadow:inset 0 0 60px rgba(94,242,192,.03), 0 0 0 1px rgba(94,242,192,.04);\n  display:flex;flex-direction:column;min-height:0;\n  flex:1 1 auto;\n}"),
        ".ht-term must be a flex column that can absorb leftover height"
    );
    // In fullscreen the term keeps flexing and the log stays the scroller.
    assert!(styles.contains("body.ht-fs .ht-term,\nbody.ft-playing.ft-immersive .ht-term{flex:1 1 auto;min-height:0;display:flex;flex-direction:column}"));
    // Windowed play: the panel fills the viewport but never exceeds it.
    // PHASE 1 · scroll escape hatch: inline play scrolls inside the panel
    // (overflow-y:auto) instead of clipping, so nothing is trapped below
    // the fold when the terminal outgrows the viewport.
    assert!(styles.contains("max-height:100vh;\n  overflow-y:auto;\n  overscroll-behavior:contain;\n}"));
    // The old fixed-height clamp is gone (it ignored the free space).
    assert!(
        !styles.contains("height:clamp(240px,38vh,380px)"),
        "fixed output height clamp must be replaced by the flex chain"
    );
    // Controls are docked inside the terminal frame.
    assert!(styles.contains(".ht-controls{\n  display:flex;gap:8px;flex-wrap:wrap;justify-content:center;align-items:center;\n  flex:none;"));
}

/// Line semantics exist for every message kind, so colour is never the only
/// distinguishing cue.
#[test]
fn terminal_line_semantics_exist() {
    let styles = css();
    for cls in [
        "l-cmd", "l-sys", "l-sim", "l-err", "l-warn", "l-cy", "l-vio", "l-ok", "l-hd",
    ] {
        assert!(
            styles.contains(&format!(".ht-out .{cls}")),
            "{cls} must be styled"
        );
    }
    // Commands carry a prompt glyph; clues carry a marker and indent.
    assert!(styles.contains(".ht-out .l-cmd::before{content:'❯ '"));
    assert!(styles.contains(".ht-out .ht-clue::before"));
    // Reduced motion still collapses the decorative animation.
    assert!(styles.contains("@media (prefers-reduced-motion:reduce){\n  .ht-card-term .cur,.ht-cur,.ht-net i{animation:none}"));
}

/// The view still mounts, tears down and registers with the shared session
/// plumbing — no lifecycle regressions.
#[tokio::test]
async fn hack_terminal_route_and_lifecycle_wired() {
    let app = app().await;
    let (code, body) = get(&app, "/console/console-app.js").await;
    assert_eq!(code, StatusCode::OK);
    for needle in [
        "function renderHtView()",
        "function htTeardown()",
        "ftSessionAfterMount('ht', 'ht-fs')",
        "'ht-fs'",
        "ht: 'HT'",
    ] {
        assert!(body.contains(needle), "{needle} must exist");
    }
    // The dead launch mount was removed from the markup.
    assert!(
        !body.contains("htOvLaunch"),
        "dead launch mount must be gone"
    );
    // Every control is still present.
    for id in [
        "htStartBtn",
        "htPauseBtn",
        "htRestartBtn",
        "htSoundBtn",
        "htFsBtn",
        "htExit",
    ] {
        assert!(body.contains(&format!("id=\"{id}\"")), "{id} must exist");
    }
}

/// Other games' cards and the shared selectors survive untouched.
#[tokio::test]
async fn other_games_survive() {
    let app = app().await;
    let (_code, js_body) = get(&app, "/console/console-app.js").await;
    for needle in [
        "ftCardT3", "ftCardSi", "ftCardHt", "ftCardCr", "ftCardCh", "ftCardTm", "ftCardSo",
        "ftCardFn", "ftCardLu", "ftCardSn",
    ] {
        assert!(js_body.contains(needle), "{needle} must survive");
    }
}
