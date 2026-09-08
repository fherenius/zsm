//! Smart session naming for zoxide directories.
//!
//! A directory becomes a session name by taking its basename, then adding just
//! enough leading path context to tell it apart from the other directories
//! that share that basename. Directories sitting inside another zoxide
//! directory start with more context, so `src` under a known project reads as
//! `project.src` rather than a bare `src` that could be anything.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::config::Config;
use crate::session_name::MAX_SESSION_NAME_BYTES;
use crate::text;

/// Character budget for a generated name.
///
/// Zellij session names live inside a platform-dependent Unix-domain-socket
/// path, and the socket directory is not knowable from inside the WASM sandbox.
/// Generated names therefore aim for the conservative byte budget enforced by
/// [`MAX_SESSION_NAME_BYTES`].
pub const MAX_GENERATED_NAME_LEN: usize = 29;

/// Leading segments a directory nested inside another one starts with.
const NESTED_CONTEXT_SEGMENTS: usize = 3;

/// Generate a session name for each of `paths`, in the same order.
pub fn session_names(paths: &[&str], config: &Config) -> Vec<String> {
    // Normalising allocates, and both passes below need every path's
    // normalised form. Doing it once up front rather than once per comparison
    // is what keeps naming from being quadratic in allocations.
    let normalized: Vec<String> = paths
        .iter()
        .map(|path| normalize_path(path, config))
        .collect();
    let nested = nested_flags(&normalized);

    // BTreeMap rather than HashMap: each group only writes its own indices so
    // the result is order-independent either way, but a stable order keeps
    // debugging and tests predictable.
    let mut basename_groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, path) in paths.iter().enumerate() {
        basename_groups
            .entry(basename(path))
            .or_default()
            .push(index);
    }

    let mut names = vec![String::new(); paths.len()];
    for indices in basename_groups.into_values() {
        for &index in &indices {
            names[index] = context_aware_name(index, &normalized, &indices, nested[index], config);
        }
    }

    names
}

/// The basename of `path`, or an empty string if it has none (`/`).
fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

/// Flag each path that sits inside another one.
///
/// This walks each path's ancestors and looks them up, rather than comparing
/// every path against every other: that turns the pass from quadratic into
/// linear in the number of directories, which matters because zoxide databases
/// run to thousands of entries and this runs every time the plugin is shown.
///
/// Only a *strict* ancestor counts. Two different raw paths can normalise to
/// the same string when several `base_paths` overlap, and treating those as
/// nested inside each other gave both the same extra context.
fn nested_flags(normalized: &[String]) -> Vec<bool> {
    let all: HashSet<&str> = normalized.iter().map(String::as_str).collect();

    normalized
        .iter()
        .map(|path| {
            Path::new(path)
                .ancestors()
                .skip(1) // `ancestors` starts with the path itself
                .any(|ancestor| {
                    ancestor
                        .to_str()
                        .map(|ancestor| !ancestor.is_empty() && all.contains(ancestor))
                        .unwrap_or(false)
                })
        })
        .collect()
}

/// Name the path at `index`, adding the fewest leading segments that tell it
/// apart from the other paths in `conflicts`.
fn context_aware_name(
    index: usize,
    normalized: &[String],
    conflicts: &[usize],
    is_nested: bool,
    config: &Config,
) -> String {
    let segments = split_segments(&normalized[index]);
    if segments.is_empty() {
        return "root".to_string();
    }

    let separator = &config.session_separator;
    let rivals: Vec<Vec<&str>> = conflicts
        .iter()
        .filter(|&&other| other != index)
        .map(|&other| split_segments(&normalized[other]))
        .collect();

    let minimum = if is_nested {
        segments.len().min(NESTED_CONTEXT_SEGMENTS)
    } else {
        1
    };

    let required = (minimum..=segments.len())
        .find(|&length| {
            let candidate = join_tail(&segments, length, separator);
            !rivals.iter().any(|rival| {
                // A rival with fewer segments cannot produce this candidate.
                rival.len() >= length && join_tail(rival, length, separator) == candidate
            })
        })
        // Nothing tells this path apart from the ones it conflicts with. The
        // longest candidate at least carries the most information; the previous
        // code fell back to the *shortest*, which was the one already known to
        // collide.
        .unwrap_or(segments.len());

    let name = join_tail(&segments, required, separator);
    if name.chars().count() > MAX_GENERATED_NAME_LEN {
        return shorten(&segments, required, config);
    }

    within_hard_limit(name)
}

/// Bring an over-long name inside [`MAX_GENERATED_NAME_LEN`].
///
/// The required segments are abbreviated first, then dropped from the left,
/// and only cut mid-segment as a last resort. Any budget left over is spent
/// adding abbreviated context back from the left.
fn shorten(segments: &[&str], required: usize, config: &Config) -> String {
    let separator = &config.session_separator;

    let mut kept: Vec<String> = tail(segments, required)
        .iter()
        .map(|segment| abbreviate_segment(segment))
        .collect();

    while width(&kept, separator) > MAX_GENERATED_NAME_LEN && kept.len() > 1 {
        kept.remove(0);
    }

    // A single segment that is still too long has to be cut. This cuts on a
    // character boundary; `String::truncate` took a byte length and panicked
    // on any name where the budget landed inside a character.
    if width(&kept, separator) > MAX_GENERATED_NAME_LEN {
        kept = vec![text::truncate_chars(&kept[0], MAX_GENERATED_NAME_LEN)];
    }

    // Add abbreviated context back from the left while it fits. The range is
    // exclusive at the bottom of the *kept* segments, so index 0 is reachable;
    // the old loop stopped short of it and could never include the leftmost
    // segment.
    for index in (0..segments.len().saturating_sub(required)).rev() {
        let mut extended = vec![abbreviate_segment(segments[index])];
        extended.extend(kept.iter().cloned());

        if width(&extended, separator) > MAX_GENERATED_NAME_LEN {
            break;
        }
        kept = extended;
    }

    within_hard_limit(kept.join(separator))
}

/// Abbreviate one path segment.
///
/// `lobster-watcher` becomes `l-w`; a plain word keeps its first character
/// plus the next two letters.
fn abbreviate_segment(segment: &str) -> String {
    if segment.chars().count() <= 3 {
        return segment.to_string();
    }

    // Hyphenated and underscored names read well as initials.
    if segment.contains('-') || segment.contains('_') {
        let parts: Vec<&str> = segment.split(['-', '_']).collect();
        if parts.len() > 1 {
            return parts
                .iter()
                .filter_map(|part| part.chars().next())
                .map(String::from)
                .collect::<Vec<String>>()
                .join("-");
        }
    }

    let mut abbreviated = String::new();
    let mut characters = segment.chars();
    if let Some(first) = characters.next() {
        abbreviated.push(first);
    }
    for character in characters {
        if abbreviated.chars().count() >= 3 {
            break;
        }
        if character.is_alphabetic() {
            abbreviated.push(character);
        }
    }

    // A segment with no letters after the first character ("v1234") would
    // otherwise abbreviate down to a single character.
    if abbreviated.chars().count() < 2 {
        if let Some(second) = segment.chars().nth(1) {
            abbreviated.push(second);
        }
    }

    abbreviated
}

/// Strip the longest configured `base_path` from `path`.
///
/// A path that *is* a base path keeps its full form: stripping it would leave
/// nothing to name the session after.
fn normalize_path(path: &str, config: &Config) -> String {
    let mut longest: Option<&str> = None;

    for base_path in &config.base_paths {
        let base = base_path.trim_end_matches('/');
        if !path.starts_with(base) {
            continue;
        }
        // Match only on a component boundary, so /home/user does not match
        // /home/username.
        let on_boundary =
            path.len() == base.len() || path.as_bytes().get(base.len()) == Some(&b'/');
        if on_boundary && longest.map_or(0, str::len) <= base.len() {
            longest = Some(base);
        }
    }

    match longest {
        // `starts_with` guarantees base.len() is a character boundary.
        Some(base) if path.len() > base.len() => {
            path[base.len()..].trim_start_matches('/').to_string()
        }
        _ => path.to_string(),
    }
}

/// Split a path into its non-empty components.
fn split_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// The last `count` segments of `segments`.
fn tail<'a>(segments: &'a [&'a str], count: usize) -> &'a [&'a str] {
    &segments[segments.len().saturating_sub(count)..]
}

/// The last `count` segments of `segments`, joined.
fn join_tail(segments: &[&str], count: usize, separator: &str) -> String {
    tail(segments, count).join(separator)
}

/// Character width of `segments` once joined.
fn width(segments: &[String], separator: &str) -> usize {
    segments.join(separator).chars().count()
}

/// Keep a name inside the conservative byte budget for the socket name.
///
/// The character budget above is the design target; this is the backstop for
/// multi-byte names, where 29 characters can exceed 29 bytes.
fn within_hard_limit(name: String) -> String {
    if name.len() <= MAX_SESSION_NAME_BYTES {
        return name;
    }

    text::truncate_bytes(&name, MAX_SESSION_NAME_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_name;

    fn config(separator: &str, base_paths: &[&str]) -> Config {
        Config {
            session_separator: separator.to_string(),
            base_paths: base_paths.iter().map(|path| path.to_string()).collect(),
            ..Config::default()
        }
    }

    fn plain() -> Config {
        config(".", &[])
    }

    #[test]
    fn a_unique_basename_names_the_session_on_its_own() {
        let names = session_names(&["/home/user/zsm", "/home/user/other"], &plain());
        assert_eq!(names, vec!["zsm", "other"]);
    }

    #[test]
    fn a_unique_long_basename_is_shortened() {
        let basename = "a".repeat(120);
        let path = format!("/home/user/{basename}");
        let names = session_names(&[&path], &plain());

        assert_eq!(names.len(), 1);
        assert!(names[0].chars().count() <= MAX_GENERATED_NAME_LEN);
        assert!(session_name::validate(&names[0]).is_ok());
    }

    #[test]
    fn clashing_basenames_gain_the_least_context_that_separates_them() {
        let names = session_names(&["/home/user/a/api", "/home/user/b/api"], &plain());
        assert_eq!(names, vec!["a.api", "b.api"]);
    }

    #[test]
    fn context_grows_until_the_names_differ() {
        let names = session_names(&["/x/team/a/api", "/y/team/a/api"], &plain());
        assert_eq!(names, vec!["x.team.a.api", "y.team.a.api"]);
    }

    #[test]
    fn a_directory_inside_another_zoxide_directory_gets_extra_context() {
        let names = session_names(&["/home/user/zsm", "/home/user/zsm/src"], &plain());
        assert_eq!(names[0], "zsm");
        // `src` alone would say nothing about which project it belongs to.
        assert_eq!(names[1], "user.zsm.src");
    }

    #[test]
    fn the_separator_is_configurable() {
        let names = session_names(&["/home/user/a/api", "/home/user/b/api"], &config("_", &[]));
        assert_eq!(names, vec!["a_api", "b_api"]);
    }

    #[test]
    fn base_paths_are_stripped() {
        let cfg = config(".", &["/home/user"]);
        assert_eq!(
            normalize_path("/home/user/projects/zsm", &cfg),
            "projects/zsm"
        );

        let names = session_names(&["/home/user/zsm", "/home/user/zsm/src"], &cfg);
        assert_eq!(names[0], "zsm");
        assert_eq!(names[1], "zsm.src");
    }

    #[test]
    fn the_longest_matching_base_path_wins() {
        let cfg = config(".", &["/home/user", "/home/user/projects"]);
        let names = session_names(&["/home/user/projects/a/api", "/home/user/b/api"], &cfg);
        assert_eq!(names, vec!["a.api", "b.api"]);
    }

    #[test]
    fn a_path_equal_to_a_base_path_keeps_its_full_form() {
        let cfg = config(".", &["/home/user"]);
        // Stripping it would leave nothing to name the session after.
        assert_eq!(session_names(&["/home/user"], &cfg), vec!["user"]);
    }

    #[test]
    fn base_paths_only_match_on_a_component_boundary() {
        let cfg = config(".", &["/home/user"]);
        // /home/username must not be treated as living under /home/user.
        assert_eq!(normalize_path("/home/username/a", &cfg), "/home/username/a");
        assert_eq!(normalize_path("/home/user/a", &cfg), "a");
        // A path that *is* the base path keeps its full form.
        assert_eq!(normalize_path("/home/user", &cfg), "/home/user");
    }

    #[test]
    fn the_longest_base_path_is_the_one_stripped() {
        let cfg = config(".", &["/home/user", "/home/user/projects"]);
        assert_eq!(normalize_path("/home/user/projects/zsm", &cfg), "zsm");
        assert_eq!(normalize_path("/home/user/other/zsm", &cfg), "other/zsm");
        // Declaration order must not matter.
        let reversed = config(".", &["/home/user/projects", "/home/user"]);
        assert_eq!(normalize_path("/home/user/projects/zsm", &reversed), "zsm");
    }

    #[test]
    fn paths_outside_every_base_path_are_left_alone() {
        let cfg = config(".", &["/home/user"]);
        assert_eq!(normalize_path("/opt/tools", &cfg), "/opt/tools");
        assert_eq!(normalize_path("/opt/tools", &plain()), "/opt/tools");
    }

    #[test]
    fn trailing_slashes_in_base_paths_are_ignored() {
        let cfg = config(".", &["/home/user/"]);
        assert_eq!(session_names(&["/home/user/zsm"], &cfg), vec!["zsm"]);
    }

    #[test]
    fn the_filesystem_root_is_named_rather_than_left_blank() {
        // The basename of "/" is empty, which used to become an empty session
        // name that Zellij cannot use.
        assert_eq!(session_names(&["/"], &plain()), vec!["root"]);
    }

    #[test]
    fn long_names_are_abbreviated_within_the_budget() {
        let names = session_names(
            &["/home/developer/workspace/lobster-watcher/services/api"],
            &plain(),
        );
        assert!(
            names[0].chars().count() <= MAX_GENERATED_NAME_LEN,
            "{} is too long",
            names[0]
        );
        assert!(!names[0].is_empty());
    }

    #[test]
    fn hyphenated_segments_abbreviate_to_initials() {
        assert_eq!(abbreviate_segment("lobster-watcher"), "l-w");
        assert_eq!(abbreviate_segment("my_cool_project"), "m-c-p");
        assert_eq!(abbreviate_segment("api"), "api");
        assert_eq!(abbreviate_segment("services"), "ser");
        // No letters after the first character, so fall back to the second.
        assert_eq!(abbreviate_segment("v1234"), "v1");
    }

    /// Regression: shortening called `String::truncate` with a byte length, so
    /// a budget landing inside a multi-byte character panicked - and a panic
    /// traps the WASM instance and kills the plugin.
    #[test]
    fn multi_byte_paths_do_not_panic_and_stay_inside_the_hard_limit() {
        let paths = [
            "/home/\u{043f}\u{043e}\u{043b}\u{044c}\u{0437}\u{043e}\u{0432}\u{0430}\u{0442}\u{0435}\u{043b}\u{044c}/\u{043f}\u{0440}\u{043e}\u{0435}\u{043a}\u{0442}\u{044b}/\u{0441}\u{0430}\u{0439}\u{0442}",
            "/Users/fester/\u{6587}\u{66f8}/\u{30d7}\u{30ed}\u{30b8}\u{30a7}\u{30af}\u{30c8}/\u{8a2d}\u{5b9a}",
            "/home/u/\u{1f680}\u{1f680}\u{1f680}\u{1f680}\u{1f680}\u{1f680}\u{1f680}\u{1f680}\u{1f680}\u{1f680}/app",
            "/\u{1f4a9}",
            "/home/u/a\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}\u{0301}/very-long-name-here/deeper",
        ];

        for name in session_names(&paths, &plain()) {
            assert!(
                name.len() <= MAX_SESSION_NAME_BYTES,
                "{name:?} too many bytes"
            );
            assert!(session_name::validate(&name).is_ok(), "{name:?} invalid");
        }
    }

    /// Every generated name has to be usable, or selecting the directory fails
    /// at the point of creating the session.
    #[test]
    fn every_generated_name_is_valid() {
        let paths = [
            "/",
            "/home",
            "/home/user",
            "/home/user/projects/zsm",
            "/home/user/projects/zsm/src/ui",
            "/home/user/projects/another-project-with-a-very-long-name/src",
            "/home/user/projects/another-project-with-a-very-long-name/src/deeply/nested/thing",
            "relative/path",
            "/home/user/a/api",
            "/home/user/b/api",
        ];

        for cfg in [plain(), config("_", &["/home/user"]), config("-", &["/"])] {
            for name in session_names(&paths, &cfg) {
                assert!(session_name::validate(&name).is_ok(), "{name:?} invalid");
                assert!(!name.is_empty(), "empty name for {cfg:?}");
                assert!(
                    name.chars().count() <= MAX_GENERATED_NAME_LEN,
                    "{name:?} over the budget"
                );
            }
        }
    }

    #[test]
    fn names_are_generated_in_input_order() {
        let paths = ["/z/api", "/a/api", "/m/other"];
        assert_eq!(
            session_names(&paths, &plain()),
            vec!["z.api", "a.api", "other"]
        );
    }

    #[test]
    fn duplicate_paths_do_not_look_nested_in_each_other() {
        // Both normalise to the same string; neither is a strict ancestor.
        let cfg = config(".", &["/a", "/b"]);
        let names = session_names(&["/a/x", "/b/x"], &cfg);
        assert_eq!(names.len(), 2);
        for name in &names {
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn an_empty_directory_list_produces_no_names() {
        assert!(session_names(&[], &plain()).is_empty());
    }
}
