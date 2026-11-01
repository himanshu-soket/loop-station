//! Solitaire integration checks: the two new assets are served, the console
//! HTML loads them in the right order, and the SPA registers the route.

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
async fn solitaire_engine_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/solitaire-engine.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(
        body.contains("SolitaireEngine"),
        "engine must expose its API"
    );
    assert!(body.contains("createDeck"));
    assert!(body.contains("moveTableauToFoundation"));
}

#[tokio::test]
async fn solitaire_game_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/solitaire-game.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains("renderSoView"));
    assert!(body.contains("soTeardown"));
}

#[tokio::test]
async fn console_html_loads_solitaire_scripts_before_the_app() {
    let app = app().await;
    let (_code, html) = get(&app, "/console").await;
    let engine = html
        .find("/console/solitaire-engine.js")
        .expect("engine tag");
    let game = html.find("/console/solitaire-game.js").expect("game tag");
    let appjs = html.find("/console/console-app.js").expect("app tag");
    assert!(engine < game, "engine must load before the game UI");
    assert!(game < appjs, "game UI must load before the router");
}

#[tokio::test]
async fn console_app_registers_the_solitaire_route_and_card() {
    let app = app().await;
    let (_code, js) = get(&app, "/console/console-app.js").await;
    assert!(js.contains("funtime/so"), "sub-route must be registered");
    assert!(js.contains("ftCardSo"), "arcade card must exist");
    assert!(js.contains("renderSoView"), "view must be dispatched");
    assert!(js.contains("soTeardown"), "teardown must be wired");
    // The existing games must all still be there.
    for needle in [
        "ftCardT3", "ftCardSi", "ftCardHt", "ftCardCr", "ftCardCh", "ftCardTm",
    ] {
        assert!(js.contains(needle), "{needle} must survive");
    }
}
