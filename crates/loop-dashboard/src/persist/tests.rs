//! Persistence tests: durability, restart survival, migrations, retention,
//! concurrent writes, filtering, pagination.

use std::sync::Arc;

use crate::persist::{
    DashboardStore, MessageRow, RequestFilter, RequestRow, RetentionConfig, SpanRow, TraceFilter,
    TraceRow, TraceStatus,
};

fn sample_trace(id: &str, started: i64, status: TraceStatus) -> TraceRow {
    TraceRow {
        trace_id: id.into(),
        root_span_id: format!("{id}-root"),
        operation: "dashboard.chat".into(),
        status,
        model: Some("m1".into()),
        provider: Some("soket".into()),
        request_id: Some(format!("{id}-req")),
        session_id: None,
        started_at_ms: started,
        ended_at_ms: Some(started + 100),
        duration_ms: Some(100),
        input_tokens: Some(10),
        output_tokens: Some(20),
        ttft_ms: Some(50),
        error_message: None,
        params_json: Some(r#"{"temperature":0.7}"#.into()),
        content_captured: false,
        owner_pid: None,
    }
}

/// Matching request row for a trace row (referential integrity in tests).
fn request_of(t: &TraceRow) -> RequestRow {
    RequestRow {
        request_id: t.request_id.clone().unwrap_or_default(),
        trace_id: t.trace_id.clone(),
        conversation_id: None,
        model: t.model.clone(),
        provider: t.provider.clone(),
        status: t.status,
        started_at_ms: t.started_at_ms,
        ended_at_ms: t.ended_at_ms,
        duration_ms: t.duration_ms,
        input_tokens: t.input_tokens,
        output_tokens: t.output_tokens,
        ttft_ms: t.ttft_ms,
        cancel_reason: None,
        error_message: None,
    }
}

#[test]
fn migrations_are_idempotent_and_versioned() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("store.db");
    let store = DashboardStore::open(&path).unwrap();
    // Reopening runs migrations again — must be a no-op.
    drop(store);
    let store2 = DashboardStore::open(&path).unwrap();
    let applied = store2
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT COUNT(*) FROM schema_migrations")?;
            let n: i64 = stmt.query_row([], |r| r.get(0))?;
            Ok(n)
        })
        .unwrap();
    // The count must equal the number of compiled-in migrations exactly:
    // each recorded once, none missing, none duplicated. Asserting against
    // the registry (rather than a literal) keeps this test correct as the
    // schema evolves.
    assert_eq!(
        applied as usize,
        crate::persist::MIGRATIONS.len(),
        "all migrations recorded exactly once"
    );
}

#[test]
fn traces_survive_process_restart() {
    // The restart test: open, write, DROP the store (simulating process exit),
    // reopen from disk, and verify data.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("store.db");

    {
        let store = DashboardStore::open(&path).unwrap();
        store
            .insert_trace(&sample_trace("t1", 1000, TraceStatus::Success))
            .unwrap();
        store
            .insert_span(&SpanRow {
                trace_id: "t1".into(),
                span_id: "t1-root".into(),
                parent_span_id: None,
                name: "http.request".into(),
                kind: "phase".into(),
                status: TraceStatus::Success,
                start_ms: 1000,
                end_ms: Some(1100),
                duration_ms: Some(100),
                attributes_json: None,
            })
            .unwrap();
        store
            .insert_request(&RequestRow {
                request_id: "t1-req".into(),
                trace_id: "t1".into(),
                conversation_id: None,
                model: Some("m1".into()),
                provider: Some("soket".into()),
                status: TraceStatus::Success,
                started_at_ms: 1000,
                ended_at_ms: Some(1100),
                duration_ms: Some(100),
                input_tokens: Some(10),
                output_tokens: Some(20),
                ttft_ms: Some(50),
                cancel_reason: None,
                error_message: None,
            })
            .unwrap();
    } // store dropped: connection closed, like a process exit

    // Reopen: everything must still be there.
    let store = DashboardStore::open(&path).unwrap();
    let trace = store
        .get_trace("t1")
        .unwrap()
        .expect("trace survived restart");
    assert_eq!(trace.operation, "dashboard.chat");
    assert_eq!(trace.input_tokens, Some(10));
    let detail = store.get_trace_detail("t1").unwrap().unwrap();
    assert_eq!(detail.spans.len(), 1);
    assert_eq!(detail.spans[0].name, "http.request");
    let (traces, _spans, requests) = store.counts().unwrap();
    assert_eq!((traces, requests), (1, 1));
}

#[test]
fn running_rows_recover_as_interrupted_not_success() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("store.db");

    {
        let store = DashboardStore::open(&path).unwrap();
        // A trace that "crashed" mid-flight: still running, owned by a
        // process that no longer exists. (`insert_trace` defaults a NULL
        // owner to this process, which is alive — an explicit dead pid
        // simulates the crashed writer.)
        let mut row = sample_trace("crash", 500, TraceStatus::Running);
        row.owner_pid = Some(-1);
        store.insert_trace(&row).unwrap();
    }

    // Next process open recovers it honestly.
    let store = DashboardStore::open(&path).unwrap();
    let trace = store.get_trace("crash").unwrap().unwrap();
    assert_eq!(trace.status, TraceStatus::Interrupted);
    assert_ne!(
        trace.status,
        TraceStatus::Success,
        "never silently converted"
    );
    assert!(
        trace.error_message.unwrap().contains("process exited"),
        "carries an honest explanation"
    );
}

#[test]
fn unique_ids_across_many_writes() {
    let store = DashboardStore::open_in_memory().unwrap();
    let mut ids = std::collections::HashSet::new();
    for i in 0..200 {
        let id = uuid::Uuid::now_v7().to_string();
        assert!(ids.insert(id.clone()), "uuid v7 collision at {i}");
        store
            .insert_trace(&sample_trace(&id, i, TraceStatus::Success))
            .unwrap();
    }
    let (traces, _, _) = store.counts().unwrap();
    assert_eq!(traces, 200);
}

#[test]
fn concurrent_writes_are_safe() {
    // One store, many threads writing through separate clones of the handle.
    let store = Arc::new(DashboardStore::open_in_memory().unwrap());
    let mut handles = Vec::new();
    for t in 0..8 {
        let store = Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            for i in 0..25 {
                let id = format!("t{t}-{i}");
                store
                    .insert_trace(&sample_trace(&id, 1000 + i, TraceStatus::Success))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let (traces, _, _) = store.counts().unwrap();
    assert_eq!(traces, 200, "all concurrent writes landed");
}

#[test]
fn filtering_and_pagination_happen_in_sql() {
    let store = DashboardStore::open_in_memory().unwrap();
    for i in 0..50 {
        let status = if i % 3 == 0 {
            TraceStatus::Error
        } else {
            TraceStatus::Success
        };
        let mut row = sample_trace(&format!("t{i}"), 10_000 + i * 10, status);
        row.model = Some(if i % 2 == 0 { "m1".into() } else { "m2".into() });
        row.duration_ms = Some(i * 10);
        store.insert_trace(&row).unwrap();
    }

    // Status filter.
    let f = TraceFilter {
        statuses: Some(vec![TraceStatus::Error]),
        page: 1,
        per_page: 10,
        ..Default::default()
    };
    let (items, total) = store.list_traces(&f).unwrap();
    assert_eq!(total, 17); // ceil(50/3)
    assert!(items.len() <= 10);
    assert!(items.iter().all(|t| t.status == TraceStatus::Error));

    // Model filter.
    let f = TraceFilter {
        models: Some(vec!["m2".into()]),
        page: 1,
        per_page: 100,
        ..Default::default()
    };
    let (_, total) = store.list_traces(&f).unwrap();
    assert_eq!(total, 25);

    // Time range.
    let f = TraceFilter {
        from_ms: Some(10_100),
        to_ms: Some(10_200),
        page: 1,
        per_page: 100,
        ..Default::default()
    };
    let (items, total) = store.list_traces(&f).unwrap();
    assert_eq!(total, 11);
    assert!(items.first().unwrap().started_at_ms >= 10_100);

    // Duration sort ascending.
    let f = TraceFilter {
        sort: Some("duration".into()),
        dir: Some("asc".into()),
        page: 1,
        per_page: 5,
        ..Default::default()
    };
    let (items, _) = store.list_traces(&f).unwrap();
    let durations: Vec<_> = items.iter().map(|t| t.duration_ms.unwrap()).collect();
    let mut sorted = durations.clone();
    sorted.sort_unstable();
    assert_eq!(durations, sorted);

    // Free-text search by trace id.
    let f = TraceFilter {
        q: "t42".into(),
        page: 1,
        per_page: 10,
        ..Default::default()
    };
    let (_, total) = store.list_traces(&f).unwrap();
    assert_eq!(total, 1);

    // Page beyond the end returns empty but correct totals.
    let f = TraceFilter {
        page: 99,
        per_page: 10,
        ..Default::default()
    };
    let (items, total) = store.list_traces(&f).unwrap();
    assert_eq!(total, 50);
    assert!(items.is_empty());
}

#[test]
fn per_page_is_capped() {
    let store = DashboardStore::open_in_memory().unwrap();
    store
        .insert_trace(&sample_trace("t1", 1, TraceStatus::Success))
        .unwrap();
    let f = TraceFilter {
        page: 1,
        per_page: 1_000_000, // absurd
        ..Default::default()
    };
    let (items, _) = store.list_traces(&f).unwrap();
    assert_eq!(items.len(), 1, "cap applied without failing");
}

#[test]
fn retention_deletes_oldest_beyond_cap() {
    let store = DashboardStore::open_in_memory().unwrap();
    for i in 0..30 {
        let row = sample_trace(&format!("t{i}"), 1000 + i, TraceStatus::Success);
        store.insert_request(&request_of(&row)).unwrap();
        store.insert_trace(&row).unwrap();
    }
    let (deleted_traces, deleted_requests) = store
        .enforce_retention(&RetentionConfig {
            max_traces: 10,
            max_age_ms: None,
        })
        .unwrap();
    assert_eq!(deleted_traces, 20);
    assert_eq!(deleted_requests, 20);
    let (traces, _, requests) = store.counts().unwrap();
    assert_eq!((traces, requests), (10, 10));
    // The newest ten survive.
    let f = TraceFilter {
        sort: Some("started".into()),
        dir: Some("desc".into()),
        page: 1,
        per_page: 1,
        ..Default::default()
    };
    let (items, _) = store.list_traces(&f).unwrap();
    assert_eq!(items[0].trace_id, "t29");
}

#[test]
fn retention_by_age() {
    let store = DashboardStore::open_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    for row in [
        sample_trace("old", now - 10_000_000, TraceStatus::Success),
        sample_trace("new", now - 100, TraceStatus::Success),
    ] {
        store.insert_request(&request_of(&row)).unwrap();
        store.insert_trace(&row).unwrap();
    }
    store
        .enforce_retention(&RetentionConfig {
            max_traces: 0,
            max_age_ms: Some(1_000_000),
        })
        .unwrap();
    let (traces, _, requests) = store.counts().unwrap();
    assert_eq!((traces, requests), (1, 1));
    assert!(store.get_trace("new").unwrap().is_some());
    assert!(store.get_trace("old").unwrap().is_none());
}

#[test]
fn conversations_persist_and_order() {
    let store = DashboardStore::open_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    store
        .create_conversation(&crate::persist::ConversationRow {
            conversation_id: "c1".into(),
            owner_account_id: Some(7),
            title: "First chat".into(),
            created_at_ms: now,
            updated_at_ms: now,
            model: Some("m1".into()),
        })
        .unwrap();
    for (seq, role, content) in [
        (1, "user", "hello"),
        (2, "assistant", "hi there"),
        (3, "user", "again"),
    ] {
        store
            .append_message(&MessageRow {
                conversation_id: "c1".into(),
                message_id: format!("m{seq}"),
                seq,
                role: role.into(),
                content: content.into(),
                created_at_ms: now + seq,
                status: "complete".into(),
                error_message: None,
                model: None,
                request_id: None,
                trace_id: None,
            })
            .unwrap();
    }
    let messages = store.list_messages("c1").unwrap();
    assert_eq!(messages.len(), 3);
    // Ordering preserved.
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[2].content, "again");

    let summaries = store.list_conversations(10).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].message_count, 3);

    // Ownership scoping: the same conversation is invisible to another
    // account, and rename/delete through that other account are no-ops.
    assert!(store.list_conversations_for_owner(8, 10).unwrap().is_empty());
    assert!(!store.rename_conversation_for_owner("c1", 8, "Stolen").unwrap());
    assert!(!store.delete_conversation_for_owner("c1", 8).unwrap());
    // The owner still sees it untouched.
    assert_eq!(
        store
            .list_conversations_for_owner(7, 10)
            .unwrap()[0]
            .title,
        "First chat"
    );

    // Rename + delete by the real owner.
    assert!(store.rename_conversation_for_owner("c1", 7, "Renamed").unwrap());
    assert!(store.delete_conversation_for_owner("c1", 7).unwrap());
    assert!(store.list_messages("c1").unwrap().is_empty(), "cascade");
}

#[test]
fn failed_messages_are_represented_honestly() {
    let store = DashboardStore::open_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    store
        .create_conversation(&crate::persist::ConversationRow {
            conversation_id: "c1".into(),
            owner_account_id: Some(1),
            title: "t".into(),
            created_at_ms: now,
            updated_at_ms: now,
            model: None,
        })
        .unwrap();
    store
        .append_message(&MessageRow {
            conversation_id: "c1".into(),
            message_id: "m1".into(),
            seq: 1,
            role: "user".into(),
            content: "q".into(),
            created_at_ms: now,
            status: "complete".into(),
            error_message: None,
            model: None,
            request_id: None,
            trace_id: None,
        })
        .unwrap();
    // A failed assistant turn keeps its partial text + error.
    store
        .update_message("c1", "m2", "", "error", Some("boom"))
        .unwrap();
    // (m2 was inserted by append in real flow; simulate directly:)
    store
        .append_message(&MessageRow {
            conversation_id: "c1".into(),
            message_id: "m2".into(),
            seq: 2,
            role: "assistant".into(),
            content: String::new(),
            created_at_ms: now + 1,
            status: "streaming".into(),
            error_message: None,
            model: Some("m1".into()),
            request_id: None,
            trace_id: None,
        })
        .unwrap();
    store
        .update_message("c1", "m2", "partial…", "error", Some("boom"))
        .unwrap();
    let messages = store.list_messages("c1").unwrap();
    assert_eq!(messages[1].status, "error");
    assert_eq!(messages[1].error_message.as_deref(), Some("boom"));
    assert_eq!(messages[1].content, "partial…");
}

#[test]
fn settings_round_trip() {
    let store = DashboardStore::open_in_memory().unwrap();
    store.save_setting("captureContent", "false").unwrap();
    store.save_setting("retentionMaxTraces", "1234").unwrap();
    let map = store.load_settings().unwrap();
    assert_eq!(
        map.0.get("captureContent").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        map.0.get("retentionMaxTraces").map(String::as_str),
        Some("1234")
    );
}

#[test]
fn storage_errors_do_not_corrupt() {
    let store = DashboardStore::open_in_memory().unwrap();
    store
        .insert_trace(&sample_trace("t1", 1, TraceStatus::Success))
        .unwrap();
    // Duplicate primary key is an error, not corruption.
    let err = store.insert_trace(&sample_trace("t1", 2, TraceStatus::Success));
    assert!(err.is_err());
    let (traces, _, _) = store.counts().unwrap();
    assert_eq!(traces, 1, "original row intact after failed write");
}

#[test]
fn request_filtering_works() {
    let store = DashboardStore::open_in_memory().unwrap();
    for i in 0..10 {
        store
            .insert_request(&RequestRow {
                request_id: format!("r{i}"),
                trace_id: format!("t{i}"),
                conversation_id: None,
                model: Some("m1".into()),
                provider: Some("p".into()),
                status: if i < 3 {
                    TraceStatus::CancelledUser
                } else {
                    TraceStatus::Success
                },
                started_at_ms: 1000 + i,
                ended_at_ms: None,
                duration_ms: None,
                input_tokens: None,
                output_tokens: None,
                ttft_ms: None,
                cancel_reason: if i < 3 { Some("user".into()) } else { None },
                error_message: None,
            })
            .unwrap();
    }
    let f = RequestFilter {
        statuses: Some(vec![TraceStatus::CancelledUser]),
        page: 1,
        per_page: 10,
        ..Default::default()
    };
    let (items, total) = store.list_requests(&f).unwrap();
    assert_eq!(total, 3);
    assert!(items
        .iter()
        .all(|r| r.cancel_reason.as_deref() == Some("user")));
}
