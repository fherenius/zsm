use crate::{projects::Projects, records::encode, session_usage::SessionUsage};
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Store(PathBuf);
impl Store {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("zsm-store-{}", uuid::Uuid::new_v4())))
    }
    fn command(&self, kind: &str) -> Command {
        let mut command = Command::new("sh");
        command
            .args(["-c", include_str!("session/usage.sh"), "zsm-test", kind])
            .env("XDG_CACHE_HOME", self.0.join("cache with spaces"))
            .env("XDG_STATE_HOME", self.0.join("state with spaces"));
        command
    }
    fn read(&self, kind: &str) -> String {
        let output = self.command(kind).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn compaction_migrates_legacy_history_bounds_size_and_preserves_nanoseconds() {
    let store = Store::new();
    let file = store.0.join("cache with spaces/zsm/session-usage");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut legacy = String::new();
    for index in 0..5000 {
        legacy.push_str(&format!(
            "{} {}\n",
            1_800_000_000_000_000_000u128 + index,
            encode(&format!("session-{index}"))
        ));
    }
    // Older records arrive last, including timestamps differing by only 1 ns.
    legacy.push_str(&format!(
        "1800000000000000000 {}\nbad\n",
        encode("session-4999")
    ));
    std::fs::write(&file, legacy).unwrap();
    let compacted = store.read("usage");
    assert_eq!(compacted.lines().count(), 4096);
    assert!(compacted.starts_with("1800000000000004999 "));
    assert!(!compacted
        .lines()
        .any(|line| line.ends_with(&format!(" {}", encode("session-0")))));
    assert_eq!(compacted, store.read("usage"));
    let mut usage = SessionUsage::default();
    usage.merge(&compacted);
    assert_eq!(
        usage.compare("session-4999", "session-4998"),
        std::cmp::Ordering::Less
    );
}

#[test]
fn concurrent_compaction_does_not_lose_visits_pins_or_unpin_records() {
    let store = Store::new();
    let mut writers = Vec::new();
    for index in 0..24 {
        let record = format!("{} {}", index + 1, encode(&format!("session-{index}")));
        writers.push(
            store
                .command("usage")
                .arg(record)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut projects = Projects::default();
        let path = format!("/projects/{index} 日本語 '$(touch unwanted)'\nnext");
        let pin = projects.set_pinned(&path, true, 1_800_000_000_000_000_000);
        let unpin = projects.set_pinned(&path, false, 1_800_000_000_000_000_001);
        writers.push(
            store
                .command("projects")
                .args([unpin, pin])
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for mut writer in writers {
        assert!(writer.wait().unwrap().success());
    }
    assert_eq!(store.read("usage").lines().count(), 24);
    let records = store.read("projects");
    assert_eq!(records.lines().count(), 24);
    let mut projects = Projects::default();
    projects.merge(&records);
    assert_eq!(projects.pinned_paths().count(), 0);
    assert!(!store.0.join("state with spaces/zsm/projects.lock").exists());
}

#[test]
fn failed_compaction_leaves_the_previous_snapshot_and_releases_the_lock() {
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new();
    let original = format!("10 {}", encode("original"));
    let output = store.command("usage").arg(&original).output().unwrap();
    assert!(output.status.success());
    let bin = store.0.join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("awk"), "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(bin.join("awk"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = store
        .command("usage")
        .arg(format!("20 {}", encode("new")))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(store.read("usage").trim(), original);
}
