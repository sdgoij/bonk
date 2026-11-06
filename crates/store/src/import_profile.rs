//! Stored CSV import profiles.
//!
//! A CSV export carries no schema, so the mapping from columns to meaning is
//! something a person states once and the archive keeps. This module is the
//! storage half only: it does not know what a profile says, and it does not parse
//! one. The importer parses on the way in and on every use, so a row that was
//! written by something other than the command line still fails loudly at import
//! rather than producing amounts nobody can explain.
//!
//! Two things are checked here, because they are about identity rather than
//! meaning: a name has to be usable as a key and readable in a list, and an
//! account may only name a profile that exists. The second is not a foreign key,
//! because `bank_account.import_profile` predates this table; enforcing it in code
//! keeps a removal from leaving an account pointing at nothing.

use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::audit;
use crate::error::StoreError;

/// A profile as the archive holds it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StoredProfile {
    /// The name a command asks for.
    pub name: String,
    /// SHA-256 of the stored text, so a change is visible without diffing it.
    pub sha256: String,
    /// When it was last written.
    pub updated_at: String,
}

/// Stores a profile, replacing one of the same name.
///
/// The text is kept exactly as it was given: a profile is something a person
/// wrote and may have commented, and rewriting it would make the stored profile
/// and the file in the repository disagree.
pub fn put(
    conn: &rusqlite::Connection,
    name: &str,
    toml: &str,
    actor: &str,
    now: &str,
) -> Result<StoredProfile, StoreError> {
    let name = check_name(name)?;
    let sha256 = audit::sha256_hex(toml.as_bytes());

    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO import_profile (name, toml, sha256, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(name) DO UPDATE SET
             toml = excluded.toml,
             sha256 = excluded.sha256,
             updated_at = excluded.updated_at",
        params![name, toml, sha256, now],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "import_profile_stored",
        "import_profile",
        name,
        serde_json::json!({ "sha256": sha256 }),
    )?;
    transaction.commit()?;

    Ok(StoredProfile {
        name: name.to_owned(),
        sha256,
        updated_at: now.to_owned(),
    })
}

/// Every stored profile, by name.
pub fn list(conn: &rusqlite::Connection) -> Result<Vec<StoredProfile>, StoreError> {
    let mut statement = conn.prepare("SELECT name, sha256, updated_at FROM import_profile ORDER BY name")?;
    let rows = statement.query_map([], |row| {
        Ok(StoredProfile {
            name: row.get(0)?,
            sha256: row.get(1)?,
            updated_at: row.get(2)?,
        })
    })?;
    let mut profiles = Vec::new();
    for row in rows {
        profiles.push(row?);
    }
    Ok(profiles)
}

/// The text of one profile, when it exists.
pub fn load(conn: &rusqlite::Connection, name: &str) -> Result<Option<String>, StoreError> {
    conn.query_row("SELECT toml FROM import_profile WHERE name = ?1", [name], |row| {
        row.get(0)
    })
    .optional()
    .map_err(StoreError::from)
}

/// Removes a profile.
///
/// An account that still names it keeps the name and is reported as pointing at
/// nothing when its profile is looked up; silently clearing the column instead
/// would hide that a rule the operator wrote no longer exists.
pub fn remove(conn: &rusqlite::Connection, name: &str, actor: &str, now: &str) -> Result<(), StoreError> {
    let name = check_name(name)?;
    let transaction = conn.unchecked_transaction()?;
    let removed = transaction.execute("DELETE FROM import_profile WHERE name = ?1", [name])?;
    if removed == 0 {
        return Err(StoreError::NotFound(format!("import profile {name}")));
    }
    audit::append(
        &transaction,
        now,
        actor,
        "import_profile_removed",
        "import_profile",
        name,
        serde_json::json!({}),
    )?;
    transaction.commit()?;
    Ok(())
}

/// The profile an account names, if it names one that still exists.
///
/// Returns the name and its text: the caller needs both, because the name goes
/// into what it reports and the text is what it reads the file with.
pub fn for_account(conn: &rusqlite::Connection, account_id: &str) -> Result<Option<(String, String)>, StoreError> {
    let name: Option<String> = conn
        .query_row(
            "SELECT import_profile FROM bank_account WHERE id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("bank account {account_id}")))?;

    let Some(name) = name else {
        return Ok(None);
    };
    match load(conn, &name)? {
        Some(toml) => Ok(Some((name, toml))),
        None => Err(StoreError::Rejected(format!(
            "bank account {account_id} names import profile {name}, which no longer exists"
        ))),
    }
}

/// Points an account at a profile, or at nothing.
pub fn set_for_account(
    conn: &rusqlite::Connection,
    account_id: &str,
    name: Option<&str>,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    if let Some(name) = name
        && load(conn, name)?.is_none()
    {
        return Err(StoreError::NotFound(format!("import profile {name}")));
    }

    let transaction = conn.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE bank_account SET import_profile = ?2 WHERE id = ?1",
        params![account_id, name],
    )?;
    if changed == 0 {
        return Err(StoreError::NotFound(format!("bank account {account_id}")));
    }
    audit::append(
        &transaction,
        now,
        actor,
        "bank_account_profile_set",
        "bank_account",
        account_id,
        serde_json::json!({ "import_profile": name }),
    )?;
    transaction.commit()?;
    Ok(())
}

/// A name that may be used as a key and read in a list.
fn check_name(name: &str) -> Result<&str, StoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(StoreError::Rejected("a profile needs a name".to_owned()));
    }
    if trimmed != name || name.chars().any(|character| character.is_control()) {
        return Err(StoreError::Rejected(format!(
            "{name:?} is not usable as a profile name: no leading or trailing space, and no control characters"
        )));
    }
    Ok(name)
}
