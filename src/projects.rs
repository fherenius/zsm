//! Persistent project pins and explicit session-directory associations.
//!
//! Timestamped unpin/clear records are retained so late command responses
//! cannot bring back a removed pin or a previous directory association.

use crate::records::{decode, encode};
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone)]
pub struct Projects {
    records: BTreeMap<String, (u128, String)>,
}

impl Projects {
    pub fn merge(&mut self, data: &str) {
        for line in data.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 3 {
                continue;
            }
            let (Ok(timestamp), Some(key), Some(value)) = (
                fields[0].parse::<u128>(),
                decode(fields[1]),
                decode(fields[2]),
            ) else {
                continue;
            };
            if !(key.starts_with("pin:") && matches!(value.as_str(), "0" | "1")
                || key.starts_with("session:"))
            {
                continue;
            }
            let record = (timestamp, value);
            let entry = self.records.entry(key).or_default();
            // The encoded-value tie break also matches the host compactor.
            if (record.0, encode(&record.1)) > (entry.0, encode(&entry.1)) {
                *entry = record;
            }
        }
    }

    fn record(&mut self, key: String, value: &str, now: u128) -> String {
        let timestamp = self
            .records
            .get(&key)
            .map_or(now, |(previous, _)| now.max(previous.saturating_add(1)));
        let record = format!("{timestamp} {} {}", encode(&key), encode(value));
        self.merge(&record);
        record
    }

    pub fn set_pinned(&mut self, path: &str, pinned: bool, now: u128) -> String {
        self.record(format!("pin:{path}"), if pinned { "1" } else { "0" }, now)
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.records
            .get(&format!("pin:{path}"))
            .is_some_and(|(_, value)| value == "1")
    }

    pub fn pinned_paths(&self) -> impl Iterator<Item = &str> {
        self.records.iter().filter_map(|(key, (_, value))| {
            (value == "1").then(|| key.strip_prefix("pin:")).flatten()
        })
    }

    pub fn set_directory(&mut self, session: &str, path: Option<&str>, now: u128) -> String {
        self.record(format!("session:{session}"), path.unwrap_or_default(), now)
    }

    pub fn directory(&self, session: &str) -> Option<&str> {
        self.records
            .get(&format!("session:{session}"))
            .and_then(|(_, path)| (!path.is_empty()).then_some(path.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_round_trip_and_stale_responses_cannot_restore_unpinned_paths() {
        let path = "/work/日本語 '$(touch unwanted)'\nnext";
        let mut projects = Projects::default();
        let pin = projects.set_pinned(path, true, 10);
        let association = projects.set_directory("project.2", Some(path), 11);
        let unpin = projects.set_pinned(path, false, 12);
        let mut other = Projects::default();
        other.merge(&format!("{unpin}\n{association}\n{pin}"));
        assert!(!other.is_pinned(path));
        assert_eq!(other.directory("project.2"), Some(path));
        assert_eq!(other.pinned_paths().count(), 0);
        let clear = other.set_directory("project.2", None, 13);
        projects.merge(&clear);
        projects.merge(&association);
        assert_eq!(projects.directory("project.2"), None);
        projects.merge("garbage\n2 zz 11\n42 日本語 10");
    }
}
