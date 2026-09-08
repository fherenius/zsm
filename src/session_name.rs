//! Validation for session names the user can influence.
//!
//! Zellij session names end up inside a Unix-domain-socket path, whose full
//! platform-dependent length includes a socket directory that is not visible
//! inside the WASM sandbox. ZSM therefore applies the same conservative budget
//! to generated and user-entered names.

use crate::text;

/// Conservative budget for the session-name portion of Zellij's socket path.
pub const MAX_SESSION_NAME_BYTES: usize = 29;

/// Highest increment tried before the caller falls back to a random suffix.
const MAX_INCREMENT: u32 = 1000;

/// Check a session name against the limits Zellij imposes.
///
/// An empty name is accepted: every caller maps it to "let Zellij pick a
/// random name".
pub fn validate(name: &str) -> Result<(), &'static str> {
    // Empty means "let Zellij choose a random name". Whitespace is passed as
    // an explicit name, though, and Zellij rejects it.
    if name.is_empty() {
        return Ok(());
    }
    if name.trim().is_empty() {
        return Err("Session name cannot contain only whitespace");
    }
    if name == "." || name == ".." {
        return Err("Session name cannot be '.' or '..'");
    }
    if name.contains('/') {
        return Err("Session name cannot contain '/'");
    }
    if name.len() > MAX_SESSION_NAME_BYTES {
        return Err("Session name must be at most 29 bytes");
    }

    Ok(())
}

/// Check a requested name against the session the plugin is running in.
///
/// Zellij cannot switch a session to itself. Refusing silently looks like a
/// dropped key press, so this reports a message the caller can surface.
pub fn validate_against_current(name: &str, current: Option<&str>) -> Result<(), &'static str> {
    validate(name)?;

    if !name.is_empty() && Some(name) == current {
        return Err("Cannot create session with same name as current session");
    }

    Ok(())
}

/// Validate a name for a new session, including collisions with live and dead
/// sessions. Zellij treats a taken name as an attach/resurrect request and
/// ignores the requested cwd and layout, which is never what this form means.
pub fn validate_for_creation(
    name: &str,
    current: Option<&str>,
    is_taken: impl FnOnce(&str) -> bool,
) -> Result<(), &'static str> {
    validate_against_current(name, current)?;

    if !name.is_empty() && is_taken(name) {
        return Err("A session with this name already exists");
    }

    Ok(())
}

/// Fit `base<separator><suffix>` inside the safe byte budget without splitting
/// a UTF-8 codepoint. Invalid separators produce no candidate.
pub fn with_suffix(base: &str, separator: &str, suffix: &str) -> Option<String> {
    let tail = format!("{separator}{suffix}");
    let base_budget = MAX_SESSION_NAME_BYTES.checked_sub(tail.len())?;
    let base = text::truncate_bytes(base, base_budget);
    let candidate = format!("{base}{tail}");
    validate(&candidate).ok().map(|()| candidate)
}

/// Pick the first unused name in the series `base`, `base<sep>2`, `base<sep>3`.
///
/// `is_taken` must report live *and* resurrectable sessions. Switching to the
/// name of a dead session resurrects it instead of creating a fresh one, so
/// the requested directory and layout would be silently ignored.
///
/// Returns `None` when every candidate up to [`MAX_INCREMENT`] is taken,
/// leaving the caller to invent a unique suffix.
pub fn first_free_increment(
    base: &str,
    separator: &str,
    is_taken: impl Fn(&str) -> bool,
) -> Option<String> {
    if validate(base).is_ok() && !is_taken(base) {
        return Some(base.to_string());
    }

    (2..=MAX_INCREMENT)
        .filter_map(|counter| with_suffix(base, separator, &counter.to_string()))
        .find(|candidate| !is_taken(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_are_accepted() {
        assert!(validate("zsm").is_ok());
        assert!(validate("projects.zsm").is_ok());
        // Empty means "let Zellij pick a random name".
        assert!(validate("").is_ok());
        assert!(validate(&"a".repeat(MAX_SESSION_NAME_BYTES)).is_ok());
    }

    #[test]
    fn over_long_names_are_rejected() {
        assert!(validate(&"a".repeat(MAX_SESSION_NAME_BYTES + 1)).is_err());
        assert!(validate(&"a".repeat(MAX_SESSION_NAME_BYTES + 50)).is_err());
    }

    #[test]
    fn names_rejected_by_zellij_are_rejected_here_too() {
        assert!(validate(" ").is_err());
        assert!(validate("\t\n").is_err());
        assert!(validate(".").is_err());
        assert!(validate("..").is_err());
    }

    #[test]
    fn names_containing_a_path_separator_are_rejected() {
        assert!(validate("projects/zsm").is_err());
        assert!(validate("/absolute").is_err());
    }

    /// The byte limit is a byte limit, not a character limit: the socket path
    /// is measured in bytes.
    #[test]
    fn the_length_limit_counts_bytes() {
        let multi_byte = "\u{00e9}".repeat(15); // 30 bytes, 15 characters
        assert_eq!(multi_byte.chars().count(), 15);
        assert!(validate(&multi_byte).is_err());
    }

    #[test]
    fn the_current_session_cannot_be_recreated() {
        assert!(validate_against_current("zsm", Some("zsm")).is_err());
        assert!(validate_against_current("zsm", Some("other")).is_ok());
        assert!(validate_against_current("zsm", None).is_ok());
        // A random name never collides with the current one.
        assert!(validate_against_current("", Some("zsm")).is_ok());
        // The cheaper checks still apply.
        assert!(validate_against_current("a/b", Some("zsm")).is_err());
    }

    #[test]
    fn creation_rejects_live_and_resurrectable_names() {
        assert!(validate_for_creation("zsm", Some("current"), |name| name == "zsm").is_err());
        assert!(validate_for_creation("zsm", Some("current"), |_| false).is_ok());
        // Empty delegates random-name generation to Zellij and cannot collide.
        assert!(validate_for_creation("", Some("current"), |_| true).is_ok());
    }

    #[test]
    fn a_free_base_name_is_used_as_is() {
        assert_eq!(
            first_free_increment("zsm", ".", |_| false),
            Some("zsm".to_string())
        );
    }

    #[test]
    fn taken_names_are_incremented() {
        let taken = ["zsm", "zsm.2", "zsm.3"];
        assert_eq!(
            first_free_increment("zsm", ".", |name| taken.contains(&name)),
            Some("zsm.4".to_string())
        );
        assert_eq!(
            first_free_increment("zsm", "_", |name| name == "zsm"),
            Some("zsm_2".to_string())
        );
    }

    #[test]
    fn increments_leave_room_for_the_suffix() {
        let base = "a".repeat(MAX_SESSION_NAME_BYTES);
        let incremented = first_free_increment(&base, ".", |name| name == base).unwrap();

        assert!(incremented.ends_with(".2"));
        assert!(incremented.len() <= MAX_SESSION_NAME_BYTES);
        assert!(validate(&incremented).is_ok());
    }

    #[test]
    fn suffixing_a_multi_byte_base_keeps_valid_utf8() {
        let base = "\u{00e9}".repeat(20);
        let incremented = first_free_increment(&base, ".", |_| false).unwrap();

        assert!(incremented.ends_with(".2"));
        assert!(incremented.len() <= MAX_SESSION_NAME_BYTES);
        assert!(validate(&incremented).is_ok());
    }

    #[test]
    fn invalid_separators_do_not_produce_invalid_candidates() {
        assert_eq!(first_free_increment("zsm", "/", |_| true), None);
    }

    /// Regression: only the base name was checked against resurrectable
    /// sessions, so an increment could land on a dead session's name. Zellij
    /// then resurrects that session and the requested cwd and layout are lost.
    #[test]
    fn increments_avoid_resurrectable_names_too() {
        let live = ["zsm"];
        let resurrectable = ["zsm.2", "zsm.3"];
        let name = first_free_increment("zsm", ".", |name| {
            live.contains(&name) || resurrectable.contains(&name)
        });
        assert_eq!(name, Some("zsm.4".to_string()));
    }

    #[test]
    fn an_exhausted_series_gives_up_so_the_caller_can_randomise() {
        assert_eq!(first_free_increment("zsm", ".", |_| true), None);
    }
}
