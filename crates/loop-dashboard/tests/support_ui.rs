//! Help & Support — integration checks for the header-button ticket system.
//!
//! Three layers, mirroring `admin_ui.rs`:
//!
//! 1. **API integration** — through the *full* app (guard layer + support
//!    router), so session-guard and role-check behaviour is exercised
//!    exactly as a browser would hit it.
//! 2. **Authorization** — normal users must only ever see their own
//!    tickets; private internal notes must never reach them; every admin
//!    endpoint must refuse non-admins with 403.
//! 3. **UI wiring** — the console SPA must carry the header button with
//!    the same styling contract as Approvals, load the workspace script,
//!    and the served assets must contain no secrets.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // oneshot

use loop_dashboard::api::{build_app, AppOptions};
use loop_dashboard::auth::{AuthStore, SESSION_COOKIE, STATUS_APPROVED};
use loop_dashboard::support::store::SupportStore;

/// Full app (guards + auth + support) over temp DBs, with one approved
/// administrator signed in plus one approved ordinary user.
///
/// Returns `(app, dir, admin_cookie, user_cookie)`.
async fn fixture() -> (
    axum::Router,
    tempfile::TempDir,
    String,
    String,
) {
    let dir = tempfile::tempdir().unwrap();
    let auth = AuthStore::open(dir.path().join("auth.db")).unwrap();
    // Bootstrap the sole administrator exactly as a fresh install would.
    auth.sign_up(
        loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER,
        "correct horse battery",
    )
    .unwrap();
    // One ordinary user, already approved so they can sign in.
    let user_id = auth
        .create_pending_user("someone@soket.ai", "a strong passphrase")
        .unwrap();
    auth.set_account_status(user_id, STATUS_APPROVED).unwrap();

    let admin_token = auth
        .sign_in(
            loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER,
            "correct horse battery",
        )
        .unwrap();
    let user_token = auth
        .sign_in("someone@soket.ai", "a strong passphrase")
        .unwrap();
    drop(auth);

    let tickets = Arc::new(SupportStore::open(dir.path().join("support.db")).unwrap());
    let (app, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        console_state: None,
        auth_store: Some(Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap())),
        support_store: Some(tickets),
    })
    .unwrap();

    let admin_cookie = format!("{SESSION_COOKIE}={admin_token}");
    let user_cookie = format!("{SESSION_COOKIE}={user_token}");
    (app, dir, admin_cookie, user_cookie)
}

/// GET JSON through the full router with an optional cookie.
async fn get_json(
    app: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// POST JSON through the full router with an optional cookie.
async fn post_json(
    app: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// A minimal valid submission body.
fn submit_body(subject: &str, priority: &str) -> String {
    serde_json::json!({
        "subject": subject,
        "category": "technical_issue",
        "priority": priority,
        "description": "The playground stops streaming after ~30 seconds.",
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Submission + confirmation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn user_can_submit_and_sees_confirmation_with_ticket_id() {
    let (app, _dir, _admin, user) = fixture().await;
    let (status, body) = post_json(
        &app,
        "/api/support/tickets",
        Some(&user),
        &submit_body("Streaming stops", "high"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["ok"], serde_json::json!(true));
    let ticket = &body["ticket"];
    let id = ticket["ticketId"].as_str().unwrap();
    assert!(
        id.starts_with("TKT-") && id.len() == "TKT-".len() + 6,
        "ticket id must be TKT-XXXXXX: {id}"
    );
    // Identity fields come from the AUTHENTICATED account, not the request.
    assert_eq!(ticket["userEmail"], "someone@soket.ai");
    assert_eq!(ticket["status"], "pending");
    assert_eq!(ticket["priority"], "high");
    // Email status is reported honestly (no mailer configured in tests).
    assert!(
        body["emailStatus"].as_str().unwrap().contains("not configured"),
        "must be honest about email: {}",
        body["emailStatus"]
    );
}

#[tokio::test]
async fn submission_validates_fields_server_side() {
    let (app, _dir, _admin, user) = fixture().await;

    // Missing subject.
    let bad = serde_json::json!({
        "subject": "   ",
        "category": "technical_issue",
        "priority": "low",
        "description": "x",
    })
    .to_string();
    let (s, b) = post_json(&app, "/api/support/tickets", Some(&user), &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");

    // Unknown category.
    let bad = serde_json::json!({
        "subject": "s",
        "category": "not_a_category",
        "priority": "low",
        "description": "x",
    })
    .to_string();
    let (s, _) = post_json(&app, "/api/support/tickets", Some(&user), &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Unknown priority.
    let bad = serde_json::json!({
        "subject": "s",
        "category": "other",
        "priority": "urgent!!!",
        "description": "x",
    })
    .to_string();
    let (s, _) = post_json(&app, "/api/support/tickets", Some(&user), &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Empty description.
    let bad = serde_json::json!({
        "subject": "s",
        "category": "other",
        "priority": "low",
        "description": "",
    })
    .to_string();
    let (s, _) = post_json(&app, "/api/support/tickets", Some(&user), &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn attachment_validation_rejects_bad_type_and_oversize() {
    let (app, _dir, _admin, user) = fixture().await;

    // Disallowed content type (HTML must never be storable).
    let bad = serde_json::json!({
        "subject": "s", "category": "other", "priority": "low",
        "description": "x",
        "attachments": [{
            "filename": "evil.html",
            "contentType": "text/html",
            "data": "PGI+aGk8L2I+",
        }],
    })
    .to_string();
    let (s, b) = post_json(&app, "/api/support/tickets", Some(&user), &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");

    // Oversized payload: > 5 MiB of decoded data.
    let five_mib = vec![b'A'; 5 * 1024 * 1024 + 1024];
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(&five_mib);
    let bad = serde_json::json!({
        "subject": "s", "category": "other", "priority": "low",
        "description": "x",
        "attachments": [{ "filename": "big.png", "contentType": "image/png", "data": data }],
    })
    .to_string();
    let (s, b) = post_json(&app, "/api/support/tickets", Some(&user), &bad).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

#[tokio::test]
async fn unauthenticated_submission_is_refused() {
    let (app, _dir, _admin, _user) = fixture().await;
    // No cookie: the guard layer answers 401 before anything else.
    let (s, b) = post_json(&app, "/api/support/tickets", None, &submit_body("x", "low")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");
}

// ---------------------------------------------------------------------------
// Ownership scoping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn user_lists_only_their_own_tickets() {
    let (app, dir, admin, user) = fixture().await;

    // The user submits one ticket…
    let (_, u1) = post_json(&app, "/api/support/tickets", Some(&user), &submit_body("Mine", "low")).await;
    // …and the admin submits another (admins may also raise requests).
    let (_, a1) =
        post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("Admin one", "low")).await;

    // The user's listing contains ONLY their ticket.
    let (s, list) = get_json(&app, "/api/support/tickets", Some(&user)).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "user must see exactly their own ticket");
    assert_eq!(items[0]["ticketId"], u1["ticket"]["ticketId"]);

    // The admin listing sees both.
    let (s, list) = get_json(&app, "/api/support/admin/tickets", Some(&admin)).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list["total"], serde_json::json!(2));
    let _ = (dir, a1);
}

#[tokio::test]
async fn user_cannot_read_another_users_ticket_detail() {
    let (app, _dir, admin, user) = fixture().await;
    // Admin's ticket…
    let (_, a1) =
        post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("Admin secret", "low")).await;
    let id = a1["ticket"]["ticketId"].as_str().unwrap().to_string();

    // …must be invisible to the ordinary user (404, indistinguishable
    // from a nonexistent id, so ids cannot be probed).
    let (s, b) = get_json(&app, &format!("/api/support/tickets/{id}"), Some(&user)).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");

    // But the admin can read it.
    let (s, _) = get_json(&app, &format!("/api/support/tickets/{id}"), Some(&admin)).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn user_cannot_download_another_users_attachment() {
    let (app, _dir, admin, user) = fixture().await;
    // Admin ticket with a real attachment (small valid PNG header bytes).
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nrest");
    let body = serde_json::json!({
        "subject": "With file", "category": "bug_report", "priority": "medium",
        "description": "see attached",
        "attachments": [{ "filename": "shot.png", "contentType": "image/png", "data": png }],
    })
    .to_string();
    let (_, a1) = post_json(&app, "/api/support/tickets", Some(&admin), &body).await;
    let id = a1["ticket"]["ticketId"].as_str().unwrap().to_string();

    // Resolve the attachment id via the admin detail view.
    let (_, detail) = get_json(&app, &format!("/api/support/tickets/{id}"), Some(&admin)).await;
    let att_id = detail["attachments"][0]["id"].as_i64().unwrap();

    // The ordinary user must not be able to download it.
    let (s, _) = get_json(
        &app,
        &format!("/api/support/attachments/{att_id}"),
        Some(&user),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // The admin can.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/support/attachments/{att_id}"))
                .header(header::COOKIE, &admin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    // Attachment disposition prevents inline rendering of untrusted files.
    assert!(res
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("attachment"));
}

// ---------------------------------------------------------------------------
// Role-based access to the admin surface
// ---------------------------------------------------------------------------

#[tokio::test]
async fn role_probe_reports_admin_and_user_correctly() {
    let (app, _dir, admin, user) = fixture().await;
    let (s, me) = get_json(&app, "/api/support/me", Some(&admin)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["isAdmin"], serde_json::json!(true));

    let (s, me) = get_json(&app, "/api/support/me", Some(&user)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["isAdmin"], serde_json::json!(false));
    assert_eq!(me["identifier"], "someone@soket.ai");
}

#[tokio::test]
async fn admin_endpoints_refuse_normal_users() {
    let (app, _dir, _admin, user) = fixture().await;
    for uri in [
        "/api/support/admin/summary",
        "/api/support/admin/tickets",
        "/api/support/admin/tickets/TKT-XXXXXX/status",
        "/api/support/admin/tickets/TKT-XXXXXX/reply",
        "/api/support/admin/tickets/TKT-XXXXXX/note",
    ] {
        if uri.ends_with("/status") || uri.ends_with("/reply") || uri.ends_with("/note") {
            let (s, b) = post_json(&app, uri, Some(&user), "{}").await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{uri}: {b}");
        } else {
            let (s, b) = get_json(&app, uri, Some(&user)).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{uri}: {b}");
        }
    }
}

#[tokio::test]
async fn admin_summary_counts_match_submissions() {
    let (app, _dir, admin, _user) = fixture().await;
    // Two tickets: one pending/medium, one pending/critical.
    post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("one", "medium")).await;
    post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("two", "critical")).await;

    let (s, sum) = get_json(&app, "/api/support/admin/summary", Some(&admin)).await;
    assert_eq!(s, StatusCode::OK, "{sum}");
    assert_eq!(sum["total"], serde_json::json!(2));
    assert_eq!(sum["pending"], serde_json::json!(2));
    assert_eq!(sum["inProgress"], serde_json::json!(0));
    assert_eq!(sum["resolved"], serde_json::json!(0));
    assert_eq!(sum["critical"], serde_json::json!(1));
}

// ---------------------------------------------------------------------------
// Admin workflow: status, reply, internal note, history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_full_workflow_reply_status_note_history() {
    let (app, _dir, admin, user) = fixture().await;

    // User submits.
    let (_, t) = post_json(&app, "/api/support/tickets", Some(&user), &submit_body("Workflow", "high")).await;
    let id = t["ticket"]["ticketId"].as_str().unwrap().to_string();

    // Admin replies.
    let (s, r) = post_json(
        &app,
        &format!("/api/support/admin/tickets/{id}/reply"),
        Some(&admin),
        r#"{"message":"We reproduced it; a fix is on the way."}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");

    // Admin adds a PRIVATE internal note.
    let (s, r) = post_json(
        &app,
        &format!("/api/support/admin/tickets/{id}/note"),
        Some(&admin),
        r#"{"note":"Internal: suspected race in the stream pump, see PR 482."}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");

    // Admin changes status pending → in_progress.
    let (s, r) = post_json(
        &app,
        &format!("/api/support/admin/tickets/{id}/status"),
        Some(&admin),
        r#"{"status":"in_progress","expectedCurrent":"pending"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["ticket"]["status"], "in_progress");

    // ---- ADMIN sees the full history including the internal note ----
    let (_, d) = get_json(&app, &format!("/api/support/tickets/{id}"), Some(&admin)).await;
    assert_eq!(d["canManage"], serde_json::json!(true));
    let events = d["events"].as_array().unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"created"), "history must start at created: {kinds:?}");
    assert!(kinds.contains(&"reply"), "reply must be recorded: {kinds:?}");
    assert!(kinds.contains(&"internal_note"), "admin must see the note: {kinds:?}");
    assert!(kinds.contains(&"status_changed"), "status change must be recorded: {kinds:?}");

    // ---- USER sees the history WITHOUT the internal note ----
    let (_, d) = get_json(&app, &format!("/api/support/tickets/{id}"), Some(&user)).await;
    assert_eq!(d["canManage"], serde_json::json!(false));
    let events = d["events"].as_array().unwrap();
    let has_note = events
        .iter()
        .any(|e| e["kind"] == "internal_note");
    assert!(!has_note, "internal notes must NEVER reach a normal user");
    let has_reply = events.iter().any(|e| e["kind"] == "reply");
    assert!(has_reply, "the user must see the admin's reply");
    // And the raw note text must not appear anywhere in the payload.
    let raw = d.to_string();
    assert!(
        !raw.contains("stream pump"),
        "note text leaked into the user response: {raw}"
    );
}

#[tokio::test]
async fn status_change_with_stale_expectation_conflicts() {
    let (app, _dir, admin, _user) = fixture().await;
    let (_, t) = post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("CAS", "low")).await;
    let id = t["ticket"]["ticketId"].as_str().unwrap().to_string();

    // First change succeeds.
    let (s, _) = post_json(
        &app,
        &format!("/api/support/admin/tickets/{id}/status"),
        Some(&admin),
        r#"{"status":"in_progress","expectedCurrent":"pending"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Second change claiming the ticket is still 'pending' must conflict.
    let (s, b) = post_json(
        &app,
        &format!("/api/support/admin/tickets/{id}/status"),
        Some(&admin),
        r#"{"status":"resolved","expectedCurrent":"pending"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
}

#[tokio::test]
async fn unknown_status_value_is_rejected() {
    let (app, _dir, admin, _user) = fixture().await;
    let (_, t) = post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("s", "low")).await;
    let id = t["ticket"]["ticketId"].as_str().unwrap().to_string();
    let (s, b) = post_json(
        &app,
        &format!("/api/support/admin/tickets/{id}/status"),
        Some(&admin),
        r#"{"status":"done!!!"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

#[tokio::test]
async fn filters_and_search_work_on_the_admin_listing() {
    let (app, _dir, admin, _user) = fixture().await;
    post_json(&app, "/api/support/tickets", Some(&admin), &submit_body("Login broken", "high")).await;
    post_json(
        &app,
        "/api/support/tickets",
        Some(&admin),
        &serde_json::json!({
            "subject": "Pipeline stalls",
            "category": "data_pipeline",
            "priority": "low",
            "description": "stage 4 never finishes",
        })
        .to_string(),
    )
    .await;

    // Filter by category.
    let (s, l) = get_json(
        &app,
        "/api/support/admin/tickets?category=data_pipeline",
        Some(&admin),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(l["total"], serde_json::json!(1));
    assert_eq!(l["items"][0]["subject"], "Pipeline stalls");

    // Search by subject substring.
    let (s, l) = get_json(&app, "/api/support/admin/tickets?q=login", Some(&admin)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(l["total"], serde_json::json!(1));
    assert_eq!(l["items"][0]["subject"], "Login broken");

    // Search by ticket id prefix.
    let (_, all) = get_json(&app, "/api/support/admin/tickets", Some(&admin)).await;
    let first_id = all["items"][0]["ticketId"].as_str().unwrap().to_string();
    let (s, l) = get_json(
        &app,
        &format!("/api/support/admin/tickets?q={}", &first_id[..6]),
        Some(&admin),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(l["total"], serde_json::json!(1));

    // Unknown filter value: honest 400, not a silent ignore.
    let (s, _) = get_json(
        &app,
        "/api/support/admin/tickets?status=nonsense",
        Some(&admin),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// UI wiring: the header button contract
// ---------------------------------------------------------------------------

/// GET raw text through the full router with an optional cookie.
async fn get_text(
    app: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn console_page_carries_the_header_button_matching_approvals() {
    let (app, _dir, _admin, user) = fixture().await;
    // The console page is session-guarded; fetch it signed in (the guard
    // would redirect an anonymous visitor to /signin).
    let (s, html) = get_text(&app, "/console", Some(&user)).await;
    assert_eq!(s, StatusCode::OK);

    // The button exists, sits inside the user chip, BEFORE Sign out, and
    // uses the same .sw-signout base class as Approvals.
    assert!(html.contains("id=\"supportBtn\""), "button must exist");
    assert!(
        html.contains("sw-support-btn"),
        "button carries its own class for icon/badge styling"
    );
    assert!(
        html.contains("class=\"sw-signout sw-support-btn\""),
        "button must reuse the .sw-signout visual language"
    );
    let chip_start = html.find("id=\"userChip\"").expect("user chip");
    let chip_html = &html[chip_start..];
    let support_pos = chip_html.find("id=\"supportBtn\"").expect("support btn in chip");
    let signout_pos = chip_html.find("id=\"signOutBtn\"").expect("sign out in chip");
    assert!(
        support_pos < signout_pos,
        "Support must sit before Sign out, beside Approvals"
    );
    let approvals_pos = chip_html.find("id=\"adminNavLink\"").expect("approvals link");
    assert!(
        approvals_pos < support_pos,
        "Support comes after Approvals (beside the account section)"
    );

    // Hidden until the server role probe answers — never a default-visible
    // admin surface.
    assert!(
        html.contains("id=\"supportBtn\" type=\"button\" hidden"),
        "button starts hidden"
    );

    // The workspace script is loaded BEFORE console-app.js (it reuses the
    // SPA's helpers, which are defined there at parse time).
    let support_js = html.find("/console/support-workspace.js").expect("script tag");
    let app_js = html.find("/console/console-app.js").expect("app script tag");
    assert!(support_js < app_js, "support-workspace.js must load first");
}

#[tokio::test]
async fn support_workspace_script_is_served_as_a_public_asset() {
    let (app, _dir, _admin, _user) = fixture().await;
    let (s, body) = get_text(&app, "/console/support-workspace.js", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("Raise a Request"), "user label present");
    assert!(body.contains("Support Management"), "admin label present");
    // No secrets in the asset.
    assert!(!body.to_lowercase().contains("api_key"));
    assert!(!body.to_lowercase().contains("bearer "));
}

#[tokio::test]
async fn shell_css_styles_the_button_and_overlay() {
    let (app, _dir, _admin, _user) = fixture().await;
    let (s, css) = get_text(&app, "/shell.css", None).await;
    assert_eq!(s, StatusCode::OK);
    // Button styling exists…
    assert!(css.contains(".sw-support-btn:not([hidden])"));
    // …and the overlay panel too.
    assert!(css.contains(".sup-overlay"));
    assert!(css.contains(".sup-panel"));
    // Responsive rule for narrow screens.
    assert!(css.contains("@media (max-width:760px)"));
}
