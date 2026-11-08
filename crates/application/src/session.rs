//! The unlocked archive: one database and the keystore that wraps its key.
//!
//! A front end supplies the paths it wants; this crate has no platform data
//! directory. The data key is kept for the life of the session so a verified
//! backup can be written, and so a worker can open a second connection without
//! asking the user for the passphrase again.

use std::path::Path;

use store::{Database, KdfParams, KeyStore, StoreError};
use zeroize::Zeroizing;

use crate::dto::{CustomerDto, CustomerImportDto, DraftDto, DraftTotalsDto, FindingDto, Summary};

/// An unlocked application session: one database, one keystore.
pub struct Archive {
    database: Database,
    database_path: std::path::PathBuf,
    keystore_path: std::path::PathBuf,
    /// The data key, kept so the window can write a verified backup without
    /// asking for the passphrase again. It is the same key SQLCipher is using.
    dek: Zeroizing<[u8; 32]>,
}

impl Archive {
    /// Creates a new keystore and database. Refuses to overwrite either.
    pub fn create(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
    ) -> Result<Self, StoreError> {
        Self::create_with_params(database_path, keystore_path, passphrase, KdfParams::DEFAULT)
    }

    fn create_with_params(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
        params: KdfParams,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        if database_path.exists() || keystore_path.exists() {
            return Err(StoreError::InvalidKeystore);
        }
        if let Some(parent) = database_path.parent() {
            store::permissions::private_directory(parent)?;
        }
        if let Some(parent) = keystore_path.parent() {
            store::permissions::private_directory(parent)?;
        }

        let keystore = KeyStore::create(&keystore_path, passphrase, params)?;
        let dek = Zeroizing::new(*keystore.dek());
        let database = Database::create(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Unlocks an existing database.
    pub fn unlock(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        let keystore = KeyStore::open(&keystore_path, passphrase)?;
        let dek = Zeroizing::new(*keystore.dek());
        let database = Database::open(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Dashboard data.
    pub fn summary(&self) -> Result<Summary, StoreError> {
        Ok(Summary {
            sellers: self.database.seller_count()?,
            drafts: self.database.draft_count()?,
            schema_version: self.database.schema_version()?,
        })
    }

    /// Lists customers, ordered by display name.
    pub fn list_customers(&self) -> Result<Vec<CustomerDto>, StoreError> {
        Ok(store::list_customers(self.database.connection())?
            .iter()
            .map(CustomerDto::from_record)
            .collect())
    }

    /// Creates a customer when the id is empty, otherwise updates one.
    pub fn save_customer(&self, customer: &CustomerDto) -> Result<CustomerDto, StoreError> {
        let now = store::now_rfc3339();
        if customer.id.is_empty() {
            let mut assigned = customer.clone();
            assigned.id = crux::CustomerId::new().to_string();
            let record = assigned
                .into_record()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            store::insert_customer(self.database.connection(), &record, &now)?;
            Ok(CustomerDto::from_record(&record))
        } else {
            let record = customer
                .clone()
                .into_record()
                .map_err(|error| StoreError::Data(error.to_string()))?;
            store::update_customer(self.database.connection(), &record, &now)?;
            Ok(CustomerDto::from_record(&record))
        }
    }

    /// Exports the customer table as CSV.
    pub fn customers_csv(&self) -> Result<String, StoreError> {
        let customers = store::list_customers(self.database.connection())?;
        Ok(crate::customers::export(&customers))
    }

    /// Imports customers from CSV, skipping parties already on file.
    pub fn import_customers_csv(&self, text: &str) -> Result<CustomerImportDto, StoreError> {
        let rows = crate::customers::parse(text).map_err(StoreError::Data)?;
        let report = crate::customers::import(&self.database, &rows, &store::now_rfc3339())?;
        Ok(CustomerImportDto {
            imported: report.imported as i64,
            skipped: report.skipped as i64,
            errors: report.errors,
        })
    }

    /// Lists drafts, most recently updated first.
    pub fn list_drafts(&self) -> Result<Vec<DraftDto>, StoreError> {
        store::list_drafts(self.database.connection())?
            .iter()
            .map(DraftDto::from_draft)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| StoreError::Data(error.to_string()))
    }

    /// Loads a draft by identity.
    pub fn load_draft(&self, id: &str) -> Result<DraftDto, StoreError> {
        let id = crux::DocumentId::parse_str(id).map_err(|error| StoreError::Data(error.to_string()))?;
        let draft = store::load_draft(self.database.connection(), id)?
            .ok_or_else(|| StoreError::NotFound(format!("draft {id}")))?;
        DraftDto::from_draft(&draft).map_err(|error| StoreError::Data(error.to_string()))
    }

    /// Creates or updates a draft and returns the stored shape.
    pub fn save_draft(&self, draft: &DraftDto) -> Result<DraftDto, StoreError> {
        let domain = draft
            .clone()
            .into_draft()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        store::save_draft(self.database.connection(), &domain, &store::now_rfc3339())?;
        DraftDto::from_draft(&domain).map_err(|error| StoreError::Data(error.to_string()))
    }

    /// Validates a draft against its seller and customer, hiding suppressions.
    pub fn validate_draft(&self, draft: &DraftDto) -> Result<Vec<FindingDto>, StoreError> {
        let domain = draft
            .clone()
            .into_draft()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        let seller = store::load_seller(self.database.connection(), domain.seller_id)?;
        let customer = store::load_customer(self.database.connection(), domain.customer_id)?;
        let mut findings = crux::validate_draft(&domain, &seller.party, &customer.party);
        let suppressions = store::list_suppressions(self.database.connection(), domain.id)?;
        findings.retain(|finding| {
            !suppressions
                .iter()
                .any(|suppression| suppression.code == finding.code && suppression.field == finding.field)
        });
        Ok(findings.iter().map(FindingDto::from).collect())
    }

    /// Records a reason for accepting a non-blocking finding.
    pub fn suppress_finding(&self, draft_id: &str, code: &str, field: &str, reason: &str) -> Result<(), StoreError> {
        let id = crux::DocumentId::parse_str(draft_id).map_err(|error| StoreError::Data(error.to_string()))?;
        let draft = store::load_draft(self.database.connection(), id)?
            .ok_or_else(|| StoreError::NotFound(format!("draft {id}")))?;
        let seller = store::load_seller(self.database.connection(), draft.seller_id)?;
        let customer = store::load_customer(self.database.connection(), draft.customer_id)?;
        let findings = crux::validate_draft(&draft, &seller.party, &customer.party);
        let Some(finding) = findings
            .iter()
            .find(|finding| finding.code == code && finding.field == field)
        else {
            return Err(StoreError::Rejected("that finding is no longer present".to_owned()));
        };
        if finding.severity == crux::Severity::Blocking {
            return Err(StoreError::Rejected(
                "a blocking finding cannot be suppressed".to_owned(),
            ));
        }
        store::save_suppression(
            self.database.connection(),
            id,
            code,
            field,
            reason,
            "mobile",
            &store::now_rfc3339(),
        )
    }

    /// Totals for the editor preview, computed by the domain.
    pub fn draft_totals(&self, draft: &DraftDto) -> Result<DraftTotalsDto, StoreError> {
        let domain = draft
            .clone()
            .into_draft()
            .map_err(|error| StoreError::Data(error.to_string()))?;
        if domain.lines.is_empty() {
            // The domain refuses to sum an empty set because it has no currency,
            // but a draft knows its currency, so an empty draft is zero.
            return Ok(DraftTotalsDto {
                net_micros: 0,
                tax_micros: 0,
                gross_micros: 0,
            });
        }
        let totals = crux::calculate_totals(&domain.lines).map_err(|error| StoreError::Data(error.to_string()))?;
        Ok(DraftTotalsDto::from(&totals))
    }

    /// The default seller's id, creating a placeholder party when there is none.
    ///
    /// The mobile shell has no seller setup screen yet and a draft needs an
    /// owner; the settings work replaces this party.
    pub fn ensure_seller(&self) -> Result<String, StoreError> {
        if let Some(seller) = store::first_seller(self.database.connection())? {
            return Ok(seller.id.to_string());
        }
        let seller = store::SellerRecord {
            id: crux::SellerId::new(),
            party: crux::Party::new("My company", crux::Address::new("Amsterdam", crux::CountryCode::NL)),
            default_currency: crux::Currency::EUR,
            default_locale: "en".to_owned(),
        };
        store::insert_seller(self.database.connection(), &seller, &store::now_rfc3339())?;
        Ok(seller.id.to_string())
    }

    /// A new invoice draft for a customer, dated today, saved immediately.
    pub fn new_draft(&self, customer_id: &str) -> Result<DraftDto, StoreError> {
        let seller =
            crux::SellerId::parse_str(&self.ensure_seller()?).map_err(|error| StoreError::Data(error.to_string()))?;
        let customer = crux::CustomerId::parse_str(customer_id).map_err(|error| StoreError::Data(error.to_string()))?;
        let draft = crux::InvoiceDraft::new(seller, customer, crux::Currency::EUR, crux::LocalDate::today_utc());
        store::save_draft(self.database.connection(), &draft, &store::now_rfc3339())?;
        DraftDto::from_draft(&draft).map_err(|error| StoreError::Data(error.to_string()))
    }

    /// Runs SQLite's integrity check on the open database.
    pub fn integrity_check(&self) -> Result<(), StoreError> {
        self.database.integrity_check()
    }

    /// The data key, for a second connection that must run off the UI thread.
    ///
    /// The health check re-hashes the whole archive and cannot do that on the
    /// frame; SQLite allows a reader beside the window's writer, so the worker
    /// opens its own connection behind the same key. Callers copy it into a
    /// `Zeroizing` buffer that is wiped when the worker is done.
    pub fn dek(&self) -> &[u8; 32] {
        &self.dek
    }

    /// The open database, for application-level commands.
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Mutable access, required for issuing.
    pub fn database_mut(&mut self) -> &mut Database {
        &mut self.database
    }

    /// Path of the open database.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Path of the keystore.
    pub fn keystore_path(&self) -> &Path {
        &self.keystore_path
    }

    /// Adds or replaces the recovery key, returning the secret to show once.
    ///
    /// The passphrase is required again rather than taken from the unlock: a
    /// credential change should not ride on a window that has been left open.
    pub fn add_recovery_key(&self, passphrase: &str) -> Result<String, StoreError> {
        let mut keystore = KeyStore::open(self.keystore_path(), passphrase)?;
        let key = keystore.enable_recovery(passphrase)?;
        if let Err(error) = store::audit::record_recovery_key(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
        ) {
            // The key is written but unrecorded. Saying so is better than
            // hiding it, and adding a key again replaces it with one that is
            // recorded.
            return Err(StoreError::Rejected(format!(
                "the recovery key was written, but recording it in the audit chain failed: {error}. \
                 Add a recovery key again to replace it."
            )));
        }
        Ok(key)
    }

    /// Rewraps the data key under a new passphrase.
    pub fn change_passphrase(&self, current: &str, new: &str) -> Result<(), StoreError> {
        let mut keystore = KeyStore::open(self.keystore_path(), current)?;
        keystore.change_passphrase(new)?;
        if let Err(error) = store::audit::record_passphrase_change(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
        ) {
            return Err(StoreError::Rejected(format!(
                "the passphrase was changed, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(())
    }

    /// Keeps the data key in this device's credential store, so the archive can
    /// be opened without the passphrase.
    ///
    /// The passphrase is required again: trusting a machine should not be
    /// something an unattended window can do. The decision goes into the audit
    /// chain, because a second way in is evidence about the archive.
    pub fn remember_device(&self, passphrase: &str, store: &dyn store::DeviceStore) -> Result<(), StoreError> {
        let keystore = KeyStore::open(self.keystore_path(), passphrase)?;
        store::device::remember(store, self.keystore_path(), keystore.dek())?;
        if let Err(error) = store::audit::record_device_trust(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &self.keystore_path().display().to_string(),
        ) {
            return Err(StoreError::Rejected(format!(
                "the key was stored for this device, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(())
    }

    /// Stops this device from keeping the key. False when it did not have one.
    pub fn forget_device(&self, store: &dyn store::DeviceStore) -> Result<bool, StoreError> {
        let forgotten = store::device::forget(store, self.keystore_path())?;
        if forgotten
            && let Err(error) = store::audit::record_device_forget(
                self.database.connection(),
                &store::now_rfc3339(),
                "gui",
                &self.keystore_path().display().to_string(),
            )
        {
            return Err(StoreError::Rejected(format!(
                "the key was forgotten by this device, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(forgotten)
    }

    /// Records that this device's retained key was revoked, for example by an
    /// enrolment change that invalidated it.
    ///
    /// The record needs the open archive, so the host defers it until the next
    /// successful unlock; this method is that deferred write.
    pub fn record_device_revocation(&self) -> Result<(), StoreError> {
        store::audit::record_device_forget(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &self.keystore_path().display().to_string(),
        )
    }

    /// True when this device holds a key for the archive at these paths.
    ///
    /// This asks whether an entry exists without reading it, so a host whose read
    /// prompts for authentication does not show a prompt just to decide whether
    /// the device route should be offered.
    pub fn device_ready(keystore_path: impl AsRef<Path>, store: &dyn store::DeviceStore) -> bool {
        matches!(store::device::exists(store, keystore_path.as_ref()), Ok(true))
    }

    /// Opens the archive with the key this device kept.
    ///
    /// The keystore is not consulted: the point of the feature is that the
    /// operating system's login is enough.
    pub fn unlock_with_device(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        store: &dyn store::DeviceStore,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        let Some(dek) = store::device::recall(store, &keystore_path)? else {
            return Err(StoreError::Rejected(
                "this device has no key for that archive; unlock with the passphrase once and remember it".to_owned(),
            ));
        };
        let database = Database::open_with_key(&database_path, &dek)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Replaces a forgotten passphrase using the recovery key.
    ///
    /// Returns the session and the replacement recovery key. The key that was
    /// used is spent: it has now been typed somewhere, and leaving it valid would
    /// keep a second route open.
    pub fn reset_passphrase(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        recovery_key: &str,
        new_passphrase: &str,
    ) -> Result<(Self, String), StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        // Open the database first: a wrong key or a mismatched archive must
        // leave the keystore exactly as it was.
        let mut keystore = KeyStore::open_with_recovery(&keystore_path, recovery_key).map_err(|error| match error {
            StoreError::WrongPassphrase => {
                StoreError::Rejected("that recovery key does not open this archive".to_owned())
            }
            other => other,
        })?;
        let database = Database::open(&database_path, &keystore)?;
        let dek = Zeroizing::new(*keystore.dek());

        keystore.change_passphrase(new_passphrase)?;
        let replacement = keystore.enable_recovery(new_passphrase)?;
        if let Err(error) = store::audit::record_passphrase_reset(
            database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
            true,
        ) {
            return Err(StoreError::Rejected(format!(
                "the passphrase was reset, but recording it in the audit chain failed: {error}. \
                 The new recovery key is {replacement}; write it down now."
            )));
        }

        Ok((
            Self {
                database,
                database_path,
                keystore_path,
                dek,
            },
            replacement,
        ))
    }

    /// Writes a verified, encrypted backup using the key this session opened with.
    pub fn backup(
        &self,
        directory: impl AsRef<Path>,
        stamp: &str,
        keep: usize,
    ) -> Result<store::BackupReport, StoreError> {
        store::backup::backup_with_key(&self.database, &self.dek, directory, stamp, keep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::DraftLineDto;

    fn paths(directory: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        (directory.path().join("bonk.db"), directory.path().join("keystore.bin"))
    }

    fn archive() -> (tempfile::TempDir, Archive) {
        let directory = tempfile::TempDir::new().unwrap();
        let (database, keystore) = paths(&directory);
        let archive = Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST).unwrap();
        (directory, archive)
    }

    #[test]
    fn an_archive_round_trips_through_lock_and_unlock() {
        let directory = tempfile::TempDir::new().unwrap();
        let (database, keystore) = paths(&directory);
        drop(Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST).unwrap());

        assert!(Archive::unlock(&database, &keystore, "wrong").is_err());
        let archive = Archive::unlock(&database, &keystore, "passphrase").unwrap();
        assert_eq!(archive.summary().unwrap().schema_version, 17);
        assert_eq!(archive.database_path(), database);
    }

    #[test]
    fn creation_refuses_to_overwrite_an_existing_archive() {
        let directory = tempfile::TempDir::new().unwrap();
        let (database, keystore) = paths(&directory);
        let _archive = Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST).unwrap();
        let again = Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST);
        assert!(again.is_err());
    }

    #[test]
    fn an_archive_writes_a_verified_backup() {
        let (directory, archive) = archive();
        let backups = directory.path().join("backups");
        let report = archive.backup(&backups, "20261107T120000Z", 3).unwrap();
        assert!(report.path.exists());
        assert!(report.bytes > 0);
    }

    #[test]
    fn a_device_revocation_is_recorded_after_the_archive_opens() {
        let (_directory, archive) = archive();
        archive.record_device_revocation().unwrap();
        let forgotten: i64 = archive
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM audit_event WHERE kind = 'device_forgotten'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(forgotten, 1);
    }

    #[test]
    fn a_draft_round_trips_through_the_editor_api() {
        let (_directory, archive) = archive();
        let now = "2026-11-08T12:00:00Z";
        let seller = store::SellerRecord {
            id: crux::SellerId::new(),
            party: crux::Party::new("Seller BV", crux::Address::new("Delft", crux::CountryCode::NL)),
            default_currency: crux::Currency::EUR,
            default_locale: "en".to_owned(),
        };
        store::insert_seller(archive.database().connection(), &seller, now).unwrap();
        let customer = store::CustomerRecord {
            id: crux::CustomerId::new(),
            party: crux::Party::new("Buyer BV", crux::Address::new("Utrecht", crux::CountryCode::NL)),
        };
        store::insert_customer(archive.database().connection(), &customer, now).unwrap();

        let draft = DraftDto {
            id: crux::DocumentId::new().to_string(),
            seller_id: seller.id.to_string(),
            customer_id: customer.id.to_string(),
            doc_type: "380".to_owned(),
            currency: "EUR".to_owned(),
            issue_date: "2026-11-08".to_owned(),
            due_date: None,
            delivery_date: None,
            buyer_reference: None,
            order_reference: None,
            note: Some("Thanks".to_owned()),
            template: None,
            preceding_invoice: None,
            lines: vec![DraftLineDto {
                id: crux::LineId::new().to_string(),
                position: 1,
                name: "Consultancy".to_owned(),
                description: None,
                quantity_micros: 1_000_000,
                unit: "HUR".to_owned(),
                unit_price_micros: 100_000_000,
                price_base_quantity_micros: 1_000_000,
                tax_category: "S".to_owned(),
                tax_rate_bp: 2100,
                line_net_micros: 100_000_000,
                exemption_reason: None,
                exemption_reason_code: None,
            }],
        };

        let saved = archive.save_draft(&draft).unwrap();
        assert_eq!(saved, draft);
        assert_eq!(archive.list_drafts().unwrap(), vec![draft.clone()]);
        assert_eq!(archive.load_draft(&draft.id).unwrap(), draft);
        assert!(archive.validate_draft(&draft).is_ok());

        let totals = archive.draft_totals(&draft).unwrap();
        assert_eq!(totals.net_micros, 100_000_000);
        assert_eq!(totals.tax_micros, 21_000_000);
        assert_eq!(totals.gross_micros, 121_000_000);
    }

    #[test]
    fn customers_round_trip_through_csv() {
        let (_directory, archive) = archive();
        let customer = CustomerDto {
            id: String::new(),
            name: "Buyer BV".to_owned(),
            street: Some("Main 1".to_owned()),
            postal: Some("1000 AA".to_owned()),
            city: "Utrecht".to_owned(),
            country: "NL".to_owned(),
            vat: Some("NL123456788B01".to_owned()),
            kvk: None,
            peppol: None,
            contact: None,
            email: None,
            phone: None,
        };
        archive.save_customer(&customer).unwrap();

        let csv = archive.customers_csv().unwrap();
        assert!(csv.contains("Buyer BV"));

        // The same CSV is skipped rather than duplicated.
        let report = archive.import_customers_csv(&csv).unwrap();
        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped, 1);
        assert!(report.errors.is_empty(), "{report:?}");

        // A new row is imported.
        let csv = format!("{csv}New BV,Main 2,2000 BB,Delft,NL,,,,,\n");
        let report = archive.import_customers_csv(&csv).unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(archive.list_customers().unwrap().len(), 2);
    }

    #[test]
    fn a_blocking_finding_cannot_be_suppressed() {
        let (_directory, archive) = archive();
        let customer = archive
            .save_customer(&CustomerDto {
                id: String::new(),
                name: "Buyer BV".to_owned(),
                street: None,
                postal: None,
                city: "Utrecht".to_owned(),
                country: "NL".to_owned(),
                vat: None,
                kvk: None,
                peppol: None,
                contact: None,
                email: None,
                phone: None,
            })
            .unwrap();
        let draft = archive.new_draft(&customer.id).unwrap();
        let findings = archive.validate_draft(&draft).unwrap();
        let blocking = findings
            .iter()
            .find(|finding| finding.severity == "blocking")
            .expect("an empty draft blocks");
        assert!(
            archive
                .suppress_finding(&draft.id, &blocking.code, &blocking.field, "because")
                .is_err(),
            "only non-blocking findings can be suppressed"
        );
    }

    #[test]
    fn the_editor_validation_matches_the_domain_rules() {
        let (_directory, archive) = archive();
        let customer = archive
            .save_customer(&CustomerDto {
                id: String::new(),
                name: "Buyer BV".to_owned(),
                street: None,
                postal: None,
                city: "Utrecht".to_owned(),
                country: "NL".to_owned(),
                vat: None,
                kvk: None,
                peppol: None,
                contact: None,
                email: None,
                phone: None,
            })
            .unwrap();
        let draft = archive.new_draft(&customer.id).unwrap();
        let via_editor = archive.validate_draft(&draft).unwrap();

        let domain = draft.clone().into_draft().unwrap();
        let seller = store::load_seller(archive.database().connection(), domain.seller_id).unwrap();
        let buyer = store::load_customer(archive.database().connection(), domain.customer_id).unwrap();
        let direct: Vec<FindingDto> = crux::validate_draft(&domain, &seller.party, &buyer.party)
            .iter()
            .map(FindingDto::from)
            .collect();
        assert_eq!(via_editor, direct, "the editor and the desktop must share the rules");
    }
}
