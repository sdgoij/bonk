//! Named find-panel filters.
//!
//! A saved search is convenience, not evidence: it can be renamed, edited, and
//! deleted, which is why it is the one table in the archive without immutability
//! triggers. It holds only what the panel can express — words, a side, a status —
//! so a stored view can always be applied exactly as it was saved.

use crux::{SavedSearchId, SellerId};
use rusqlite::{Connection, params};

use crate::error::StoreError;
use crate::repo::{Direction, DocumentStatus};

/// A named set of find-panel filters.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SavedSearch {
    /// Identity. Stable across renames and edits because it is the key the row
    /// was created with; the name is what the user sees.
    pub id: SavedSearchId,
    /// What the user calls it.
    pub name: String,
    /// Words, exactly as typed. Empty means "everything the filters allow".
    pub query: String,
    /// Restrict to one side of the books.
    pub direction: Option<Direction>,
    /// Restrict to one status.
    pub status: Option<DocumentStatus>,
}

/// Creates a saved search, or replaces the filters of the one with that name.
///
/// Replacing keeps the original identity: renaming by saving under a new name is
/// a different thing from editing the filters of an existing one.
pub fn save(conn: &Connection, owner: SellerId, search: &SavedSearch, now: &str) -> Result<(), StoreError> {
    let name = search.name.trim();
    if name.is_empty() {
        return Err(StoreError::Rejected("a saved search needs a name".to_owned()));
    }
    conn.execute(
        "INSERT INTO saved_search
             (id, owner_id, name, query, direction, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(owner_id, name) DO UPDATE SET
             query = excluded.query,
             direction = excluded.direction,
             status = excluded.status,
             updated_at = excluded.updated_at",
        params![
            search.id.to_string(),
            owner.to_string(),
            name,
            search.query.trim(),
            search.direction.map(Direction::code),
            search.status.map(DocumentStatus::code),
            now,
        ],
    )?;
    Ok(())
}

/// Lists saved searches by name.
pub fn list(conn: &Connection, owner: SellerId) -> Result<Vec<SavedSearch>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, name, query, direction, status
         FROM saved_search
         WHERE owner_id = ?1
         ORDER BY name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([owner.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut searches = Vec::new();
    for row in rows {
        let (id, name, query, direction, status) = row?;
        searches.push(SavedSearch {
            id: SavedSearchId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
            name,
            query,
            direction: direction
                .as_deref()
                .map(|code| Direction::parse(code).ok_or_else(|| StoreError::Data(format!("unknown direction {code}"))))
                .transpose()?,
            status: status
                .as_deref()
                .map(|code| {
                    DocumentStatus::parse(code).ok_or_else(|| StoreError::Data(format!("unknown status {code}")))
                })
                .transpose()?,
        });
    }
    Ok(searches)
}

/// Deletes one by name. Returns false when there was nothing to delete, which is
/// not an error: two windows can race.
pub fn delete(conn: &Connection, owner: SellerId, name: &str) -> Result<bool, StoreError> {
    let removed = conn.execute(
        "DELETE FROM saved_search WHERE owner_id = ?1 AND name = ?2",
        params![owner.to_string(), name.trim()],
    )?;
    Ok(removed > 0)
}
