//! Fruit Ninja integration checks: the new asset is served, the console
//! HTML loads it in the right order, and the SPA registers the route.

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
async fn fruitninja_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/fruitninja-game.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains("renderFnView"), "view entrypoint must exist");
    assert!(body.contains("fnTeardown"), "teardown must exist");
    assert!(body.contains("requestAnimationFrame"), "must use RAF");
    // Real blade mechanics, not click-to-score.
    assert!(
        body.contains("fnSegHitsCircle"),
        "segment collision must exist"
    );
    assert!(body.contains("fnSplitFruit"), "halves must split");
    assert!(
        body.contains("FN_MIN_SWIPE_SPEED"),
        "swipe speed gate must exist"
    );
}

#[tokio::test]
async fn console_html_loads_fruitninja_before_the_app() {
    let app = app().await;
    let (_code, html) = get(&app, "/console").await;
    let game = html.find("/console/fruitninja-game.js").expect("game tag");
    let appjs = html.find("/console/console-app.js").expect("app tag");
    assert!(game < appjs, "game UI must load before the router");
}

#[tokio::test]
async fn console_app_registers_the_fruitninja_route_and_card() {
    let app = app().await;
    let (_code, js) = get(&app, "/console/console-app.js").await;
    assert!(js.contains("funtime/fn"), "sub-route must be registered");
    assert!(js.contains("ftCardFn"), "arcade card must exist");
    assert!(js.contains("renderFnView"), "view must be dispatched");
    assert!(js.contains("fnTeardown"), "teardown must be wired");
    // The shared fullscreen/session plumbing must know about this game.
    assert!(
        js.contains("'fn-fs'"),
        "fullscreen class must be registered"
    );
    assert!(
        js.contains("fn: 'FNG'"),
        "epoch map must include the game state"
    );
    // The existing games must all still be there.
    for needle in [
        "ftCardT3", "ftCardSi", "ftCardHt", "ftCardCr", "ftCardCh", "ftCardTm", "ftCardSo",
    ] {
        assert!(js.contains(needle), "{needle} must survive");
    }
}
