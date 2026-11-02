//! Static web assets: the dashboard SPAs served from the same binary.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

/// Embedded trace-observatory app.
const INDEX_HTML: &str = include_str!("assets/index.html");

/// Embedded system-monitor app (self-contained: no external requests).
const SYSTEM_HTML: &str = include_str!("assets/system.html");

/// Embedded console app (playground + traces + requests + analytics).
const CONSOLE_HTML: &str = include_str!("assets/console.html");

/// Console application script.
const CONSOLE_JS: &str = include_str!("assets/console-app.js");

/// Shared shell styles (auth pages + workspace chrome).
const SHELL_CSS: &str = include_str!("assets/shell.css");

/// Pretraining data-pipeline architecture canvas.
///
/// Served verbatim as a self-contained page and embedded by the console
/// SPA (`#pretrain/architecture`) in an iframe, so its styles and scripts
/// stay fully isolated from the console shell. Like the other static
/// assets it contains no user data; the console route that mounts it
/// remains behind the session guard.
const PIPELINE_ARCHITECTURE_HTML: &str = include_str!("assets/pipeline_architecture.html");

/// SFT data-quality pipeline architecture canvas (`sftqc`).
///
/// Same pattern as the pretraining canvas: served verbatim as a
/// self-contained page and embedded by the console SPA
/// (`#sft/architecture`) in an iframe, keeping its styles and scripts
/// isolated from the console shell. Boxes map to real functions in
/// `~/soket/sftqc`; live run counts come from `/api/pipelines/sft`.
const SFT_ARCHITECTURE_HTML: &str = include_str!("assets/sft_architecture.html");

/// EKA SFT data-pipeline board (`assets/sft_board.html`).
///
/// The six-stage Collect → Standardize → Clean corpus → Curate gold →
/// Audit + handoff → Train + feedback board with its Run/Reset animation,
/// rejected-row repair arrows and stage detail panel. Same pattern as the
/// other pipeline pages: served verbatim as a self-contained page and
/// embedded by the console SPA (`#sft/board`) in an iframe, so its styles
/// and scripts stay isolated from the console shell.
const SFT_BOARD_HTML: &str = include_str!("assets/sft_board.html");

/// EKA SFT data-pipeline map (`assets/sft_pipeline_map.html`).
///
/// The original `eka-sft-board-2.html` — dark pipeline board with the
/// stage strip, run controls ("now running" panel + log), six columns of
/// stage cards, dashed cross-stage flows, reject lane, repair routes and
/// the tap drawer — copied in byte-for-byte and served verbatim as a
/// self-contained page. Same pattern as the other pipeline pages: mounted
/// behind the session guard and opened from the SFT DataTrove workspace's
/// "Open pipeline map" entry in a new browser tab.
const SFT_PIPELINE_MAP_HTML: &str = include_str!("assets/sft_pipeline_map.html");

/// EKA checkpoints-by-level report (`assets/eka_checkpoints.html`).
///
/// The original "EKA Checkpoints by Level" webpage — verdict panel, the
/// level-wise and per-checkpoint accuracy tables with their tinted cells,
/// reference-model rows and the collapsible older-checkpoint tables —
/// copied in byte-for-byte and served verbatim as a self-contained page.
/// Mounted inside the console SPA at `#eval/model` (Model Evaluation) in an
/// iframe that fills the workspace, so its styles stay fully isolated from
/// the console shell. Like the other static assets it contains no user data.
const EKA_CHECKPOINTS_HTML: &str = include_str!("assets/eka_checkpoints.html");

/// Pretraining pipeline architecture report.
///
/// The navigable engineering reference for the pretraining data pipeline:
/// every phase, stage and component of the five-phase design, quick-read
/// summaries, cross-references to the implemented `codepipe` stages,
/// search, and links into the interactive map above. Self-contained page
/// mounted by the console SPA at `#pretrain/report`.
const PRETRAIN_REPORT_HTML: &str = include_str!("assets/pretrain_report.html");

/// Vendored chess.js rules library (classic script; exposes the global
/// `Chess`). Used by the Fun Time chess game and served as a public asset.
const CHESS_LIB_JS: &str = include_str!("assets/chess.js");

/// Fun Time · Game 5 — Chess (engine + UI, namespaced `ch*` / `.ch-`).
const CHESS_GAME_JS: &str = include_str!("assets/chess-game.js");

/// Fun Time · Timber Man Ultra game script (`assets/timberman-game.js`).
const TIMBERMAN_GAME_JS: &str = include_str!("assets/timberman-game.js");

/// Fun Time · Solitaire rules engine (pure logic, DOM-free).
const SOLITAIRE_ENGINE_JS: &str = include_str!("assets/solitaire-engine.js");

/// Fun Time · Game 7 — Solitaire (Classic Klondike, Draw 1) UI script.
const SOLITAIRE_GAME_JS: &str = include_str!("assets/solitaire-game.js");

/// Fun Time · Fruit Ninja game script (`assets/fruitninja-game.js`).
const FRUITNINJA_GAME_JS: &str = include_str!("assets/fruitninja-game.js");

/// Fun Time · Ludo rules engine (`assets/ludo-engine.js`). Pure, DOM-free.
const LUDO_ENGINE_JS: &str = include_str!("assets/ludo-engine.js");

/// Fun Time · Ludo game script (`assets/ludo-game.js`).
const LUDO_GAME_JS: &str = include_str!("assets/ludo-game.js");

/// Fun Time · Snakes & Ladders rules engine (`assets/snakes-engine.js`).
/// Pure, DOM-free.
const SNAKES_ENGINE_JS: &str = include_str!("assets/snakes-engine.js");

/// Fun Time · Snakes & Ladders game script (`assets/snakes-game.js`).
const SNAKES_GAME_JS: &str = include_str!("assets/snakes-game.js");

/// Internal evaluation workspaces (`assets/eval-workspaces.js`).
///
/// The six read-only surfaces over the `internal_eval` kit: question bank,
/// datasets, benchmark center, model evaluation, reports and tools. Loaded
/// by the console SPA after its own script (it reuses the SPA's helpers:
/// `api`, `esc`, `$`, `debounce`, `renderPager`, `mdToHtml`, `toast`,
/// `errorView`), so it stays a plain script rather than a module and keeps
/// working behind the session guard like every other workspace.
const EVAL_WORKSPACES_JS: &str = include_str!("assets/eval-workspaces.js");

/// Help & Support workspace (`assets/support-workspace.js`).
///
/// The role-aware ticket surface opened from the header button: normal
/// users get "Raise a Request" (submit + track their own tickets),
/// administrators get "Support Management" (all tickets, replies, status
/// changes, private notes). Loaded before the console SPA script because
/// it reuses the SPA's helpers (`api`, `el`, `esc`, `debounce`, `fmtTime`,
/// `fmtTimeFull`, `fmtInt`, `mdToHtml`, `toast`) — same pattern as the
/// eval workspaces script above.
const SUPPORT_WORKSPACE_JS: &str = include_str!("assets/support-workspace.js");

/// Sign-in / sign-up / registration page (one page, three views).
const AUTH_HTML: &str = include_str!("assets/auth.html");

/// Public password-reset completion page (User Management phase). Served
/// without a session by necessity — the user cannot sign in; the emailed
/// single-use token is the credential.
const RESET_PASSWORD_HTML: &str = include_str!("assets/reset_password.html");

/// Administrator approval queue (Phase 6). Served under the session guard:
/// unauthenticated visitors are redirected to sign-in by `apply_guards`,
/// and the page's own API calls are separately role-checked server-side.
const ADMIN_HTML: &str = include_str!("assets/admin.html");

/// `GET /` — the trace observatory.
async fn index() -> Response {
    html(INDEX_HTML)
}

/// `GET /system` — the system monitor.
async fn system() -> Response {
    html(SYSTEM_HTML)
}

/// `GET /console` — inference playground + durable trace explorer.
async fn console_page() -> Response {
    html(CONSOLE_HTML)
}

/// `GET /console/console-app.js` — the console SPA script.
async fn console_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        CONSOLE_JS,
    )
        .into_response()
}

/// `GET /shell.css` — shared shell + auth styles.
async fn shell_css() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SHELL_CSS,
    )
        .into_response()
}

/// `GET /pipeline-architecture` — the pretraining pipeline architecture
/// canvas, mounted inside the console at `#pretrain/architecture`.
async fn pipeline_architecture_page() -> Response {
    html(PIPELINE_ARCHITECTURE_HTML)
}

/// `GET /sft-architecture` — the SFT data-quality pipeline architecture
/// canvas, mounted inside the console at `#sft/architecture`.
async fn sft_architecture_page() -> Response {
    html(SFT_ARCHITECTURE_HTML)
}

/// `GET /sft-board` — the EKA SFT data-pipeline board, mounted inside the
/// console at `#sft/board`.
async fn sft_board_page() -> Response {
    html(SFT_BOARD_HTML)
}

/// `GET /sft-pipeline-map` — the original EKA SFT data-pipeline board
/// (`eka-sft-board-2.html`), opened from the SFT DataTrove workspace's
/// "Open pipeline map" entry in a new browser tab.
async fn sft_pipeline_map_page() -> Response {
    html(SFT_PIPELINE_MAP_HTML)
}

/// `GET /eka-checkpoints` — the original EKA Checkpoints by Level webpage,
/// mounted inside the console at `#eval/model` (Model Evaluation).
async fn eka_checkpoints_page() -> Response {
    html(EKA_CHECKPOINTS_HTML)
}

/// `GET /pretrain-report` — the pretraining pipeline architecture report,
/// mounted inside the console at `#pretrain/report`.
async fn pretrain_report_page() -> Response {
    html(PRETRAIN_REPORT_HTML)
}

/// `GET /console/chess.js` — vendored chess.js rules library (public asset).
async fn chess_lib_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        CHESS_LIB_JS,
    )
        .into_response()
}

/// `GET /console/chess-game.js` — the Fun Time chess game script.
async fn chess_game_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        CHESS_GAME_JS,
    )
        .into_response()
}

/// `GET /console/timberman-game.js` — the Fun Time Timber Man Ultra script.
async fn timberman_game_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        TIMBERMAN_GAME_JS,
    )
        .into_response()
}

/// `GET /console/fruitninja-game.js` — the Fun Time Fruit Ninja script.
async fn fruitninja_game_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        FRUITNINJA_GAME_JS,
    )
        .into_response()
}

/// `GET /console/ludo-engine.js` — the Ludo rules engine.
async fn ludo_engine_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        LUDO_ENGINE_JS,
    )
        .into_response()
}

/// `GET /console/ludo-game.js` — the Fun Time Ludo game script.
async fn ludo_game_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        LUDO_GAME_JS,
    )
        .into_response()
}

/// `GET /console/solitaire-engine.js` — the Solitaire rules engine.
async fn solitaire_engine_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SOLITAIRE_ENGINE_JS,
    )
        .into_response()
}

/// `GET /console/solitaire-game.js` — the Fun Time Solitaire game script.
async fn solitaire_game_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SOLITAIRE_GAME_JS,
    )
        .into_response()
}

/// `GET /console/snakes-game.js` — the Fun Time Snakes & Ladders script.
async fn snakes_game_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SNAKES_GAME_JS,
    )
        .into_response()
}

/// `GET /console/eval-workspaces.js` — the internal evaluation workspaces
/// script (question bank, datasets, benchmarks, model evaluation, reports,
/// tools). Public like the other `.js` assets: it contains no user data and
/// must survive a mid-session expiry without trapping the SPA. Its API
/// calls are separately session-guarded by `/api/eval/*`.
async fn eval_workspaces_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        EVAL_WORKSPACES_JS,
    )
        .into_response()
}

/// `GET /console/support-workspace.js` — the Help & Support workspace
/// script (ticket submission for users, ticket management for admins).
/// Public like the other `.js` assets: it contains no user data and must
/// survive a mid-session expiry without trapping the SPA. Its API calls
/// are separately session-guarded and role-checked by `/api/support/*`.
async fn support_workspace_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SUPPORT_WORKSPACE_JS,
    )
        .into_response()
}

/// `GET /console/snakes-engine.js` — the Snakes & Ladders rules engine.
async fn snakes_engine_js() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SNAKES_ENGINE_JS,
    )
        .into_response()
}

/// `GET /signin`, `GET /signup` and `GET /register` — the authentication
/// page. One document, three views; the client picks based on
/// `/api/auth/status` (first-run bootstrap vs. established install).
async fn auth_page() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        AUTH_HTML,
    )
        .into_response()
}

/// `GET /reset-password` — public password-reset completion.
///
/// The page reads the single-use token from `?token=…`, asks the server
/// whether it is still valid, and posts the new password to
/// `/api/auth/password/reset`. It never sees or transmits anything about
/// the account beyond what the token itself proves.
async fn reset_password_page() -> Response {
    (
        StatusCode::OK,
        [(
            header::CACHE_CONTROL,
            "no-store".to_string(),
        )],
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        RESET_PASSWORD_HTML,
    )
        .into_response()
}

/// `GET /admin` — the administrator approval queue.
///
/// The page itself is only session-guarded (any signed-in user can fetch
/// the HTML); the data it needs is refused by `/api/admin/*` unless the
/// caller is an active administrator, so a non-admin who loads it sees an
/// honest "not permitted" state rather than any account data.
async fn admin_page() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        ADMIN_HTML,
    )
        .into_response()
}

fn html(body: &'static str) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Router for static pages.
pub fn web_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/system", get(system))
        .route("/system/index.html", get(system))
        .route("/console", get(console_page))
        .route("/console/index.html", get(console_page))
        .route("/console/console-app.js", get(console_js))
        .route("/console/chess.js", get(chess_lib_js))
        .route("/console/chess-game.js", get(chess_game_js))
        .route("/console/timberman-game.js", get(timberman_game_js))
        .route("/console/solitaire-engine.js", get(solitaire_engine_js))
        .route("/console/solitaire-game.js", get(solitaire_game_js))
        .route("/console/fruitninja-game.js", get(fruitninja_game_js))
        .route("/console/ludo-engine.js", get(ludo_engine_js))
        .route("/console/ludo-game.js", get(ludo_game_js))
        .route("/console/snakes-engine.js", get(snakes_engine_js))
        .route("/console/snakes-game.js", get(snakes_game_js))
        .route("/console/eval-workspaces.js", get(eval_workspaces_js))
        .route("/console/support-workspace.js", get(support_workspace_js))
        .route("/pipeline-architecture", get(pipeline_architecture_page))
        .route(
            "/pipeline-architecture/index.html",
            get(pipeline_architecture_page),
        )
        .route("/sft-architecture", get(sft_architecture_page))
        .route("/sft-architecture/index.html", get(sft_architecture_page))
        .route("/sft-board", get(sft_board_page))
        .route("/sft-board/index.html", get(sft_board_page))
        .route("/sft-pipeline-map", get(sft_pipeline_map_page))
        .route("/sft-pipeline-map/index.html", get(sft_pipeline_map_page))
        .route("/eka-checkpoints", get(eka_checkpoints_page))
        .route("/eka-checkpoints/index.html", get(eka_checkpoints_page))
        .route("/pretrain-report", get(pretrain_report_page))
        .route("/pretrain-report/index.html", get(pretrain_report_page))
        .route("/shell.css", get(shell_css))
        .route("/signin", get(auth_page))
        .route("/signup", get(auth_page))
        .route("/register", get(auth_page))
        .route("/reset-password", get(reset_password_page))
        .route("/admin", get(admin_page))
}
