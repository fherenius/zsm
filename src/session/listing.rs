use std::time::Duration;

/// The CLI probes session sockets; unlike get_session_list, it does not need
/// session-metadata.kdl to exist or parse successfully for a live session.
#[derive(Debug, Default)]
pub struct SessionListing {
    pub live: Vec<(String, bool)>,
    pub resurrectable: Vec<(String, Duration)>,
}

impl SessionListing {
    pub fn parse(output: &str) -> Result<Self, String> {
        let mut listing = Self::default();
        for line in output.lines().filter(|line| !line.is_empty()) {
            let (name, details) = line
                .rsplit_once(" [Created ")
                .ok_or_else(|| format!("Unrecognized session list row: {line}"))?;
            let (age, status) = details
                .split_once(" ago]")
                .ok_or_else(|| format!("Unrecognized session list row: {line}"))?;
            if name.is_empty() {
                return Err("Session list contained an empty name".into());
            }
            match status.trim() {
                "" => listing.live.push((name.into(), false)),
                "(current)" => listing.live.push((name.into(), true)),
                "(EXITED - attach to resurrect)" => listing.resurrectable.push((
                    name.into(),
                    humantime::parse_duration(age).map_err(|error| error.to_string())?,
                )),
                _ => return Err(format!("Unrecognized session status: {status}")),
            }
        }
        Ok(listing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_names_without_splitting_spaces_or_status_text_in_names() {
        let listing = SessionListing::parse(
            "my project [Created 1h 2m ago] (current)\n\
             peer (EXITED - attach to resurrect) [Created 3s ago] \n\
             saved [Created project [Created 5m ago] (EXITED - attach to resurrect)\n",
        )
        .unwrap();
        assert_eq!(
            listing.live,
            [
                ("my project".into(), true),
                ("peer (EXITED - attach to resurrect)".into(), false)
            ]
        );
        assert_eq!(
            listing.resurrectable,
            [("saved [Created project".into(), Duration::from_secs(300))]
        );
    }

    #[test]
    fn rejects_partial_or_unknown_output_instead_of_dropping_sessions() {
        for output in [
            "peer [Created 1s ago]\ntruncated row",
            "peer [Created 1s ago] (unknown)",
            "peer [Created invalid ago] (EXITED - attach to resurrect)",
        ] {
            assert!(SessionListing::parse(output).is_err());
        }
        assert!(SessionListing::parse("").unwrap().live.is_empty());
    }
}
