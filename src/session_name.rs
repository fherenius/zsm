//! Validation for session names the user can influence.
//!
//! Zellij session names end up inside a Unix-domain-socket path, which is
//! capped at 108 bytes. The socket directory is not knowable from inside the
//! WASM sandbox, so generated names aim well below the cap (see the naming
//! module) while this module enforces the hard limits that apply to *any*
//! name, including one typed by hand.

/// Hard upper bound on a session name, from the 108-byte socket path limit.
pub const MAX_SESSION_NAME_BYTES: usize = 108;

/// Highest increment tried before the caller falls back to a random suffix.
const MAX_INCREMENT: u32 = 1000;

/// Check a session name against the limits Zellij imposes.
///
/// An empty name is accepted: every caller maps it to "let Zellij pick a
/// random name".
pub fn validate(name: &str) -> Result<(), &'static str> {
    if name.len() >= MAX_SESSION_NAME_BYTES {
        return Err("Session name must be shorter than 108 bytes");
    }
    if name.contains('/') {
        return Err("Session name cannot contain '/'");
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
    if !is_taken(base) {
        return Some(base.to_string());
    }

    (2..=MAX_INCREMENT)
        .map(|counter| format!("{}{}{}", base, separator, counter))
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
        assert!(validate(&"a".repeat(MAX_SESSION_NAME_BYTES - 1)).is_ok());
    }

    #[test]
    fn over_long_names_are_rejected() {
        assert!(validate(&"a".repeat(MAX_SESSION_NAME_BYTES)).is_err());
        assert!(validate(&"a".repeat(MAX_SESSION_NAME_BYTES + 50)).is_err());
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
        let multi_byte = "\u{00e9}".repeat(60); // 120 bytes, 60 characters
        assert_eq!(multi_byte.chars().count(), 60);
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
