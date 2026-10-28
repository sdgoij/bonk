#![allow(clippy::unwrap_used)]
//! Chain heads kept outside the archive: the check a rewritten chain cannot pass.

use crux::{Address, CountryCode, CustomerId, Party};
use crux::{
    Currency, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Quantity, SellerId, TaxTreatment,
    UnitCode, VatRate,
};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, anchor, insert_customer, insert_draft,
    insert_seller, issue_draft,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-27T12:00:00Z";

fn fixture() -> (TempDir, Database, SellerId, CustomerId) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

    let seller_id = SellerId::new();
    let customer_id = CustomerId::new();
    insert_seller(
        database.connection(),
        &SellerRecord {
            id: seller_id,
            party: Party::new("Seller BV", Address::new("Amsterdam", CountryCode::NL)),
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();
    insert_customer(
        database.connection(),
        &CustomerRecord {
            id: customer_id,
            party: Party::new("Customer BV", Address::new("Rotterdam", CountryCode::NL)),
        },
        NOW,
    )
    .unwrap();

    (dir, database, seller_id, customer_id)
}

fn issue(database: &mut Database, seller_id: SellerId, customer_id: CustomerId) -> DocumentId {
    let mut draft = InvoiceDraft::new(
        seller_id,
        customer_id,
        Currency::EUR,
        LocalDate::parse_iso("2026-10-27").unwrap(),
    );
    draft.lines.push(DraftLine {
        id: LineId::new(),
        position: 1,
        name: "Consulting".to_owned(),
        description: None,
        quantity: Quantity::from_units(1).unwrap(),
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    });
    insert_draft(database.connection(), &draft, NOW).unwrap();
    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "device-1",
            renderer_version: "test-renderer-1",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap()
    .id
}

#[test]
fn a_checkpoint_is_written_and_checked() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");

    let recorded = anchor::record(&path, database.connection(), NOW).unwrap();
    assert!(recorded.appended);
    assert_eq!(recorded.checked, 0);
    assert!(recorded.anchor.seq > 0);

    let checked = anchor::check(&path, database.connection()).unwrap();
    assert_eq!(checked.count, 1);
    assert_eq!(checked.newest, recorded.anchor);
}

#[test]
fn recording_the_same_head_twice_writes_one_line() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");

    anchor::record(&path, database.connection(), NOW).unwrap();
    let second = anchor::record(&path, database.connection(), NOW).unwrap();

    assert!(!second.appended, "nothing changed, so there is nothing new to record");
    assert_eq!(anchor::list(&path).unwrap().len(), 1);
}

#[test]
fn a_later_head_is_appended_and_earlier_ones_still_agree() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");
    let first = anchor::record(&path, database.connection(), NOW).unwrap();

    issue(&mut database, seller_id, customer_id);
    let second = anchor::record(&path, database.connection(), NOW).unwrap();

    assert!(second.appended);
    assert!(second.anchor.seq > first.anchor.seq);
    let checked = anchor::check(&path, database.connection()).unwrap();
    assert_eq!(checked.count, 2);
    assert_eq!(checked.newest, second.anchor);
}

#[test]
fn a_rewritten_chain_is_caught() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");
    anchor::record(&path, database.connection(), NOW).unwrap();

    // Build a second archive with the same code and swap its chain in. This is
    // what an attacker with the key would produce: a chain that verifies against
    // itself, which no amount of re-hashing can distinguish from the original.
    let (_other_dir, mut other, other_seller, other_customer) = fixture();
    issue(&mut other, other_seller, other_customer);
    replace_chain(database.connection(), other.connection());
    assert!(
        store::audit::verify_chain(database.connection()).is_ok(),
        "the replacement chain verifies, which is why an outside record is needed"
    );

    let error = anchor::check(&path, database.connection()).unwrap_err();
    assert!(error.to_string().contains("rewritten"), "{error}");
}

/// One audit row, as copied between archives.
struct AuditRow {
    seq: i64,
    id: String,
    at: String,
    actor: String,
    kind: String,
    subject_kind: String,
    subject_id: String,
    payload: String,
    prev: String,
    hash: String,
}

/// Replaces every audit row in `target` with the ones from `source`.
fn replace_chain(target: &rusqlite::Connection, source: &rusqlite::Connection) {
    let mut statement = source
        .prepare(
            "SELECT seq, id, at, actor, kind, subject_kind, subject_id, payload_json, prev_hash, hash
             FROM audit_event ORDER BY seq",
        )
        .unwrap();
    let rows: Vec<AuditRow> = statement
        .query_map([], |row| {
            Ok(AuditRow {
                seq: row.get(0)?,
                id: row.get(1)?,
                at: row.get(2)?,
                actor: row.get(3)?,
                kind: row.get(4)?,
                subject_kind: row.get(5)?,
                subject_id: row.get(6)?,
                payload: row.get(7)?,
                prev: row.get(8)?,
                hash: row.get(9)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    target.execute("DELETE FROM audit_event", []).unwrap();
    for row in rows {
        target
            .execute(
                "INSERT INTO audit_event
                     (seq, id, at, actor, kind, subject_kind, subject_id, payload_json, prev_hash, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    row.seq,
                    row.id,
                    row.at,
                    row.actor,
                    row.kind,
                    row.subject_kind,
                    row.subject_id,
                    row.payload,
                    row.prev,
                    row.hash
                ],
            )
            .unwrap();
    }
}

#[test]
fn a_truncated_chain_is_caught() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");
    let recorded = anchor::record(&path, database.connection(), NOW).unwrap();

    database
        .connection()
        .execute("DELETE FROM audit_event WHERE seq = ?1", [recorded.anchor.seq])
        .unwrap();

    let error = anchor::check(&path, database.connection()).unwrap_err();
    assert!(error.to_string().contains("removed"), "{error}");
}

#[test]
fn recording_refuses_a_file_that_no_longer_agrees() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");
    let recorded = anchor::record(&path, database.connection(), NOW).unwrap();

    database
        .connection()
        .execute("DELETE FROM audit_event WHERE seq = ?1", [recorded.anchor.seq])
        .unwrap();

    assert!(anchor::record(&path, database.connection(), NOW).is_err());
    assert_eq!(
        anchor::list(&path).unwrap().len(),
        1,
        "a contradicted file is not extended"
    );
}

#[test]
fn an_empty_or_missing_file_is_not_a_pass() {
    let (dir, database, ..) = fixture();
    let path = dir.path().join("nothing.txt");

    assert!(
        anchor::check(&path, database.connection()).is_err(),
        "no file means no evidence"
    );

    std::fs::write(&path, "# only a comment\n").unwrap();
    assert!(anchor::check(&path, database.connection()).is_err());
}

#[test]
fn a_hand_edited_file_is_read() {
    let (dir, mut database, seller_id, customer_id) = fixture();
    issue(&mut database, seller_id, customer_id);
    let path = dir.path().join("heads.txt");
    let recorded = anchor::record(&path, database.connection(), NOW).unwrap();

    // A user adding a note between runs must not break the check.
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.insert_str(0, "# heads of the sample archive\n\n");
    std::fs::write(&path, text).unwrap();

    let checked = anchor::check(&path, database.connection()).unwrap();
    assert_eq!(checked.newest, recorded.anchor);
}
