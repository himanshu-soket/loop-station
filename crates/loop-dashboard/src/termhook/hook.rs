//! Zsh hook library and its installation into the user's shell.
//!
//! The library itself is [`HOOK_LIBRARY`] (embedded verbatim from
//! `hook.zsh`). Installation writes it to
//! `~/.loop/agent/hooks/loop-terminal.zsh` and appends a managed source
//! block to `~/.zshrc` between stable markers, so re-running install is
//! idempotent and uninstall removes exactly what Loop added.

use std::path::PathBuf;

/// The complete Zsh hook library, embedded at compile time.
pub const HOOK_LIBRARY: &str = include_str!("hook.zsh");

/// Managed-block start marker in `~/.zshrc`.
const MARKER_BEGIN: &str =
    "# >>> loop terminal observability (managed by `loop install-terminal`) >>>";
/// Managed-block end marker.
const MARKER_END: &str = "# <<< loop terminal observability <<<";

/// Where the hook library is installed on disk.
pub fn hook_library_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("LOOP_AGENT_DIR") {
        return PathBuf::from(dir).join("hooks").join("loop-terminal.zsh");
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".loop")
        .join("agent")
        .join("hooks")
        .join("loop-terminal.zsh")
}

/// The user's zshrc path (`$ZDOTDIR/.zshrc` or `~/.zshrc`).
fn zshrc_path() -> PathBuf {
    if let Some(zdotdir) = std::env::var_os("ZDOTDIR") {
        if !zdotdir.is_empty() {
            return PathBuf::from(zdotdir).join(".zshrc");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zshrc")
}

/// The dashboard config directory that owns the ingestion token and port.
/// Mirrors `EndpointConfig::path()` on the Rust side.
pub fn dashboard_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("LOOP_DASHBOARD_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".loop")
        .join("dashboard")
}

/// The managed block written into `.zshrc`.
fn managed_block() -> String {
    format!(
        "{MARKER_BEGIN}\n\
         # Managed by Loop. Edit outside the markers only; re-run\n\
         # `loop install-terminal` to refresh or `loop uninstall-terminal`\n\
         # to remove this block.\n\
         [[ -f \"$HOME/.loop/agent/hooks/loop-terminal.zsh\" ]] && \
         source \"$HOME/.loop/agent/hooks/loop-terminal.zsh\"\n\
         {MARKER_END}\n"
    )
}
#[derive(Debug)]
pub struct InstallResult {
    /// Human-readable summary lines for the CLI to print.
    pub messages: Vec<String>,
    /// Whether anything changed on disk.
    pub changed: bool,
}

/// Install (or refresh) the hook library and the `.zshrc` source block.
///
/// Idempotent: the library file is rewritten only when its content differs;
/// the managed block replaces any previous managed block in place, leaving
/// every other line of `.zshrc` untouched.
pub fn install() -> std::io::Result<InstallResult> {
    let mut messages = Vec::new();
    let mut changed = false;

    // 1. Hook library.
    let lib_path = hook_library_path();
    if let Some(parent) = lib_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::read_to_string(&lib_path) {
        Ok(existing) if existing == HOOK_LIBRARY => {
            messages.push(format!("Hook library up to date: {}", lib_path.display()));
        }
        _ => {
            std::fs::write(&lib_path, HOOK_LIBRARY)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&lib_path, std::fs::Permissions::from_mode(0o644))?;
            }
            changed = true;
            messages.push(format!("Wrote hook library: {}", lib_path.display()));
        }
    }

    // 2. `.zshrc` managed block.
    let zshrc = zshrc_path();
    let current = std::fs::read_to_string(&zshrc).unwrap_or_default();
    let updated = upsert_managed_block(&current);
    if updated != current {
        if let Some(parent) = zshrc.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&zshrc, &updated)?;
        changed = true;
        messages.push(format!("{} {}", "Updated", zshrc.display()));
    } else {
        messages.push(format!("{} already configured", zshrc.display()));
    }

    Ok(InstallResult { messages, changed })
}

/// Remove the managed block and the hook library. Everything else in
/// `.zshrc` is preserved byte-for-byte.
pub fn uninstall() -> std::io::Result<InstallResult> {
    let mut messages = Vec::new();
    let mut changed = false;

    let zshrc = zshrc_path();
    if let Ok(current) = std::fs::read_to_string(&zshrc) {
        let stripped = strip_managed_block(&current);
        if stripped != current {
            std::fs::write(&zshrc, &stripped)?;
            changed = true;
            messages.push(format!("Removed managed block from {}", zshrc.display()));
        } else {
            messages.push(format!("No managed block found in {}", zshrc.display()));
        }
    } else {
        messages.push(format!("No {} to update", zshrc.display()));
    }

    let lib_path = hook_library_path();
    match std::fs::remove_file(&lib_path) {
        Ok(()) => {
            changed = true;
            messages.push(format!("Removed {}", lib_path.display()));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            messages.push(format!("{} already absent", lib_path.display()));
        }
        Err(e) => return Err(e),
    }

    Ok(InstallResult { messages, changed })
}

/// Insert or replace the managed block in `.zshrc` content.
///
/// * No previous block → append at the end (separated by a newline).
/// * Previous block → replace it in place, preserving surrounding lines.
pub fn upsert_managed_block(content: &str) -> String {
    if let Some((start, end)) = find_managed_block(content) {
        let mut out = String::with_capacity(content.len());
        out.push_str(&content[..start]);
        out.push_str(&managed_block());
        // Drop exactly one trailing newline duplication.
        let rest = &content[end..];
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        out.push_str(rest);
        return out;
    }
    let mut out = content.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&managed_block());
    out
}

/// Remove the managed block (and the blank line we may have added before
/// it) from `.zshrc` content.
pub fn strip_managed_block(content: &str) -> String {
    let Some((start, end)) = find_managed_block(content) else {
        return content.to_string();
    };
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..start]);
    let rest = &content[end..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    out.push_str(rest);
    // Tidy a doubled blank line left where the block used to be.
    while out.ends_with("\n\n\n") {
        out.pop();
    }
    out
}

/// Byte offsets `(begin_inclusive, end_exclusive)` of the managed block.
fn find_managed_block(content: &str) -> Option<(usize, usize)> {
    let start = content.find(MARKER_BEGIN)?;
    let end = content[start..].find(MARKER_END)? + start + MARKER_END.len();
    Some((start, end))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_inserts_block_once_and_is_idempotent() {
        let original = "# my config\nexport EDITOR=vim\n";
        let once = upsert_managed_block(original);
        assert!(once.contains(MARKER_BEGIN));
        assert!(once.contains("loop-terminal.zsh"));
        // user content survives
        assert!(once.contains("export EDITOR=vim"));
        // second install changes nothing
        let twice = upsert_managed_block(&once);
        assert_eq!(once, twice, "re-install must be a no-op");
        // only one block present
        assert_eq!(once.matches(MARKER_BEGIN).count(), 1);
    }

    #[test]
    fn install_replaces_stale_block_in_place() {
        let original = "before\n# >>> loop terminal observability (managed by `loop install-terminal`) >>>\nold line\n# <<< loop terminal observability <<<\nafter\n";
        let updated = upsert_managed_block(original);
        assert!(
            !updated.contains("old line"),
            "stale block must be replaced"
        );
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
        assert_eq!(updated.matches(MARKER_BEGIN).count(), 1);
        // order preserved around the block
        let bi = updated.find("before").unwrap();
        let mi = updated.find(MARKER_BEGIN).unwrap();
        let ai = updated.find("after").unwrap();
        assert!(bi < mi && mi < ai);
    }

    #[test]
    fn uninstall_removes_only_managed_block() {
        let original = "# my config\nexport EDITOR=vim\n";
        let installed = upsert_managed_block(original);
        let removed = strip_managed_block(&installed);
        assert!(!removed.contains(MARKER_BEGIN));
        assert!(!removed.contains(MARKER_END));
        assert!(removed.contains("# my config"));
        assert!(removed.contains("export EDITOR=vim"));
        // round-trip returns to (almost) the original
        assert_eq!(removed.trim_end(), original.trim_end());
    }

    #[test]
    fn uninstall_on_clean_config_is_noop() {
        let original = "alias ll='ls -la'\n";
        assert_eq!(strip_managed_block(original), original);
    }

    #[test]
    fn missing_trailing_newline_is_handled() {
        let original = "export A=1";
        let installed = upsert_managed_block(original);
        assert!(installed.contains('\n'));
        assert!(installed.starts_with("export A=1\n"));
    }

    #[test]
    fn hook_library_embeds_real_content() {
        assert!(HOOK_LIBRARY.contains("_loop_terminal_preexec"));
        assert!(HOOK_LIBRARY.contains("_loop_terminal_precmd"));
        assert!(HOOK_LIBRARY.contains("loopterm"));
        assert!(HOOK_LIBRARY.contains("precmd_functions"));
        // the hook must never write to the user's terminal
        assert!(!HOOK_LIBRARY.contains("print -r -- \"$_LOOP"));
    }
}
