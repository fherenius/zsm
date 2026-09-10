//! Session visits shared between ZSM instances on the same host.

use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct SessionUsage {
    last_used: BTreeMap<String, u128>,
}

impl SessionUsage {
    pub fn record(&mut self, name: &str, timestamp: u128) -> String {
        self.remember(name.to_owned(), timestamp);
        // Hex keeps arbitrary session names on one line and out of shell syntax.
        let encoded: String = name.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        format!("{timestamp} {encoded}")
    }

    fn remember(&mut self, name: String, timestamp: u128) {
        self.last_used
            .entry(name)
            .and_modify(|previous| *previous = (*previous).max(timestamp))
            .or_insert(timestamp);
    }

    pub fn merge(&mut self, history: &str) {
        for line in history.lines() {
            let Some((timestamp, encoded)) = line.split_once(' ') else {
                continue;
            };
            let Ok(timestamp) = timestamp.parse::<u128>() else {
                continue;
            };
            if encoded.is_empty() || encoded.len() % 2 != 0 || !encoded.is_ascii() {
                continue;
            }
            let bytes: Result<Vec<_>, _> = (0..encoded.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16))
                .collect();
            let Ok(bytes) = bytes else { continue };
            if let Ok(name) = String::from_utf8(bytes) {
                self.remember(name, timestamp);
            }
        }
    }

    /// Most recently visited first; names make sessions without history stable.
    pub fn compare(&self, a: &str, b: &str) -> Ordering {
        self.last_used
            .get(b)
            .cmp(&self.last_used.get(a))
            .then_with(|| a.cmp(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_sort_newest_first_and_survive_other_instances_and_stale_reads() {
        let mut first = SessionUsage::default();
        let older = first.record("alpha", 10);
        let newer = first.record("zulu", 20);
        let mut second = SessionUsage::default();
        second.merge(&format!("{older}\n{newer}\n"));
        second.record("alpha", 30);
        second.merge(&older);
        let mut names = ["unvisited", "zulu", "alpha", "another"];
        names.sort_by(|a, b| second.compare(a, b));
        assert_eq!(names, ["alpha", "zulu", "another", "unvisited"]);
    }

    #[test]
    fn corrupt_records_are_ignored_and_unusual_names_round_trip() {
        let name = "日本語 '$(touch unwanted)'\nnext";
        let record = SessionUsage::default().record(name, 10);
        let mut usage = SessionUsage::default();
        usage.merge(&format!("garbage\n42 日\n12 zz\n2 ff\n{record}\n"));
        assert_eq!(usage.last_used.len(), 1);
        assert_eq!(usage.last_used.get(name), Some(&10));
    }

    #[test]
    #[cfg(unix)]
    fn host_cache_shares_concurrent_visits_and_respects_cache_paths() {
        use std::process::Command;
        let root = std::env::temp_dir().join(format!("zsm-usage-{}", uuid::Uuid::new_v4()));
        let cache = root.join("cache with spaces");
        let script = include_str!("session/usage.sh");
        let mut usage = SessionUsage::default();
        let records = [
            usage.record("alpha", 10),
            usage.record("日本語 '$(touch unwanted)'", 20),
        ];
        let mut children: Vec<_> = records
            .iter()
            .map(|record| {
                Command::new("sh")
                    .args(["-c", script, "zsm-test", record])
                    .env("XDG_CACHE_HOME", &cache)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let output = Command::new("sh")
            .args(["-c", script, "zsm-test"])
            .env("XDG_CACHE_HOME", &cache)
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut restored = SessionUsage::default();
        restored.merge(&String::from_utf8(output.stdout).unwrap());
        assert_eq!(restored.last_used, usage.last_used);

        let output = Command::new("sh")
            .args(["-c", script, "zsm-test", &records[0]])
            .env_remove("XDG_CACHE_HOME")
            .env("HOME", &root)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(root.join(".cache/zsm/session-usage").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
