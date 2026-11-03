//! Recovery kits: the key, kept apart from the copies of the data it opens.
//!
//! A database backup cannot be opened on its own. The archive is encrypted with
//! a random data key, and the passphrase only unwraps that key from
//! `keystore.bin`; a recovery key is a second wrap inside the same file, so
//! neither is of any use when the keystore is gone. A kit is that file, copied
//! somewhere else, next to the instructions a future reader will need.
//!
//! Both the command line and the window go through [`write`], so the refusal to
//! put a kit beside the archive cannot be forgotten by one of them.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::StoreError;
use crate::keystore;

/// What one kit write produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KitReport {
    /// The directory the kit was written into.
    pub directory: PathBuf,
    /// The copy of the keystore.
    pub key: PathBuf,
    /// The note explaining what the key is and how to restore.
    pub instructions: PathBuf,
}

/// Writes a copy of `source` (a keystore) and its instructions into `directory`.
///
/// The archive's own directory and everything under it are refused: a kit stored
/// beside the archive, or beside its backups, is taken by the same theft or the
/// same disk failure as the data it is meant to survive.
///
/// Nothing is decrypted, so no passphrase is needed, and an existing kit in the
/// destination is never replaced.
pub fn write(
    source: impl AsRef<Path>,
    archive: impl AsRef<Path>,
    directory: impl AsRef<Path>,
    stamp: &str,
) -> Result<KitReport, StoreError> {
    let source = source.as_ref();
    let archive = archive.as_ref();
    let directory = directory.as_ref();

    fs::create_dir_all(directory)?;
    // Resolve before comparing: the same folder can be reached two ways.
    let target = fs::canonicalize(directory)?;
    let archive_directory = archive.parent().unwrap_or_else(|| Path::new("."));
    if let Ok(resolved) = fs::canonicalize(archive_directory)
        && target.starts_with(&resolved)
    {
        return Err(StoreError::Rejected(format!(
            "a recovery kit must not be written inside the archive directory ({}); use another medium",
            archive_directory.display()
        )));
    }

    let key = keystore::copy_to_directory(source, directory)?;
    let instructions = directory.join("RECOVERY.txt");
    fs::write(&instructions, note(archive, stamp))?;

    Ok(KitReport {
        directory: directory.to_path_buf(),
        key,
        instructions,
    })
}

/// The note that travels with the key.
fn note(archive: &Path, stamp: &str) -> String {
    let name = archive
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bonk.db".to_owned());
    let body = r#"Bonk recovery kit
=================

Written: STAMP
Archive: ARCHIVE

keystore.bin in this directory is the key to that archive. The database backups
do not contain it: a backup, this file, and the passphrase (or the recovery key
enabled when this copy was taken) are what restore an archive.

Keep this directory apart from the database and its backups — use another medium:
a different disk, a USB stick in a safe, or an attachment in a password manager.
Anyone who has both this file and the credential has the archive.

To restore:
  1. Put a database backup next to this file, named as the application expects
     (for example bonk.db).
  2. Open it with this keystore, or check it first:
       bonk verify --db bonk.db --keystore keystore.bin
  3. Enter the passphrase that was current when this kit was written.

This kit holds no passphrase and no recovery key. If both are lost, the archive
cannot be opened, whatever copies exist.

If the passphrase changes, or a recovery key is added or replaced, write a new
kit: an old copy still unwraps the same data key, but with the credential it was
taken under, so it is a second way in with the old passphrase.
"#;
    body.replace("STAMP", stamp).replace("ARCHIVE", &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::{KdfParams, KeyStore};
    use tempfile::TempDir;

    fn keystore_in(dir: &TempDir) -> KeyStore {
        KeyStore::create(dir.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap()
    }

    #[test]
    fn a_kit_holds_the_key_and_the_steps() {
        let archive_dir = TempDir::new().unwrap();
        let store = keystore_in(&archive_dir);
        let archive = archive_dir.path().join("bonk.db");
        let kit = TempDir::new().unwrap();

        let report = write(store.path(), &archive, kit.path(), "2026-11-03T12:00:00Z").unwrap();
        assert_eq!(report.directory, kit.path());
        assert_eq!(fs::read(&report.key).unwrap(), fs::read(store.path()).unwrap());

        let steps = fs::read_to_string(&report.instructions).unwrap();
        assert!(steps.contains("another medium"), "{steps}");
        assert!(steps.contains("bonk.db"), "{steps}");
        assert!(steps.contains("2026-11-03T12:00:00Z"), "{steps}");
    }

    #[test]
    fn a_kit_refuses_to_sit_beside_the_archive() {
        let dir = TempDir::new().unwrap();
        let store = keystore_in(&dir);
        let archive = dir.path().join("bonk.db");

        let error = write(store.path(), &archive, dir.path().join("kit"), "2026-11-03T12:00:00Z").unwrap_err();
        assert!(matches!(error, StoreError::Rejected(_)), "{error}");
        assert!(
            !dir.path().join("kit").join("keystore.bin").exists(),
            "a refused kit copies nothing"
        );
    }
}
