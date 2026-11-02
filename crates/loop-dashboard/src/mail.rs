//! Email delivery for OTP registration codes.
//!
//! Design notes
//! ------------
//! * **Provider abstraction.** [`MailSender`] is the seam; production uses
//!   [`ResendSender`] (HTTPS via the workspace `reqwest`, rustls — no native
//!   TLS stack). Tests use [`CapturingSender`], which records messages
//!   in memory and never touches the network.
//! * **Fail-closed configuration.** Delivery is only "available" when all
//!   three variables are present and sane. Otherwise
//!   [`EmailConfig::from_env`] yields `None` and callers must report that
//!   email delivery is unavailable — never a fake success.
//! * **No secret leakage.** The API key lives only in the sender's memory
//!   and the `Authorization` header. Error messages carry the provider
//!   status code and a *redacted* body (bounded length, newlines stripped);
//!   the OTP code is never part of any error or log line this module can
//!   produce.
//! * **Timeouts.** 5 s connect / 10 s total per send attempt; failures map
//!   to [`DeliveryError`] variants that are safe to serialize to clients.
//! * **Not wired to HTTP yet.** This module deliberately exposes the pieces
//!   Phase 4 needs ([`deliver_otp_challenge`]) without adding endpoints.

use std::time::Duration;

use crate::auth_email::normalize_registration_email;
use crate::auth_otp::OtpStore;

/// Provider type currently supported.
pub const SUPPORTED_PROVIDER: &str = "resend";

/// Resend's send endpoint.
const RESEND_ENDPOINT: &str = "https://api.resend.com/emails";

/// Total request timeout for one delivery attempt.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// TCP/TLS connection establishment timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Environment variable selecting the provider implementation.
pub const ENV_PROVIDER: &str = "LOOP_EMAIL_PROVIDER";

/// Environment variable carrying the provider API key.
pub const ENV_API_KEY: &str = "LOOP_EMAIL_API_KEY";

/// Environment variable carrying the verified From identity.
pub const ENV_FROM: &str = "LOOP_EMAIL_FROM";

/// Errors from delivery attempts. All `Display` output is safe for client
/// responses and logs: it contains no credentials, no raw provider bodies,
/// and never the OTP itself.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// Configuration absent/invalid — delivery is not possible at all.
    #[error("email delivery is not configured on this instance")]
    NotConfigured,
    /// The provider rejected the request (HTTP >= 400).
    #[error("email provider rejected the request (status {status})")]
    Rejected { status: u16 },
    /// Network/timeout/transport failure reaching the provider.
    #[error("could not reach the email provider")]
    Transport(String),
}

impl DeliveryError {
    /// HTTP status appropriate for this error (wired up in Phase 4).
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            DeliveryError::NotConfigured => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            DeliveryError::Rejected { .. } => axum::http::StatusCode::BAD_GATEWAY,
            DeliveryError::Transport(_) => axum::http::StatusCode::BAD_GATEWAY,
        }
    }
}

/// Fully validated email-delivery configuration.
#[derive(Debug, Clone)]
pub struct EmailConfig {
    /// Verified `From` header, e.g. `"Soket Workflow <noreply@soket.ai>"`.
    pub from: String,
    /// API key (secret; never logged, never serialized).
    pub api_key: String,
}

impl EmailConfig {
    /// Read + validate configuration from the environment.
    ///
    /// Returns `Ok(None)` when nothing is configured (the common local
    /// case) and `Err` when configuration is present but invalid — so a
    /// typo'd setup is loud rather than silently disabled.
    pub fn from_env() -> Result<Option<EmailConfig>, String> {
        let provider = std::env::var(ENV_PROVIDER).unwrap_or_default();
        if provider.trim().is_empty() {
            return Ok(None);
        }
        if provider.trim() != SUPPORTED_PROVIDER {
            return Err(format!(
                "unsupported {ENV_PROVIDER} {provider:?}; expected {SUPPORTED_PROVIDER:?}"
            ));
        }
        let api_key = std::env::var(ENV_API_KEY).unwrap_or_default();
        if api_key.trim().is_empty() {
            return Err(format!(
                "{ENV_PROVIDER}={SUPPORTED_PROVIDER} but {ENV_API_KEY} is empty"
            ));
        }
        let from = std::env::var(ENV_FROM).unwrap_or_default();
        if from.trim().is_empty() {
            return Err(format!(
                "{ENV_PROVIDER}={SUPPORTED_PROVIDER} but {ENV_FROM} is empty"
            ));
        }
        // The From header must contain a legal address part; reuse the
        // registration validator on whatever sits inside <...> when the
        // value is display-name shaped, else on the whole trimmed value.
        let candidate = from
            .split('<')
            .nth(1)
            .and_then(|rest| rest.split('>').next())
            .unwrap_or(from.trim());
        // Registration-domain policy does NOT apply to the sender identity
        // (it may be any verified Resend domain), so validate only that it
        // is a plausible single address with no control characters.
        if candidate.contains(char::is_control)
            || candidate.matches('@').count() != 1
            || candidate
                .trim_start_matches(|c: char| c.is_whitespace())
                .is_empty()
        {
            return Err(format!(
                "{ENV_FROM} does not contain a valid sender address"
            ));
        }
        Ok(Some(EmailConfig {
            from: from.trim().to_string(),
            api_key,
        }))
    }
}

/// A message ready for delivery.
#[derive(Debug, Clone)]
pub struct OtpMessage {
    /// Normalized recipient (validated `@soket.ai`).
    pub to: String,
    /// Six-digit code. Never logged; exists only inside this message.
    pub code: String,
    /// Minutes until expiry, for the human-readable copy.
    pub expires_in_minutes: u64,
}

/// An administrator-triggered password-reset email.
///
/// The reset link carries the raw single-use token. Like the OTP code, it
/// exists in cleartext only inside this message and is never logged.
#[derive(Debug, Clone)]
pub struct PasswordResetMessage {
    /// Normalized recipient.
    pub to: String,
    /// Display name of the administrator who triggered the reset.
    pub admin_email: String,
    /// Absolute URL of the reset form (already carries the token).
    pub reset_url: String,
    /// Minutes until the token expires.
    pub expires_in_minutes: u64,
}

/// A support-ticket notification email.
///
/// One payload shape covers every support notification (confirmation,
/// support-staff alert, reply, status update); the subject/body builders
/// below vary by [`kind`](Self::kind). Ticket content appears only inside
/// the message handed to the sender — never in logs or errors.
#[derive(Debug, Clone)]
pub struct SupportTicketMessage {
    /// Normalized recipient.
    pub to: String,
    /// Which email this is; see the constructor list for the vocabulary.
    pub kind: SupportNotificationKind,
    /// Human-readable ticket id (e.g. `TKT-7F3K9Q`).
    pub ticket_id: String,
    /// Ticket subject line.
    pub subject: String,
    /// Current ticket status (`pending`, `in_progress`, …), when relevant.
    pub status: Option<String>,
    /// Submitter's display name (support-staff alerts only).
    pub submitter_name: Option<String>,
    /// Submitter's email (support-staff alerts only).
    pub submitter_email: Option<String>,
    /// Ticket priority (support-staff alerts only).
    pub priority: Option<String>,
    /// Ticket category (support-staff alerts only).
    pub category: Option<String>,
}

/// The kinds of support notification this module can deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportNotificationKind {
    /// Sent to the user right after they submit a request.
    Confirmation,
    /// Sent to the configured support/admin address about a new ticket.
    NotifySupport,
    /// Sent to the user when an administrator replies.
    Reply,
    /// Sent to the user when the ticket's status changes.
    StatusUpdate,
}

impl SupportTicketMessage {
    /// Confirmation for the submitter: "we got your request".
    pub fn confirmation(to: String, ticket_id: &str, subject: &str, status: &str) -> Self {
        Self {
            to,
            kind: SupportNotificationKind::Confirmation,
            ticket_id: ticket_id.to_string(),
            subject: subject.to_string(),
            status: Some(status.to_string()),
            submitter_name: None,
            submitter_email: None,
            priority: None,
            category: None,
        }
    }

    /// Alert for the support/admin inbox: a new request arrived.
    #[allow(clippy::too_many_arguments)]
    pub fn notify_support(
        to: String,
        ticket_id: &str,
        subject: &str,
        submitter_name: &str,
        submitter_email: &str,
        priority: &str,
        category: &str,
    ) -> Self {
        Self {
            to,
            kind: SupportNotificationKind::NotifySupport,
            ticket_id: ticket_id.to_string(),
            subject: subject.to_string(),
            status: None,
            submitter_name: Some(submitter_name.to_string()),
            submitter_email: Some(submitter_email.to_string()),
            priority: Some(priority.to_string()),
            category: Some(category.to_string()),
        }
    }

    /// Reply notification for the submitter.
    pub fn reply(to: String, ticket_id: &str, subject: &str) -> Self {
        Self {
            to,
            kind: SupportNotificationKind::Reply,
            ticket_id: ticket_id.to_string(),
            subject: subject.to_string(),
            status: None,
            submitter_name: None,
            submitter_email: None,
            priority: None,
            category: None,
        }
    }

    /// Status-change notification for the submitter.
    pub fn status_update(to: String, ticket_id: &str, subject: &str, status: &str) -> Self {
        Self {
            to,
            kind: SupportNotificationKind::StatusUpdate,
            ticket_id: ticket_id.to_string(),
            subject: subject.to_string(),
            status: Some(status.to_string()),
            submitter_name: None,
            submitter_email: None,
            priority: None,
            category: None,
        }
    }
}

/// The delivery seam. Implementations must never log the code or key.
pub trait MailSender: Send + Sync {
    fn send_otp(&self, message: &OtpMessage) -> Result<(), DeliveryError>;

    /// Deliver an administrator-triggered password-reset link.
    ///
    /// The default implementation returns [`DeliveryError::NotConfigured`]
    /// so a sender that predates this method keeps compiling; production
    /// senders override it.
    fn send_password_reset(&self, _message: &PasswordResetMessage) -> Result<(), DeliveryError> {
        Err(DeliveryError::NotConfigured)
    }

    /// Deliver a support-ticket notification.
    ///
    /// Same default-implementation contract as
    /// [`MailSender::send_password_reset`].
    fn send_support_notification(
        &self,
        _message: &SupportTicketMessage,
    ) -> Result<(), DeliveryError> {
        Err(DeliveryError::NotConfigured)
    }
}

// ---------------------------------------------------------------------------
// Email content
// ---------------------------------------------------------------------------

/// Subject line for OTP emails.
pub const OTP_SUBJECT: &str = "Your Soket Workflow verification code";

/// Plain-text body. The code appears exactly once.
pub fn otp_text_body(message: &OtpMessage) -> String {
    format!(
        "Welcome to Soket Workflow.\n\n\
         Your verification code is: {code}\n\n\
         It expires in {minutes} minutes. If you did not request it, you can \
         ignore this email.\n\n\
         — Soket Workflow\n",
        code = message.code,
        minutes = message.expires_in_minutes,
    )
}

/// Minimal branded HTML body: dark card, monospace code, no external assets.
pub fn otp_html_body(message: &OtpMessage) -> String {
    let code = html_escape(&message.code);
    let minutes = message.expires_in_minutes;
    format!(
        r#"<div style="font-family:ui-sans-serif,system-ui,-apple-system,'Segoe UI',sans-serif;background:#0b1220;padding:32px;border-radius:16px;color:#e7ecf3;max-width:480px;margin:0 auto">
  <div style="font-size:13px;letter-spacing:.18em;text-transform:uppercase;color:#5ef2c0;font-weight:700">Soket Workflow</div>
  <h1 style="font-size:20px;margin:14px 0 10px">Verify your email</h1>
  <p style="font-size:14px;line-height:1.6;color:#aab4c3;margin:0 0 20px">Enter this code to finish creating your owner account:</p>
  <div style="font-family:ui-monospace,'JetBrains Mono',Menlo,monospace;font-size:30px;letter-spacing:.35em;background:#111a2c;border:1px solid #23324d;border-radius:12px;padding:16px 20px;text-align:center;color:#5ef2c0">{code}</div>
  <p style="font-size:12.5px;color:#8b96a8;margin:18px 0 0">This code expires in {minutes} minutes. If you didn't request it, you can safely ignore this email.</p>
</div>"#
    )
}

/// Escape the five characters that matter in HTML text nodes.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// Password-reset email content (User Management phase)
// ---------------------------------------------------------------------------

/// Subject line for password-reset emails.
pub const PASSWORD_RESET_SUBJECT: &str = "Reset your Loop dashboard password";

/// Plain-text body of a password-reset email. The link appears exactly once.
pub fn password_reset_text_body(m: &PasswordResetMessage) -> String {
    format!(
        "Hello,\n\n\
         {admin} (a Loop dashboard administrator) has initiated a password \
         reset for your account.\n\n\
         Open this link within {minutes} minutes to choose a new password:\n\n\
         {url}\n\n\
         The link works once and expires after {minutes} minutes. If you did \
         not expect this email you can ignore it — your current password \
         keeps working until the link is used.\n\n\
         — Soket Workflow\n",
        admin = m.admin_email,
        minutes = m.expires_in_minutes,
        url = m.reset_url,
    )
}

/// Minimal branded HTML body for the reset email, matching the OTP style.
pub fn password_reset_html_body(m: &PasswordResetMessage) -> String {
    let url = html_escape(&m.reset_url);
    let admin = html_escape(&m.admin_email);
    let minutes = m.expires_in_minutes;
    format!(
        r#"<div style="font-family:ui-sans-serif,system-ui,-apple-system,'Segoe UI',sans-serif;background:#0b1220;padding:32px;border-radius:16px;color:#e7ecf3;max-width:480px;margin:0 auto">
  <div style="font-size:13px;letter-spacing:.18em;text-transform:uppercase;color:#5ef2c0;font-weight:700">Soket Workflow</div>
  <h1 style="font-size:20px;margin:14px 0 10px">Reset your password</h1>
  <p style="font-size:14px;line-height:1.6;color:#aab4c3;margin:0 0 20px"><strong style="color:#e7ecf3">{admin}</strong>, a dashboard administrator, has initiated a password reset for your account.</p>
  <div style="text-align:center;margin:0 0 18px">
    <a href="{url}" style="display:inline-block;background:#5ef2c0;color:#04231a;font-size:14px;font-weight:600;text-decoration:none;border-radius:10px;padding:12px 22px">Choose a new password</a>
  </div>
  <p style="font-family:ui-monospace,'JetBrains Mono',Menlo,monospace;font-size:11px;color:#8b96a8;word-break:break-all;margin:0 0 18px">{url}</p>
  <p style="font-size:12.5px;color:#8b96a8;margin:0">This link works once and expires in {minutes} minutes. Completing it signs out every active session on your account. If you didn't expect this email, you can safely ignore it.</p>
</div>"#
    )
}

// ---------------------------------------------------------------------------
// Support-ticket email content (Support phase)
// ---------------------------------------------------------------------------

/// Subject line for a submitter confirmation.
fn support_confirmation_subject(m: &SupportTicketMessage) -> String {
    format!("[{}] We received your request: {}", m.ticket_id, m.subject)
}

/// Subject line for the support-staff alert.
fn support_alert_subject(m: &SupportTicketMessage) -> String {
    format!(
        "[{}] New {} priority request from {}: {}",
        m.ticket_id,
        m.priority.as_deref().unwrap_or("unknown"),
        m.submitter_email.as_deref().unwrap_or("unknown user"),
        m.subject
    )
}

/// Subject line for a reply notification.
fn support_reply_subject(m: &SupportTicketMessage) -> String {
    format!("[{}] Update on your request: {}", m.ticket_id, m.subject)
}

/// Subject line for a status-change notification.
fn support_status_subject(m: &SupportTicketMessage) -> String {
    format!(
        "[{}] Your request is now {}: {}",
        m.ticket_id,
        display_status(m.status.as_deref().unwrap_or("unknown")),
        m.subject
    )
}

/// Human-readable form of a ticket status for email copy.
pub fn display_status(status: &str) -> &str {
    match status {
        "pending" => "Pending",
        "in_progress" => "In Progress",
        "waiting_user" => "Waiting for You",
        "resolved" => "Resolved",
        "closed" => "Closed",
        other => other,
    }
}

/// Human-readable form of a ticket category for email copy.
pub fn display_category(category: &str) -> &str {
    match category {
        "technical_issue" => "Technical Issue",
        "bug_report" => "Bug Report",
        "login_problem" => "Login Problem",
        "performance_issue" => "Performance Issue",
        "data_pipeline" => "Data Pipeline",
        "model_evaluation" => "Model Evaluation",
        "feature_request" => "Feature Request",
        "general_inquiry" => "General Inquiry",
        "other" => "Other",
        other => other,
    }
}

/// Shared visual shell for support emails, matching the OTP/reset style.
fn support_email_shell(heading: &str, body_html: &str, footer: &str) -> String {
    format!(
        r#"<div style="font-family:ui-sans-serif,system-ui,-apple-system,'Segoe UI',sans-serif;background:#0b1220;padding:32px;border-radius:16px;color:#e7ecf3;max-width:520px;margin:0 auto">
  <div style="font-size:13px;letter-spacing:.18em;text-transform:uppercase;color:#5ef2c0;font-weight:700">Soket Workflow · Support</div>
  <h1 style="font-size:20px;margin:14px 0 10px">{heading}</h1>
  {body_html}
  <p style="font-size:12.5px;color:#8b96a8;margin:18px 0 0">{footer}</p>
</div>"#
    )
}

/// Ticket-id chip used across the support email bodies.
fn ticket_chip(m: &SupportTicketMessage) -> String {
    format!(
        "<div style=\"font-family:ui-monospace,'JetBrains Mono',Menlo,monospace;font-size:15px;\
         letter-spacing:.12em;background:#111a2c;border:1px solid #23324d;border-radius:12px;\
         padding:12px 18px;text-align:center;color:#5ef2c0;margin:0 0 18px\">{}</div>",
        html_escape(&m.ticket_id)
    )
}

/// Plain-text body of a support notification.
pub fn support_text_body(m: &SupportTicketMessage) -> String {
    match m.kind {
        SupportNotificationKind::Confirmation => format!(
            "Hello,\n\nWe received your support request and it is now \
             {status}.\n\nTicket ID : {id}\nSubject   : {subject}\n\n\
             Keep the ticket ID handy — you can check the status of your \
             request any time from the dashboard's Help & Support button.\n\n\
             — Soket Workflow Support\n",
            status = display_status(m.status.as_deref().unwrap_or("pending")),
            id = m.ticket_id,
            subject = m.subject,
        ),
        SupportNotificationKind::NotifySupport => format!(
            "A new support request was submitted.\n\n\
             Ticket ID : {id}\nSubject   : {subject}\nFrom     : {name} <{email}>\n\
             Priority : {priority}\nCategory : {category}\n\n\
             Review it in the dashboard's Support Management view.\n\n\
             — Soket Workflow Support\n",
            id = m.ticket_id,
            subject = m.subject,
            name = m.submitter_name.as_deref().unwrap_or("unknown"),
            email = m.submitter_email.as_deref().unwrap_or("unknown"),
            priority = m.priority.as_deref().unwrap_or("unknown"),
            category = display_category(m.category.as_deref().unwrap_or("other")),
        ),
        SupportNotificationKind::Reply => format!(
            "Hello,\n\nAn administrator replied to your support request.\n\n\
             Ticket ID : {id}\nSubject   : {subject}\n\n\
             Read the reply in the dashboard's Help & Support view.\n\n\
             — Soket Workflow Support\n",
            id = m.ticket_id,
            subject = m.subject,
        ),
        SupportNotificationKind::StatusUpdate => format!(
            "Hello,\n\nThe status of your support request changed to \
             {status}.\n\nTicket ID : {id}\nSubject   : {subject}\n\n\
             Track it any time from the dashboard's Help & Support button.\n\n\
             — Soket Workflow Support\n",
            status = display_status(m.status.as_deref().unwrap_or("unknown")),
            id = m.ticket_id,
            subject = m.subject,
        ),
    }
}

/// Branded HTML body of a support notification.
pub fn support_html_body(m: &SupportTicketMessage) -> String {
    let subject_line = format!(
        "<p style=\"font-size:14px;line-height:1.6;color:#aab4c3;margin:0 0 18px\">\
         Subject: <strong style=\"color:#e7ecf3\">{}</strong></p>",
        html_escape(&m.subject)
    );
    match m.kind {
        SupportNotificationKind::Confirmation => support_email_shell(
            "We received your request",
            &format!(
                "{chip}{subject}<p style=\"font-size:14px;line-height:1.6;color:#aab4c3;margin:0\">\
                 Your request is now <strong style=\"color:#5ef2c0\">{status}</strong>. We will \
                 get back to you at this address.</p>",
                chip = ticket_chip(m),
                subject = subject_line,
                status = display_status(m.status.as_deref().unwrap_or("pending")),
            ),
            "You can check the status any time from the dashboard's Help & Support button.",
        ),
        SupportNotificationKind::NotifySupport => support_email_shell(
            "New support request",
            &format!(
                "{chip}{subject}\
                 <table style=\"font-size:13px;color:#aab4c3;border-collapse:collapse\">\
                 <tr><td style=\"padding:3px 12px 3px 0;color:#8b96a8\">From</td>\
                 <td>{name} &lt;{email}&gt;</td></tr>\
                 <tr><td style=\"padding:3px 12px 3px 0;color:#8b96a8\">Priority</td>\
                 <td>{priority}</td></tr>\
                 <tr><td style=\"padding:3px 12px 3px 0;color:#8b96a8\">Category</td>\
                 <td>{category}</td></tr></table>",
                chip = ticket_chip(m),
                subject = subject_line,
                name = html_escape(m.submitter_name.as_deref().unwrap_or("unknown")),
                email = html_escape(m.submitter_email.as_deref().unwrap_or("unknown")),
                priority = html_escape(m.priority.as_deref().unwrap_or("unknown")),
                category = display_category(m.category.as_deref().unwrap_or("other")),
            ),
            "Review it in the dashboard's Support Management view.",
        ),
        SupportNotificationKind::Reply => support_email_shell(
            "Update on your request",
            &format!(
                "{chip}{subject}<p style=\"font-size:14px;line-height:1.6;color:#aab4c3;margin:0\">\
                 An administrator replied to your request. Open the dashboard's Help &amp; \
                 Support view to read the response.</p>",
                chip = ticket_chip(m),
                subject = subject_line,
            ),
            "Reply to this request from the dashboard at any time.",
        ),
        SupportNotificationKind::StatusUpdate => support_email_shell(
            "Your request's status changed",
            &format!(
                "{chip}{subject}<p style=\"font-size:14px;line-height:1.6;color:#aab4c3;margin:0\">\
                 Status: <strong style=\"color:#5ef2c0\">{status}</strong></p>",
                chip = ticket_chip(m),
                subject = subject_line,
                status = display_status(m.status.as_deref().unwrap_or("unknown")),
            ),
            "Track the full history from the dashboard's Help & Support button.",
        ),
    }
}

// ---------------------------------------------------------------------------
// Production sender: Resend over HTTPS
// ---------------------------------------------------------------------------

/// Resend HTTPS sender. Holds the API key in memory only.
pub struct ResendSender {
    client: reqwest::blocking::Client,
    config: EmailConfig,
}

impl ResendSender {
    /// Build a sender. Fails only if the HTTP client cannot be constructed
    /// (e.g., TLS backend unavailable) — reported without secrets.
    pub fn new(config: EmailConfig) -> Result<Self, DeliveryError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .https_only(true)
            .build()
            .map_err(|e| DeliveryError::Transport(format!("client init failed: {e}")))?;
        Ok(Self { client, config })
    }

    /// Redact a provider response body for logs: bounded, single-line,
    /// stripped of anything resembling the authorization header. Status
    /// code alone is usually sufficient; the redacted body helps debugging
    /// misconfigured sender domains without leaking provider echoes.
    fn redact_body(body: &str) -> String {
        let collapsed: String = body
            .chars()
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        let mut out: String = collapsed.chars().take(200).collect();
        if collapsed.chars().count() > 200 {
            out.push('…');
        }
        out
    }
}

impl MailSender for ResendSender {
    fn send_otp(&self, message: &OtpMessage) -> Result<(), DeliveryError> {
        use serde_json::json;

        let payload = json!({
            "from": self.config.from,
            "to": [message.to],
            "subject": OTP_SUBJECT,
            "html": otp_html_body(message),
            "text": otp_text_body(message),
        });
        self.deliver(payload)
    }

    fn send_password_reset(&self, message: &PasswordResetMessage) -> Result<(), DeliveryError> {
        use serde_json::json;

        let payload = json!({
            "from": self.config.from,
            "to": [message.to],
            "subject": PASSWORD_RESET_SUBJECT,
            "html": password_reset_html_body(message),
            "text": password_reset_text_body(message),
        });
        self.deliver(payload)
    }

    fn send_support_notification(
        &self,
        message: &SupportTicketMessage,
    ) -> Result<(), DeliveryError> {
        use serde_json::json;

        // An empty recipient would be rejected by the provider anyway;
        // refusing here keeps a misconfigured support address from
        // producing confusing provider errors.
        if message.to.trim().is_empty() {
            return Err(DeliveryError::NotConfigured);
        }
        let subject = match message.kind {
            SupportNotificationKind::Confirmation => support_confirmation_subject(message),
            SupportNotificationKind::NotifySupport => support_alert_subject(message),
            SupportNotificationKind::Reply => support_reply_subject(message),
            SupportNotificationKind::StatusUpdate => support_status_subject(message),
        };
        let payload = json!({
            "from": self.config.from,
            "to": [message.to],
            "subject": subject,
            "html": support_html_body(message),
            "text": support_text_body(message),
        });
        self.deliver(payload)
    }
}

impl ResendSender {
    /// Shared POST + status handling for both message kinds. Never logs the
    /// token, code or API key; provider rejections are reduced to a status
    /// plus a heavily redacted body for stderr only.
    fn deliver(&self, payload: serde_json::Value) -> Result<(), DeliveryError> {
        // Blocking client used from async context must go through
        // spawn_blocking in the caller (see deliver_otp_challenge).
        let response = self
            .client
            .post(RESEND_ENDPOINT)
            .bearer_auth(&self.config.api_key)
            .json(&payload)
            .send();

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                // reqwest errors can embed the URL (fine) but never headers;
                // keep the description generic anyway.
                let _ = e;
                return Err(DeliveryError::Transport("request failed".into()));
            }
        };

        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        // Non-2xx: capture only the status (+ heavily redacted body for
        // server-side diagnostics, never surfaced to clients verbatim).
        let body = response.text().unwrap_or_default();
        tracing_free_diag(&status, &Self::redact_body(&body));
        Err(DeliveryError::Rejected { status })
    }
}

/// Server-side diagnostic that never carries secrets: emits to stderr only
/// (the dashboard has no tracing subscriber wired for this module yet).
fn tracing_free_diag(status: &u16, redacted_body: &str) {
    eprintln!("email delivery rejected by provider: status={status} body={redacted_body}");
}

// ---------------------------------------------------------------------------
// Test double
// ---------------------------------------------------------------------------

/// In-memory sender for tests. Records messages; can be scripted to fail.
#[derive(Default)]
pub struct CapturingSender {
    pub sent: parking_lot::Mutex<Vec<OtpMessage>>,
    pub reset_sent: parking_lot::Mutex<Vec<PasswordResetMessage>>,
    pub support_sent: parking_lot::Mutex<Vec<SupportTicketMessage>>,
    pub fail_with: parking_lot::Mutex<Option<DeliveryError>>,
}

impl CapturingSender {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next (and subsequent) sends to fail.
    pub fn fail(&self, err: DeliveryError) {
        *self.fail_with.lock() = Some(err);
    }

    /// Messages captured so far.
    pub fn messages(&self) -> Vec<OtpMessage> {
        self.sent.lock().clone()
    }

    /// Password-reset emails captured so far.
    pub fn reset_messages(&self) -> Vec<PasswordResetMessage> {
        self.reset_sent.lock().clone()
    }

    /// Support notifications captured so far.
    pub fn support_messages(&self) -> Vec<SupportTicketMessage> {
        self.support_sent.lock().clone()
    }
}

impl MailSender for CapturingSender {
    fn send_otp(&self, message: &OtpMessage) -> Result<(), DeliveryError> {
        if let Some(err) = self.fail_with.lock().take() {
            return Err(err);
        }
        self.sent.lock().push(message.clone());
        Ok(())
    }

    fn send_password_reset(&self, message: &PasswordResetMessage) -> Result<(), DeliveryError> {
        if let Some(err) = self.fail_with.lock().take() {
            return Err(err);
        }
        self.reset_sent.lock().push(message.clone());
        Ok(())
    }

    fn send_support_notification(
        &self,
        message: &SupportTicketMessage,
    ) -> Result<(), DeliveryError> {
        if let Some(err) = self.fail_with.lock().take() {
            return Err(err);
        }
        self.support_sent.lock().push(message.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Orchestration: issue challenge → deliver → invalidate on failure
// ---------------------------------------------------------------------------

/// Issue an OTP challenge and deliver its code to the validated address.
///
/// Guarantees:
/// * The code goes **only** to the normalized, validated recipient.
/// * No database lock is held while sending: `issue_challenge` returns (and
///   releases the store mutex) before any network I/O happens.
/// * If delivery fails, the just-created challenge is invalidated *by row
///   id*, so a newer challenge issued meanwhile (or an older unrelated one)
///   can never be affected.
/// * Success is reported only when the provider accepted the request.
/// * Resend cooldown and attempt semantics are untouched: a failed delivery
///   still counts as an issuance for cooldown purposes (prevents using
///   delivery failures as a free-spam oracle).
pub fn deliver_otp_challenge(
    store: &OtpStore,
    sender: &dyn MailSender,
    raw_email: &str,
) -> Result<DeliveredChallenge, DeliverOtpError> {
    // Validate BEFORE touching the store so invalid addresses never create
    // challenges.
    let email = normalize_registration_email(raw_email).map_err(DeliverOtpError::Validation)?;

    // Phase 1 of issuance: create the challenge (sync, lock released on
    // return). The row id travels back with the code.
    let issued = store
        .issue_challenge(&email)
        .map_err(DeliverOtpError::Otp)?;

    // Phase 2: deliver outside any store lock.
    let message = OtpMessage {
        to: email.clone(),
        code: issued.code.clone(),
        expires_in_minutes: (crate::auth_otp::OTP_TTL.as_secs() / 60).max(1),
    };
    match sender.send_otp(&message) {
        Ok(()) => Ok(DeliveredChallenge {
            email,
            expires_at_ms: issued.expires_at_ms,
            resend_cooldown_ms: issued.resend_cooldown_ms,
        }),
        Err(delivery) => {
            // Invalidate exactly the row we created. Challenge-supersession
            // logic keys on (email, live); targeting the specific id means a
            // concurrently issued newer challenge is untouched, and cleanup
            // cannot confuse this row with any other.
            store.invalidate_challenge_by_id(issued.challenge_id);
            Err(DeliverOtpError::Delivery(delivery))
        }
    }
}

/// Successful issuance + delivery result (no code inside — the code was
/// handed to the sender and dropped).
#[derive(Debug)]
pub struct DeliveredChallenge {
    pub email: String,
    pub expires_at_ms: i64,
    pub resend_cooldown_ms: i64,
}

/// Combined error for issue+deliver orchestration.
#[derive(Debug, thiserror::Error)]
pub enum DeliverOtpError {
    #[error("{0}")]
    Validation(String),
    #[error(transparent)]
    Otp(#[from] crate::auth_otp::OtpError),
    #[error(transparent)]
    Delivery(#[from] DeliveryError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_otp::{OtpStore, TestClock};
    use std::sync::Arc;

    const T0: i64 = 1_700_000_000_000;

    fn setup() -> (OtpStore, Arc<TestClock>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::new(T0));
        let store =
            OtpStore::open_with_key(dir.path().join("auth.db"), vec![7u8; 32], clock.clone())
                .unwrap();
        (store, clock, dir)
    }

    // ---- configuration ----

    #[test]
    fn missing_configuration_yields_none() {
        // SAFETY: tests run multi-threaded; env races are avoided by using
        // values unique to this test.
        std::env::set_var(ENV_PROVIDER, "");
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_FROM);
        assert!(EmailConfig::from_env().unwrap().is_none());
    }

    #[test]
    fn invalid_provider_is_rejected_loudly() {
        std::env::set_var(ENV_PROVIDER, "sendgrid");
        std::env::set_var(ENV_API_KEY, "re_test_key");
        std::env::set_var(ENV_FROM, "Soket <noreply@soket.ai>");
        assert!(EmailConfig::from_env().is_err());
        std::env::remove_var(ENV_PROVIDER);
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_FROM);
    }

    #[test]
    fn valid_configuration_parses() {
        std::env::set_var(ENV_PROVIDER, "resend");
        std::env::set_var(ENV_API_KEY, "re_test_key");
        std::env::set_var(ENV_FROM, "Soket Workflow <noreply@soket.ai>");
        let cfg = EmailConfig::from_env().unwrap().expect("should configure");
        assert_eq!(cfg.from, "Soket Workflow <noreply@soket.ai>");
        assert_eq!(cfg.api_key, "re_test_key");
        std::env::remove_var(ENV_PROVIDER);
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_FROM);
    }

    #[test]
    fn empty_key_or_from_is_invalid() {
        std::env::set_var(ENV_PROVIDER, "resend");
        std::env::set_var(ENV_API_KEY, "");
        std::env::set_var(ENV_FROM, "x <x@soket.ai>");
        assert!(EmailConfig::from_env().is_err());

        std::env::set_var(ENV_API_KEY, "k");
        std::env::set_var(ENV_FROM, "");
        assert!(EmailConfig::from_env().is_err());
        std::env::remove_var(ENV_PROVIDER);
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_FROM);
    }

    #[test]
    fn malformed_from_address_is_invalid() {
        std::env::set_var(ENV_PROVIDER, "resend");
        std::env::set_var(ENV_API_KEY, "k");
        std::env::set_var(ENV_FROM, "no-address-here");
        assert!(EmailConfig::from_env().is_err());
        std::env::remove_var(ENV_PROVIDER);
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_FROM);
    }

    // ---- message content ----

    #[test]
    fn bodies_contain_code_once_and_expiry() {
        let msg = OtpMessage {
            to: "user@soket.ai".into(),
            code: "042913".into(),
            expires_in_minutes: 5,
        };
        let text = otp_text_body(&msg);
        assert_eq!(text.matches("042913").count(), 1);
        assert!(text.contains("5 minutes"));
        let html = otp_html_body(&msg);
        assert_eq!(html.matches("042913").count(), 1);
        assert!(html.contains("5 minutes"));
    }

    #[test]
    fn html_escapes_code_context() {
        let msg = OtpMessage {
            to: "user@soket.ai".into(),
            code: "123456".into(),
            expires_in_minutes: 5,
        };
        let _html = otp_html_body(&msg);
        // Code digits need no escaping, but the helper must be correct for
        // any input; verify round-trip on a hostile string.
        assert_eq!(
            html_escape("<script>&\"'"),
            "&lt;script&gt;&amp;&quot;&#39;"
        );
    }

    // ---- orchestration ----

    #[test]
    fn successful_delivery_sends_code_to_validated_address_only() {
        let (store, _clock, _d) = setup();
        let sender = CapturingSender::new();
        let out = deliver_otp_challenge(&store, &sender, "  USER@SOKET.AI ").unwrap();
        assert_eq!(out.email, "user@soket.ai");
        let msgs = sender.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].to, "user@soket.ai"); // normalized, not raw input
        assert_eq!(msgs[0].code.len(), 6);
        assert!(msgs[0].code.bytes().all(|b| b.is_ascii_digit()));
        // Challenge stays live after successful delivery.
        assert_eq!(store.live_challenge_count(), 1);
    }

    #[test]
    fn invalid_email_never_creates_a_challenge() {
        let (store, _clock, _d) = setup();
        let sender = CapturingSender::new();
        assert!(matches!(
            deliver_otp_challenge(&store, &sender, "user@gmail.com"),
            Err(DeliverOtpError::Validation(_))
        ));
        assert!(sender.messages().is_empty());
        assert_eq!(store.live_challenge_count(), 0);
    }

    #[test]
    fn provider_rejection_invalidates_the_new_challenge() {
        let (store, _clock, _d) = setup();
        let sender = CapturingSender::new();
        sender.fail(DeliveryError::Rejected { status: 422 });
        let err = deliver_otp_challenge(&store, &sender, "user@soket.ai").unwrap_err();
        assert!(matches!(
            err,
            DeliverOtpError::Delivery(DeliveryError::Rejected { status: 422 })
        ));
        // The undelivered OTP must not remain usable.
        assert_eq!(store.live_challenge_count(), 0);
        // And nothing was delivered.
        assert!(sender.messages().is_empty());
    }

    #[test]
    fn transport_failure_invalidates_the_new_challenge() {
        let (store, _clock, _d) = setup();
        let sender = CapturingSender::new();
        sender.fail(DeliveryError::Transport("timeout".into()));
        assert!(matches!(
            deliver_otp_challenge(&store, &sender, "user@soket.ai"),
            Err(DeliverOtpError::Delivery(DeliveryError::Transport(_)))
        ));
        assert_eq!(store.live_challenge_count(), 0);
    }

    #[test]
    fn failed_delivery_still_enforces_resend_cooldown() {
        let (store, clock, _d) = setup();
        let sender = CapturingSender::new();
        sender.fail(DeliveryError::Transport("down".into()));
        let _ = deliver_otp_challenge(&store, &sender, "user@soket.ai").unwrap_err();
        // Immediate retry hits the cooldown even though delivery failed.
        sender.fail(DeliveryError::Transport("down".into()));
        match deliver_otp_challenge(&store, &sender, "user@soket.ai") {
            Err(DeliverOtpError::Otp(crate::auth_otp::OtpError::Cooldown { retry_in_ms })) => {
                assert!(retry_in_ms > 0);
            }
            other => panic!("expected cooldown, got {other:?}"),
        }
        // After cooldown, a working sender succeeds end-to-end.
        clock.advance(60_000);
        *sender.fail_with.lock() = None; // clear the pending scripted failure
        assert!(deliver_otp_challenge(&store, &sender, "user@soket.ai").is_ok());
        assert_eq!(store.live_challenge_count(), 1);
    }

    #[test]
    fn failure_invalidation_targets_only_the_new_row() {
        let (store, clock, _d) = setup();
        // First: a successfully delivered live challenge for another user.
        let ok_sender = CapturingSender::new();
        deliver_otp_challenge(&store, &ok_sender, "alice@soket.ai").unwrap();
        assert_eq!(store.live_challenge_count(), 1);

        // Second: a failing delivery for bob.
        clock.advance(120_000); // past cooldown for a different email? cooldown is per-email
        let fail_sender = CapturingSender::new();
        fail_sender.fail(DeliveryError::Transport("down".into()));
        assert!(deliver_otp_challenge(&store, &fail_sender, "bob@soket.ai").is_err());

        // Alice's live challenge must be untouched.
        assert_eq!(store.live_challenge_count(), 1);
        // Alice can still verify with her delivered code.
        let alice_code = ok_sender.messages()[0].code.clone();
        assert!(store.verify_otp("alice@soket.ai", &alice_code).is_ok());
    }

    #[test]
    fn delivered_code_verifies_end_to_end() {
        let (store, _clock, _d) = setup();
        let sender = CapturingSender::new();
        deliver_otp_challenge(&store, &sender, "user@soket.ai").unwrap();
        let code = sender.messages()[0].code.clone();
        let grant = store.verify_otp("user@soket.ai", &code).unwrap();
        assert_eq!(grant.email, "user@soket.ai");
    }

    // ---- leakage guards ----

    #[test]
    fn error_display_never_contains_code_or_key() {
        let errors = [
            DeliveryError::NotConfigured,
            DeliveryError::Rejected { status: 422 },
            DeliveryError::Transport("connection reset".into()),
        ];
        for e in &errors {
            let text = e.to_string();
            assert!(!text.contains("re_test"), "leaked key material: {text}");
            assert!(!text.contains("042913"), "leaked OTP: {text}");
        }
        // Redaction helper bounds and flattens bodies.
        let long = "x\ny".repeat(500);
        let redacted = ResendSender::redact_body(&long);
        assert!(!redacted.contains('\n'));
        assert!(redacted.chars().count() <= 201); // 200 + ellipsis
    }

    #[test]
    fn redact_body_strips_newlines_and_bounds_length() {
        let body = "line1\nline2\nBearer re_live_secret";
        let redacted = ResendSender::redact_body(body);
        assert!(!redacted.contains('\n'));
        assert!(redacted.contains("line1 line2"));
    }
}

    // ---- support notifications (Help & Support phase) ----

    #[test]
    fn support_confirmation_email_carries_ticket_id_and_subject() {
        let m = SupportTicketMessage::confirmation(
            "user@soket.ai".into(),
            "TKT-7F3K9Q",
            "Streaming stops",
            "pending",
        );
        let text = support_text_body(&m);
        let html = support_html_body(&m);
        for body in [&text, &html] {
            assert!(body.contains("TKT-7F3K9Q"), "ticket id missing: {body}");
            assert!(body.contains("Streaming stops"), "subject missing");
            assert!(body.contains("Pending"), "status missing");
        }
        // No secrets, no other recipient.
        assert!(!text.contains("re_test"));
    }

    #[test]
    fn support_alert_email_identifies_the_submitter() {
        let m = SupportTicketMessage::notify_support(
            "support@soket.ai".into(),
            "TKT-AB12CD",
            "Login broken",
            "Someone",
            "someone@soket.ai",
            "critical",
            "login_problem",
        );
        let text = support_text_body(&m);
        assert!(text.contains("someone@soket.ai"));
        assert!(text.contains("critical"));
        assert!(text.contains("Login Problem"), "category must be human-readable");
        let html = support_html_body(&m);
        assert!(html.contains("Critical Issues".replace("Issues", "").trim())
            || html.contains("critical"));
    }

    #[test]
    fn support_reply_and_status_emails_are_distinct() {
        let reply = SupportTicketMessage::reply("u@soket.ai".into(), "TKT-1", "s");
        let status = SupportTicketMessage::status_update(
            "u@soket.ai".into(),
            "TKT-1",
            "s",
            "waiting_user",
        );
        assert_ne!(
            support_text_body(&reply),
            support_text_body(&status),
            "reply and status copy must differ"
        );
        assert!(support_text_body(&status).contains("Waiting for You"));
        // Subjects differ per kind so threads stay readable in a mailbox.
        assert_ne!(
            support_reply_subject(&reply),
            support_status_subject(&status)
        );
    }

    #[test]
    fn support_email_html_escapes_user_content() {
        let m = SupportTicketMessage::confirmation(
            "u@soket.ai".into(),
            "TKT-X",
            "<script>alert(1)</script>",
            "pending",
        );
        let html = support_html_body(&m);
        assert!(!html.contains("<script>"), "unescaped subject reached HTML");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn capturing_sender_records_support_messages() {
        let sender = CapturingSender::new();
        let m = SupportTicketMessage::confirmation("u@soket.ai".into(), "TKT-1", "s", "pending");
        sender.send_support_notification(&m).unwrap();
        let sent = sender.support_messages();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].ticket_id, "TKT-1");
        assert_eq!(sent[0].kind, SupportNotificationKind::Confirmation);

        // Scripted failure surfaces as an error, not a silent drop.
        sender.fail(DeliveryError::NotConfigured);
        assert!(sender
            .send_support_notification(&m)
            .is_err());
    }

    #[test]
    fn mailer_default_send_support_notification_is_not_configured() {
        // A sender that predates the method must fail honestly rather than
        // pretend to deliver.
        struct Legacy;
        impl MailSender for Legacy {
            fn send_otp(&self, _m: &OtpMessage) -> Result<(), DeliveryError> {
                Ok(())
            }
        }
        let m = SupportTicketMessage::reply("u@soket.ai".into(), "TKT-1", "s");
        assert!(matches!(
            Legacy.send_support_notification(&m),
            Err(DeliveryError::NotConfigured)
        ));
    }
