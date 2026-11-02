//! Snakes & Ladders integration checks: both new assets are served, the
//! console HTML loads them in the right order, and the SPA registers the
//! route + card.

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

#[tokio::test]
async fn snakes_engine_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/snakes-engine.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(
        body.contains("SnakesEngine"),
        "engine must expose its API global"
    );
    // The full ruleset must be present.
    for needle in [
        "createGame",
        "rollDice",
        "previewMove",
        "applyRoll",
        "advanceTurn",
        "resolve",
        "aiShouldRoll",
        "LADDERS",
        "SNAKES",
        "CELL",
    ] {
        assert!(body.contains(needle), "{needle} must exist");
    }
    // Deterministic board configuration: 7 ladders and 7 snakes.
    assert_eq!(
        body.matches("[4, 14]").count(),
        1,
        "ladder 4->14 defined once"
    );
    assert_eq!(
        body.matches("[98, 78]").count(),
        1,
        "snake 98->78 defined once"
    );
}

#[tokio::test]
async fn snakes_game_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/snakes-game.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains("renderSnView"), "view entrypoint must exist");
    assert!(body.contains("snTeardown"), "teardown must exist");
    assert!(
        body.contains("SnakesEngine."),
        "UI must go through the engine"
    );
    // The required game modes.
    for mode in ["pvc", "'2p'", "'3p'", "'4p'", "'2vc'"] {
        assert!(body.contains(mode), "mode {mode} must exist");
    }
    // Real animation mechanics, not teleporting tokens.
    assert!(body.contains("requestAnimationFrame"), "must use RAF");
    assert!(
        body.contains("snAnimateJump"),
        "snake/ladder jump animation must exist"
    );
    assert!(
        body.contains("snWalkStep"),
        "square-by-square walk must exist"
    );
}

#[tokio::test]
async fn console_html_loads_snakes_scripts_before_the_app() {
    let app = app().await;
    let (_code, html) = get(&app, "/console").await;
    let engine = html.find("/console/snakes-engine.js").expect("engine tag");
    let game = html.find("/console/snakes-game.js").expect("game tag");
    let appjs = html.find("/console/console-app.js").expect("app tag");
    assert!(engine < game, "engine must load before the game UI");
    assert!(game < appjs, "game UI must load before the router");
}

#[tokio::test]
async fn console_app_registers_the_snakes_route_and_card() {
    let app = app().await;
    let (_code, js) = get(&app, "/console/console-app.js").await;
    assert!(js.contains("funtime/sn"), "sub-route must be registered");
    assert!(js.contains("ftCardSn"), "arcade card must exist");
    assert!(js.contains("renderSnView"), "view must be dispatched");
    assert!(js.contains("snTeardown"), "teardown must be wired");
    // The shared fullscreen/session plumbing must know about this game.
    assert!(
        js.contains("'sn-fs'"),
        "fullscreen class must be registered"
    );
    assert!(
        js.contains("sn: 'SNG'"),
        "epoch map must include the game state"
    );
    // The existing games must all still be there.
    for needle in [
        "ftCardT3", "ftCardSi", "ftCardHt", "ftCardCr", "ftCardCh", "ftCardTm", "ftCardSo",
        "ftCardFn", "ftCardLu",
    ] {
        assert!(js.contains(needle), "{needle} must survive");
    }
}
