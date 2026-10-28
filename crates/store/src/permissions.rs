//! Who is allowed to read the archive.
//!
//! Encryption protects the bytes; these modes protect the files. On Unix a new
//! file gets whatever the umask allows, and a umask of `022` — the common default
//! — leaves it readable by every account on the machine. So a directory we create
//! for the archive is `0700`, and the keystore, the database, and backups are
//! `0600`.
//!
//! Two deliberate choices:
//!
//! - An existing directory is left alone. `--db /tmp/x.db` must not chmod `/tmp`,
//!   and a directory the user made is theirs to decide about.
//! - Opening an archive tightens what is already there, best effort. A file copied
//!   from a backup or a shared folder arrives with whatever mode it had, and a
//!   keystore the whole machine can read defeats the point.
//!
//! Windows needs none of this in the same way: a file under `%APPDATA%` inherits
//! an ACL that grants the user, the system, and administrators. Expressing "only
//! the owner" there means an explicit DACL through an external crate or unsafe
//! code, which this crate does not do, so these calls are no-ops and the threat
//! model says so instead of implying a guarantee.

use std::io;
use std::path::Path;

/// The mode a file only its owner can read.
#[cfg(unix)]
pub const FILE_MODE: u32 = 0o600;

/// The mode a directory only its owner can enter.
#[cfg(unix)]
pub const DIRECTORY_MODE: u32 = 0o700;

/// Creates a directory only its owner can enter, and leaves an existing one as
/// it is.
///
/// Nested paths get the same mode on every level this call creates.
pub fn private_directory(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(DIRECTORY_MODE)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

/// Restricts a file to its owner. A missing file is not an error, because the
/// write-ahead log and shared-memory files come and go.
pub fn restrict(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !path.exists() {
            return Ok(());
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(FILE_MODE))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Restricts a file to its owner, ignoring a failure.
///
/// Used on paths the platform may not let us change — a file on a mounted share,
/// for instance — because failing to open an archive over a permission nudge
/// would be worse than the nudge not landing.
pub fn restrict_quietly(path: &Path) {
    let _ = restrict(path);
}

/// Restricts a database and the files SQLite keeps beside it.
///
/// SQLite creates `-wal` and `-shm` with the same permissions as the database
/// file, so setting the database is enough for files created later.
pub fn restrict_archive(path: &Path) {
    restrict_quietly(path);
    for suffix in ["-wal", "-shm"] {
        restrict_quietly(Path::new(&format!("{}{suffix}", path.display())));
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_new_directory_is_private_and_an_existing_one_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("a").join("b");
        private_directory(&nested).unwrap();
        let mode = std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, DIRECTORY_MODE);

        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_directory(&nested).unwrap();
        let mode = std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "a directory this did not create is not ours to tighten");
    }

    #[cfg(unix)]
    #[test]
    fn restricting_a_file_narrows_its_mode_and_tolerates_a_missing_one() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("keystore.bin");
        std::fs::write(&path, b"secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        restrict(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, FILE_MODE);

        assert!(restrict(&root.path().join("not-there")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_long_path_with_sidecars_is_restricted_too() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("bonk.db");
        std::fs::write(&database, b"x").unwrap();
        std::fs::write(root.path().join("bonk.db-wal"), b"x").unwrap();

        restrict_archive(&database);

        for path in [&database, &root.path().join("bonk.db-wal")] {
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, FILE_MODE);
        }
    }
}
