//! Watching a folder for dropped invoices.
//!
//! An ERP or a bookkeeper drops files into a directory; this polls it and hands
//! each new file to the import pipeline once.
//!
//! Polling rather than change notifications is deliberate. Drop folders are
//! often network shares, where `inotify` and its relatives either do not work or
//! lie, and the real question is not "did something happen" but "has the writer
//! finished" — which is a quiet period, not an event. A file is therefore offered
//! when it is at least `quiet_for` old and is new or has changed since it was last
//! offered.
//!
//! Anything offered is remembered, so a file that will never parse is not retried
//! on every poll. Editing it changes its modification time and offers it again.
//!
//! Files already in the folder when the watcher starts are left alone: a drop
//! folder can hold years of invoices, and starting a watcher must not import
//! them. `ready_now` is the one-shot exception, for a deliberate folder import.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use store::StoreError;

/// What a file looked like the last time the watcher offered it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

/// Polls a folder and remembers what it has already offered.
#[derive(Clone, Debug)]
pub struct Watcher {
    quiet_for: Duration,
    seen: BTreeMap<PathBuf, Fingerprint>,
    primed: bool,
}

impl Watcher {
    /// Watches `quiet_for` before reading a file, so a half-written copy is not
    /// parsed.
    pub fn new(quiet_for: Duration) -> Self {
        Self {
            quiet_for,
            seen: BTreeMap::new(),
            primed: false,
        }
    }

    /// Files that are old enough and are new or changed since the last poll.
    ///
    /// The first call only records what is there and returns nothing, which is
    /// what keeps a restart from importing the folder's back catalogue.
    pub fn poll(&mut self, directory: &Path, now: SystemTime) -> Result<Vec<PathBuf>, StoreError> {
        let found = entries(directory)?;
        let mut offered = Vec::new();

        if !self.primed {
            self.seen = found.into_iter().collect();
            self.primed = true;
            return Ok(offered);
        }

        for (path, print) in found {
            if self.seen.get(&path) == Some(&print) {
                continue;
            }
            // Only remember it once it has been offered: a file that is still
            // being written keeps changing, and must be noticed when it settles.
            if is_quiet(print, now, self.quiet_for) {
                self.seen.insert(path.clone(), print);
                offered.push(path);
            }
        }
        offered.sort();
        Ok(offered)
    }
}

/// Everything in the folder that is old enough to read, ignoring history.
///
/// This is a one-shot folder import: no priming and no memory, so a cron job or a
/// `--once` run acts on the folder as it stands.
pub fn ready_now(directory: &Path, now: SystemTime, quiet_for: Duration) -> Result<Vec<PathBuf>, StoreError> {
    let mut ready: Vec<PathBuf> = entries(directory)?
        .into_iter()
        .filter(|(_, print)| is_quiet(*print, now, quiet_for))
        .map(|(path, _)| path)
        .collect();
    ready.sort();
    Ok(ready)
}

/// Names that are never invoices: editor lock files, partial downloads, editor
/// temporaries, and anything hidden.
fn ignored(name: &str) -> bool {
    if name.starts_with('.') || name.starts_with("~$") {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    [".tmp", ".part", ".crdownload", ".swp", ".download"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

/// Files in one directory, with what the watcher needs to recognise them.
///
/// Not recursive: an ERP drops invoices, not trees, and recursing invites
/// importing an archive by accident.
fn entries(directory: &Path) -> Result<Vec<(PathBuf, Fingerprint)>, StoreError> {
    let reader = std::fs::read_dir(directory).map_err(|error| {
        StoreError::Io(std::io::Error::new(
            error.kind(),
            format!("cannot watch {}: {error}", directory.display()),
        ))
    })?;

    let mut found = Vec::new();
    for entry in reader {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if ignored(name) {
            continue;
        }
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        found.push((
            entry.path(),
            Fingerprint {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            },
        ));
    }
    Ok(found)
}

/// True when the file has not been touched for at least the quiet period.
///
/// A file whose modification time cannot be read is treated as ready: there is
/// nothing better to go on, and refusing it forever would be worse.
fn is_quiet(print: Fingerprint, now: SystemTime, quiet_for: Duration) -> bool {
    let Some(modified) = print.modified else {
        return true;
    };
    match now.duration_since(modified) {
        Ok(age) => age >= quiet_for,
        // A modification time in the future is a clock problem, not a signal.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write(directory: &Path, name: &str, contents: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_new_file_waits_for_the_quiet_period() {
        let directory = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(Duration::from_secs(5));
        let now = SystemTime::now();

        // The folder is watched from empty, as it would be in practice.
        assert!(watcher.poll(directory.path(), now).unwrap().is_empty());
        let path = write(directory.path(), "invoice.xml", "<Invoice/>");

        // Too fresh to read.
        assert!(watcher.poll(directory.path(), now).unwrap().is_empty());
        // Quiet long enough.
        let later = now + Duration::from_secs(6);
        assert_eq!(watcher.poll(directory.path(), later).unwrap(), vec![path.clone()]);
        // Already offered, so it is not offered again.
        assert!(
            watcher
                .poll(directory.path(), later + Duration::from_secs(30))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_changed_file_is_offered_again() {
        let directory = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(Duration::from_secs(1));
        let now = SystemTime::now();
        watcher.poll(directory.path(), now).unwrap();
        let path = write(directory.path(), "invoice.xml", "<Invoice/>");

        let later = now + Duration::from_secs(2);
        assert_eq!(watcher.poll(directory.path(), later).unwrap(), vec![path.clone()]);

        std::fs::write(&path, "<Invoice><ID>2</ID></Invoice>").unwrap();
        let later_still = later + Duration::from_secs(2);
        assert_eq!(watcher.poll(directory.path(), later_still).unwrap(), vec![path]);
    }

    #[test]
    fn the_first_poll_only_remembers_what_is_there() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "old.xml", "<Invoice/>");
        let mut watcher = Watcher::new(Duration::from_secs(0));

        assert!(
            watcher
                .poll(directory.path(), SystemTime::now() + Duration::from_secs(60))
                .unwrap()
                .is_empty(),
            "a drop folder's back catalogue is not a batch to import"
        );
    }

    #[test]
    fn a_ready_now_scan_ignores_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = write(directory.path(), "invoice.xml", "<Invoice/>");
        let now = SystemTime::now() + Duration::from_secs(60);

        assert_eq!(
            ready_now(directory.path(), now, Duration::from_secs(5)).unwrap(),
            vec![path]
        );
    }

    #[test]
    fn editor_leftovers_and_hidden_files_are_skipped() {
        let directory = tempfile::tempdir().unwrap();
        for name in [".invoice.xml", "~$invoice.docx", "invoice.xml.tmp", "invoice.pdf.part"] {
            write(directory.path(), name, "<Invoice/>");
        }
        let path = write(directory.path(), "invoice.xml", "<Invoice/>");
        let now = SystemTime::now() + Duration::from_secs(60);

        assert_eq!(
            ready_now(directory.path(), now, Duration::from_secs(5)).unwrap(),
            vec![path]
        );
    }

    #[test]
    fn folders_are_not_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("2026")).unwrap();
        let now = SystemTime::now() + Duration::from_secs(60);

        assert!(
            ready_now(directory.path(), now, Duration::from_secs(0))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_watch() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("typo");
        assert!(ready_now(&missing, SystemTime::now(), Duration::from_secs(5)).is_err());
        assert!(
            Watcher::new(Duration::from_secs(5))
                .poll(&missing, SystemTime::now())
                .is_err()
        );
    }
}
