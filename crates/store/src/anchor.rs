//! Chain heads kept outside the archive.
//!
//! A hash chain proves that nothing changed *after* a link, but not that the
//! whole chain is the original one: whoever can rewrite every row can produce a
//! chain that verifies against itself. The only defence is a copy of a head that
//! lives somewhere the attacker does not control, so a checkpoint is written as
//! one line in a file the user keeps elsewhere — printed, mailed to themselves,
//! or in a repository — and every later run checks the archive still agrees with
//! it.
//!
//! The file is plain text, one checkpoint per line:
//!
//! ```text
//! 2026-10-27T12:00:00Z 12 3f2a…c1
//! ```
//!
//! Timestamp, sequence number, hash. Lines starting with `#` and blank lines are
//! ignored, so the file can carry notes. Nothing secret is in it, so its file
//! mode is left to the user: an anchor is meant to be copied.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use rusqlite::Connection;

use crate::audit;
use crate::error::StoreError;

/// One recorded head.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Anchor {
    /// When the checkpoint was taken, RFC 3339.
    pub at: String,
    /// Sequence number it was taken at.
    pub seq: i64,
    /// Hash at that sequence.
    pub hash: String,
}

impl Anchor {
    /// The line this anchor occupies in its file.
    pub fn line(&self) -> String {
        format!("{} {} {} {}", self.at, self.seq, self.hash, "")
            .trim_end()
            .to_owned()
    }

    /// Reads one line, or `None` for a blank or commented one.
    fn parse(line: &str) -> Result<Option<Self>, StoreError> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(None);
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(at), Some(seq), Some(hash), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(rejected(format!(
                "an anchor line must be `timestamp sequence hash`: {trimmed}"
            )));
        };
        let seq: i64 = seq.parse().map_err(|_| {
            rejected(format!(
                "an anchor line has a sequence number that is not a number: {trimmed}"
            ))
        })?;
        if seq <= 0 {
            return Err(rejected(format!(
                "an anchor line has a sequence number below one: {trimmed}"
            )));
        }
        let hash = hash.to_ascii_lowercase();
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(rejected(format!(
                "an anchor line has a hash that is not a SHA-256: {trimmed}"
            )));
        }
        Ok(Some(Anchor {
            at: at.to_owned(),
            seq,
            hash,
        }))
    }
}

/// What checking an anchor file found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AnchorCheck {
    /// Checkpoints in the file.
    pub count: usize,
    /// The newest one, by sequence number.
    pub newest: Anchor,
}

/// What recording a checkpoint did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AnchorRecord {
    /// Checkpoints that were checked before writing.
    pub checked: usize,
    /// The checkpoint as it stands in the file, new or already there.
    pub anchor: Anchor,
    /// True when this run wrote a line.
    pub appended: bool,
}

/// Checks that every checkpoint in `path` is still part of this archive's chain.
///
/// This is the whole point of an anchor: it is the one check that a rewritten
/// chain cannot pass.
pub fn check(path: impl AsRef<Path>, conn: &Connection) -> Result<AnchorCheck, StoreError> {
    let anchors = read(path.as_ref())?;
    if anchors.is_empty() {
        return Err(rejected(
            "the anchor file has no checkpoints yet; run `bonk anchor` while the archive is sound",
        ));
    }
    agree(conn, &anchors)?;
    let newest = anchors
        .iter()
        .max_by_key(|anchor| anchor.seq)
        .cloned()
        .ok_or_else(|| rejected("the anchor file has no checkpoints yet"))?;
    Ok(AnchorCheck {
        count: anchors.len(),
        newest,
    })
}

/// Checks the file, then appends the current head when it is new.
///
/// A file that no longer agrees with the archive is refused rather than extended:
/// appending to it would turn a contradiction into a longer contradiction.
pub fn record(path: impl AsRef<Path>, conn: &Connection, now: &str) -> Result<AnchorRecord, StoreError> {
    let path = path.as_ref();
    let anchors = read(path)?;
    agree(conn, &anchors)?;

    let head = audit::head(conn)?
        .ok_or_else(|| rejected("this archive has no audit events yet, so there is no head to anchor"))?;
    let anchor = Anchor {
        at: now.to_owned(),
        seq: head.seq,
        hash: head.hash,
    };

    if anchors
        .iter()
        .any(|existing| existing.seq == anchor.seq && existing.hash == anchor.hash)
    {
        return Ok(AnchorRecord {
            checked: anchors.len(),
            anchor,
            appended: false,
        });
    }

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    // A newline first when the existing file does not end with one, so a
    // hand-edited file cannot swallow the new checkpoint.
    let needs_newline = std::fs::read(path)
        .ok()
        .and_then(|bytes| bytes.last().copied())
        .is_some_and(|last| last != b'\n');
    if needs_newline {
        file.write_all(b"\n")?;
    }
    file.write_all(anchor.line().as_bytes())?;
    file.write_all(b"\n")?;
    // A checkpoint that is lost in a crash is a checkpoint that was never taken.
    file.sync_all()?;

    Ok(AnchorRecord {
        checked: anchors.len(),
        anchor,
        appended: true,
    })
}

/// Reads every checkpoint, refusing a file that is not ordered.
fn read(path: &Path) -> Result<Vec<Anchor>, StoreError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    let mut anchors = Vec::new();
    for line in text.lines() {
        if let Some(anchor) = Anchor::parse(line)? {
            if let Some(previous) = anchors.last().map(|previous: &Anchor| previous.seq)
                && anchor.seq < previous
            {
                return Err(rejected(format!(
                    "the anchor file goes backwards: sequence {} follows {}",
                    anchor.seq, previous
                )));
            }
            anchors.push(anchor);
        }
    }
    Ok(anchors)
}

/// Every checkpoint must still be the link at its own sequence.
fn agree(conn: &Connection, anchors: &[Anchor]) -> Result<(), StoreError> {
    let current = audit::head(conn)?;
    let end = current.as_ref().map(|head| head.seq).unwrap_or(0);
    for anchor in anchors {
        if anchor.seq > end {
            return Err(rejected(format!(
                "an anchor from {} says the chain had {} links; this archive has {} — \
                 history was removed or this is a different archive",
                anchor.at, anchor.seq, end
            )));
        }
        let link = audit::at(conn, anchor.seq)?.ok_or_else(|| {
            rejected(format!(
                "an anchor from {} says link {} was {}; this archive has no link {} — \
                 history was removed",
                anchor.at, anchor.seq, anchor.hash, anchor.seq
            ))
        })?;
        if !link.hash.eq_ignore_ascii_case(&anchor.hash) {
            return Err(rejected(format!(
                "an anchor from {} says link {} was {}; this archive's link {} is {} — \
                 the chain was rewritten",
                anchor.at, anchor.seq, anchor.hash, anchor.seq, link.hash
            )));
        }
    }
    Ok(())
}

fn rejected(message: impl Into<String>) -> StoreError {
    StoreError::Rejected(message.into())
}

/// The file's checkpoints, for a caller that wants to print them.
pub fn list(path: impl AsRef<Path>) -> Result<Vec<Anchor>, StoreError> {
    read(path.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_line_round_trips() {
        let anchor = Anchor {
            at: "2026-10-27T12:00:00Z".to_owned(),
            seq: 12,
            hash: HASH.to_owned(),
        };
        assert_eq!(Anchor::parse(&anchor.line()).unwrap(), Some(anchor));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        assert_eq!(Anchor::parse("").unwrap(), None);
        assert_eq!(Anchor::parse("   ").unwrap(), None);
        assert_eq!(Anchor::parse("# a note").unwrap(), None);
    }

    #[test]
    fn a_malformed_line_is_refused() {
        for line in [
            "2026-10-27T12:00:00Z 12",
            "2026-10-27T12:00:00Z not-a-number abc",
            "2026-10-27T12:00:00Z 0 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "2026-10-27T12:00:00Z 1 too-short",
            "2026-10-27T12:00:00Z 1 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg",
            "2026-10-27T12:00:00Z 1 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef extra",
        ] {
            assert!(Anchor::parse(line).is_err(), "{line}");
        }
    }

    #[test]
    fn an_uppercase_hash_is_accepted() {
        let anchor = Anchor::parse(&format!("2026-10-27T12:00:00Z 1 {}", HASH.to_uppercase()))
            .unwrap()
            .unwrap();
        assert_eq!(anchor.hash, HASH);
    }
}
