//! loop-dashboard — local observability + inference console for Loop.
//!
//! The library builds one axum router ([`api::build_app`]) bundling three
//! surfaces over the same process:
//!
//! * `/` — read-only trace observatory over the session database,
//! * `/system` — live local system monitor,
//! * `/console` — inference playground with durable traces/requests plus
//!   the internal AI-evaluation workspaces (`eval`).
//!
//! The standalone `loop-dashboard` binary serves this router directly; the
//! `loop` CLI embeds the same router in-process (see `loop-cli`'s
//! `dashboard` module) so the console runs against the live application
//! runtime instead of a copy of its configuration.

pub mod admin_http;
pub mod analytics;
pub mod analytics_admin;
pub mod api;
pub mod auth;
pub mod auth_email;
pub mod auth_http;
pub mod auth_otp;
pub mod console;
pub mod demo;
pub mod engine;
pub mod eval;
pub mod mail;
pub mod persist;
pub mod pipelines;
pub mod playground;
pub mod store;
pub mod support;
pub mod sysmon;
pub mod termhook;
pub mod web;

mod payload;

#[cfg(test)]
mod inference_tests;
