//! Forward-only migrations with recorded checksums.
//!
//! A migration is applied exactly once, inside a transaction, and its SQL is
//! hashed. If the file changes after it has been applied, opening the database
//! fails loudly instead of pretending the schema is what the source says it is.
//!
//! If the database has a version newer than this binary knows about, opening
//! fails too: guessing is how archives get corrupted.

use std::fmt::Write as _;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::error::StoreError;

/// One migration.
pub struct Migration {
    /// Monotonic version number.
    pub version: i64,
    /// File stem, for messages.
    pub name: &'static str,
    /// SQL source.
    pub sql: &'static str,
    /// Reverse SQL, when a safe rollback exists.
    pub down: Option<&'static str>,
}

/// All compiled-in migrations, in order.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init",
        sql: include_str!("../migrations/0001_init.sql"),
        down: None,
    },
    Migration {
        version: 2,
        name: "0002_documents",
        sql: include_str!("../migrations/0002_documents.sql"),
        down: None,
    },
    Migration {
        version: 3,
        name: "0003_suppression",
        sql: include_str!("../migrations/0003_suppression.sql"),
        down: Some(include_str!("../migrations/0003_suppression_down.sql")),
    },
    Migration {
        version: 4,
        name: "0004_search_order",
        sql: include_str!("../migrations/0004_search_order.sql"),
        down: Some(include_str!("../migrations/0004_search_order_down.sql")),
    },
    Migration {
        version: 5,
        name: "0005_search_order_index",
        sql: include_str!("../migrations/0005_search_order_index.sql"),
        down: Some(include_str!("../migrations/0005_search_order_index_down.sql")),
    },
    Migration {
        version: 6,
        name: "0006_find_index",
        sql: include_str!("../migrations/0006_find_index.sql"),
        down: Some(include_str!("../migrations/0006_find_index_down.sql")),
    },
    Migration {
        version: 7,
        name: "0007_template_digest",
        sql: include_str!("../migrations/0007_template_digest.sql"),
        down: Some(include_str!("../migrations/0007_template_digest_down.sql")),
    },
    Migration {
        version: 8,
        name: "0008_draft_template",
        sql: include_str!("../migrations/0008_draft_template.sql"),
        down: Some(include_str!("../migrations/0008_draft_template_down.sql")),
    },
    Migration {
        version: 9,
        name: "0009_artefact_blob_index",
        sql: include_str!("../migrations/0009_artefact_blob_index.sql"),
        down: Some(include_str!("../migrations/0009_artefact_blob_index_down.sql")),
    },
    Migration {
        version: 10,
        name: "0010_blob_sha256_index",
        sql: include_str!("../migrations/0010_blob_sha256_index.sql"),
        down: Some(include_str!("../migrations/0010_blob_sha256_index_down.sql")),
    },
    Migration {
        version: 11,
        name: "0011_ledger",
        // Reversible only while the ledger is empty; the down migration refuses
        // to drop postings, because a posting is evidence for a filed return.
        sql: include_str!("../migrations/0011_ledger.sql"),
        down: Some(include_str!("../migrations/0011_ledger_down.sql")),
    },
    Migration {
        version: 12,
        name: "0012_vat_return",
        sql: include_str!("../migrations/0012_vat_return.sql"),
        down: Some(include_str!("../migrations/0012_vat_return_down.sql")),
    },
    Migration {
        version: 13,
        name: "0013_open_items",
        sql: include_str!("../migrations/0013_open_items.sql"),
        down: Some(include_str!("../migrations/0013_open_items_down.sql")),
    },
    Migration {
        version: 14,
        name: "0014_bank",
        sql: include_str!("../migrations/0014_bank.sql"),
        down: Some(include_str!("../migrations/0014_bank_down.sql")),
    },
    Migration {
        version: 15,
        name: "0015_allocation_sign",
        sql: include_str!("../migrations/0015_allocation_sign.sql"),
        down: Some(include_str!("../migrations/0015_allocation_sign_down.sql")),
    },
];

/// SHA-256 of the migration source, hex encoded.
pub fn checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub fn rollback_to(conn: &mut Connection, target: i64) -> Result<usize, StoreError> {
    let versions = {
        let mut statement = conn.prepare("SELECT version FROM schema_migration ORDER BY version DESC")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    let mut to_rollback = Vec::new();
    for version in versions {
        if version <= target {
            break;
        }
        let migration =
            MIGRATIONS
                .iter()
                .find(|migration| migration.version == version)
                .ok_or(StoreError::Migration {
                    version,
                    name: "unknown",
                    detail: "applied migration is not present in this binary".to_owned(),
                })?;
        if migration.down.is_none() {
            return Err(StoreError::Migration {
                version,
                name: migration.name,
                detail: "no down migration; rollback refused".to_owned(),
            });
        }
        to_rollback.push(migration);
    }

    if to_rollback.is_empty() {
        return Ok(0);
    }

    let transaction = conn.transaction()?;
    for migration in &to_rollback {
        let down = migration.down.unwrap_or_default();
        transaction.execute_batch(down).map_err(|error| StoreError::Migration {
            version: migration.version,
            name: migration.name,
            detail: error.to_string(),
        })?;
        transaction.execute("DELETE FROM schema_migration WHERE version = ?1", [migration.version])?;
    }
    transaction.commit()?;
    Ok(to_rollback.len())
}

pub(crate) fn apply(conn: &mut Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migration (
             version    INTEGER PRIMARY KEY,
             name       TEXT NOT NULL,
             checksum   TEXT NOT NULL,
             applied_at TEXT NOT NULL
         );",
    )?;

    let newest_known = MIGRATIONS.last().map(|migration| migration.version).unwrap_or(0);
    let applied_newest: Option<i64> =
        conn.query_row("SELECT max(version) FROM schema_migration", [], |row| row.get(0))?;
    if let Some(version) = applied_newest
        && version > newest_known
    {
        return Err(StoreError::Migration {
            version,
            name: "unknown",
            detail: "database was created by a newer version of the application".to_owned(),
        });
    }

    for migration in MIGRATIONS {
        let stored: Option<String> = conn
            .query_row(
                "SELECT checksum FROM schema_migration WHERE version = ?1",
                [migration.version],
                |row| row.get(0),
            )
            .optional()?;

        let expected = checksum(migration.sql);
        if let Some(stored) = stored {
            if stored != expected {
                return Err(StoreError::Migration {
                    version: migration.version,
                    name: migration.name,
                    detail: "migration source changed after it was applied".to_owned(),
                });
            }
            continue;
        }

        let transaction = conn.transaction()?;
        transaction
            .execute_batch(migration.sql)
            .map_err(|error| StoreError::Migration {
                version: migration.version,
                name: migration.name,
                detail: error.to_string(),
            })?;
        transaction.execute(
            "INSERT INTO schema_migration (version, name, checksum, applied_at)
             VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![migration.version, migration.name, expected],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_are_stable_and_distinct() {
        assert_eq!(checksum("select 1"), checksum("select 1"));
        assert_ne!(checksum("select 1"), checksum("select 2"));
        assert_eq!(checksum("select 1").len(), 64);
    }

    #[test]
    fn rollback_is_preflighted_and_refuses_irreversible_migrations() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        assert_eq!(current_version(&conn), 15);

        assert!(rollback_to(&mut conn, 1).is_err());
        assert_eq!(
            current_version(&conn),
            15,
            "a refused rollback must not partially apply"
        );

        assert_eq!(
            rollback_to(&mut conn, 2).unwrap(),
            13,
            "0015 through 0003 all declare a down"
        );
        assert_eq!(current_version(&conn), 2);
    }

    #[test]
    fn the_ledger_refuses_to_roll_back_once_it_holds_postings() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        conn.execute_batch(
            "INSERT INTO seller (id, legal_name, default_currency, created_at, updated_at)
                 VALUES ('s1', 'Seller', 'EUR', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
             INSERT INTO fiscal_period (id, seller_id, year, period_no, starts_on, ends_on, status)
                 VALUES ('p1', 's1', 2026, 1, '2026-01-01', '2026-01-31', 'open');
             INSERT INTO account (code, name, kind, updated_at)
                 VALUES ('1300', 'Debiteuren', 'asset', '2026-01-01T00:00:00Z');
             INSERT INTO journal_entry
                 (id, journal_code, period_id, number, entry_date_local, description,
                  source_kind, posting_profile_id, rule_version, currency,
                  created_at, created_by, content_hash)
                 VALUES ('e1', 'MEM', 'p1', '2026-0001', '2026-01-01', 'Manual',
                         'manual', 'test', '1', 'EUR', '2026-01-01T00:00:00Z', 'test', 'x');",
        )
        .unwrap();

        let error = rollback_to(&mut conn, 2).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("no_postings"), "unexpected error: {text}");
        assert_eq!(
            current_version(&conn),
            15,
            "nothing may be dropped before the guard trips"
        );
    }

    fn current_version(conn: &Connection) -> i64 {
        conn.query_row("SELECT max(version) FROM schema_migration", [], |row| row.get(0))
            .unwrap()
    }
}
