//! Authoritative email normalization and validation for new owner
//! registration.
//!
//! Validation rules (documented once, enforced here for every caller):
//!
//! 1. Surrounding whitespace is trimmed; the address is Unicode-aware
//!    lowercased (`"USER@SOKET.AI"` and `"user@soket.ai"` are the same
//!    identity).
//! 2. The address must contain **exactly one** `@` separating a local part
//!    and a domain part, both non-empty.
//! 3. Local part: 1–64 ASCII characters from `[a-z0-9._%+-]`. It may not
//!    start or end with `.`, nor contain consecutive dots (`..`).
//! 4. Domain part: must equal **exactly** `soket.ai` (byte equality on the
//!    normalized form — not a suffix or substring match). Lookalikes such as
//!    `soket.ai.example.com`, `example.soket.ai`, `xn--oket-ai-…`, or
//!    `soket.ai.` (trailing dot) are rejected.
//! 5. Total length ≤ 254 characters (RFC 5321 practical limit); control
//!    characters (and any non-ASCII) are rejected outright.
//!
//! This validates *syntax and domain policy only*. Proving that the person
//! controls the mailbox is the OTP challenge's job — never infer ownership
//! from a passing validation.

/// The only domain from which new owner accounts may be registered.
pub const REGISTRATION_DOMAIN: &str = "soket.ai";

/// Maximum total address length (RFC 5321 forward-path bound).
const MAX_EMAIL_LEN: usize = 254;

/// Maximum local-part length (RFC 5321 atom-sequence bound).
const MAX_LOCAL_LEN: usize = 64;

/// Normalize and validate an email address for registration.
///
/// Returns the canonical (trimmed, lowercased) address, or a human-readable
/// reason suitable for a 400 response. The returned string is the single
/// authoritative identity used by the OTP store.
pub fn normalize_registration_email(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("email is required".into());
    }
    // Reject control characters anywhere (including inside the trimmed form)
    // before any other work; they can never appear in a legal address.
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("email contains invalid characters".into());
    }
    let normalized = trimmed.to_lowercase();
    if normalized.len() > MAX_EMAIL_LEN {
        return Err("email is too long".into());
    }

    // Exactly one '@'.
    let mut parts = normalized.splitn(3, '@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some() || !normalized.contains('@') {
        return Err("email must contain exactly one @".into());
    }

    // Local part.
    if local.is_empty() {
        return Err("email local part is required".into());
    }
    if local.len() > MAX_LOCAL_LEN {
        return Err("email local part is too long".into());
    }
    if !local
        .chars()
        .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '%' | '+' | '-'))
    {
        return Err("email local part contains invalid characters".into());
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return Err("email local part has invalid dot placement".into());
    }

    // Domain part: exact match only.
    if domain != REGISTRATION_DOMAIN {
        return Err(format!(
            "registration requires an @{REGISTRATION_DOMAIN} email address"
        ));
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_domain() {
        assert_eq!(
            normalize_registration_email("himanshu@soket.ai").unwrap(),
            "himanshu@soket.ai"
        );
        assert_eq!(
            normalize_registration_email("first.last@soket.ai").unwrap(),
            "first.last@soket.ai"
        );
        assert_eq!(
            normalize_registration_email("a+b_tag%suffix@soket.ai").unwrap(),
            "a+b_tag%suffix@soket.ai"
        );
    }

    #[test]
    fn normalizes_uppercase_and_whitespace() {
        assert_eq!(
            normalize_registration_email("  USER@SOKET.AI \n").unwrap(),
            "user@soket.ai"
        );
        assert_eq!(
            normalize_registration_email("\tMixed.Case@Soket.AI").unwrap(),
            "mixed.case@soket.ai"
        );
    }

    #[test]
    fn rejects_external_domains() {
        for bad in [
            "user@gmail.com",
            "user@outlook.com",
            "user@example.com",
            "user@company.io",
            "@soket.ai",
            "user@",
        ] {
            assert!(
                normalize_registration_email(bad).is_err(),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn rejects_lookalike_domains() {
        for bad in [
            "user@soket.ai.example.com",
            "user@example.soket.ai",
            "user@soket-ai.com",
            "user@soket.ai.", // trailing dot
            "user@so ket.ai", // space inside domain
            "user@SOKET.AI.evil",
            "user@xn--oket-ai-9na.ai",
            "user@soket.ai\u{200b}", // zero-width space (not a control char)
        ] {
            assert!(
                normalize_registration_email(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_addresses() {
        for bad in [
            "",
            "   ",
            "no-at-sign",
            "a@@soket.ai",
            "a@b@soket.ai",
            "@soket.ai",
            ".lead@soket.ai",
            "trail.@soket.ai",
            "double..dot@soket.ai",
            "sp ace@soket.ai",
            "us\u{0007}er@soket.ai", // control character
        ] {
            assert!(
                normalize_registration_email(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_length_limits() {
        // Local part of 65 chars.
        let long_local = "a".repeat(65);
        assert!(normalize_registration_email(&format!("{long_local}@soket.ai")).is_err());
        // 64-char local part is fine.
        let ok_local = "a".repeat(64);
        assert!(normalize_registration_email(&format!("{ok_local}@soket.ai")).is_ok());
        // Total length cap (unreachable via soket.ai + 64 local = 73, so test
        // the cap through the raw guard).
        let huge = format!("{}@soket.ai", "a".repeat(300));
        assert!(normalize_registration_email(&huge).is_err());
    }

    #[test]
    fn unicode_local_parts_are_rejected() {
        // Non-ASCII local parts fail the charset check even though they are
        // technically permitted by some mail systems; we keep the parser
        // strict and ASCII-only.
        assert!(normalize_registration_email("héllo@soket.ai").is_err());
        assert!(normalize_registration_email("用户@soket.ai").is_err());
    }
}
