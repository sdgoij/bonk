//! Invoice state machine.
//!
//! The rule that matters legally: a document becomes immutable at `Issued`, and
//! the only way to correct it is a credit note. The state machine makes that
//! unavoidable rather than a convention.

use core::fmt;

use crate::calc::line_net;
use crate::date::LocalDate;
use crate::error::DomainError;
use crate::ids::{CustomerId, DocumentId, LineId, SellerId};
use crate::money::{Currency, Money};
use crate::quantity::{Quantity, UnitCode};
use crate::vat::TaxTreatment;

/// Document type codes from UN/CEFACT 1001.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum DocType {
    /// Commercial invoice (`380`).
    Invoice,
    /// Credit note (`381`).
    CreditNote,
}

impl DocType {
    /// The UN/CEFACT code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Invoice => "380",
            Self::CreditNote => "381",
        }
    }

    /// True for a credit note.
    pub const fn is_credit_note(self) -> bool {
        matches!(self, Self::CreditNote)
    }
}

/// The invoice a credit note corrects (BG-3).
///
/// The number and date are snapshotted rather than looked up when printing: a
/// correction must keep naming exactly the document it was issued against.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PrecedingInvoice {
    /// Identity of the issued invoice in this database.
    pub id: DocumentId,
    /// Its number (BT-25).
    pub number: String,
    /// Its issue date (BT-26).
    pub issue_date: LocalDate,
}

impl PrecedingInvoice {
    /// Builds a reference to an issued invoice.
    pub fn new(id: DocumentId, number: impl Into<String>, issue_date: LocalDate) -> Self {
        Self {
            id,
            number: number.into(),
            issue_date,
        }
    }
}

/// Where a document is in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum InvoiceState {
    /// Being edited; mutable.
    Draft,
    /// Passes blocking validation; still mutable.
    Validated,
    /// Numbered, hashed, and written once.
    Issued,
    /// Handed to a connector and acknowledged.
    Sent,
    /// Settlement recorded (append-only payment events).
    Paid,
    /// Superseded by a credit note.
    Credited,
    /// Abandoned before issue.
    Discarded,
}

impl InvoiceState {
    /// True when the document must never change again.
    pub const fn is_immutable(self) -> bool {
        matches!(self, Self::Issued | Self::Sent | Self::Paid | Self::Credited)
    }

    /// True when no further event is accepted.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Credited | Self::Discarded)
    }

    /// True when the document has a legal number.
    pub const fn is_numbered(self) -> bool {
        matches!(self, Self::Issued | Self::Sent | Self::Paid | Self::Credited)
    }
}

/// Something that can happen to a document.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum InvoiceEvent {
    /// Run blocking validation.
    Validate,
    /// Return to editing.
    Edit,
    /// Allocate a number and freeze the document.
    Issue,
    /// Throw the draft away.
    Discard,
    /// A connector acknowledged delivery.
    MarkSent,
    /// A payment was recorded.
    RecordPayment,
    /// A credit note was issued against this document.
    Credit,
}

/// An illegal transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TransitionError {
    /// State the document was in.
    pub from: InvoiceState,
    /// Event that was rejected.
    pub event: InvoiceEvent,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot apply {:?} to a document in {:?}", self.event, self.from)
    }
}

impl std::error::Error for TransitionError {}

/// Applies an event, returning the next state or the rejection.
pub fn transition(state: InvoiceState, event: InvoiceEvent) -> Result<InvoiceState, TransitionError> {
    use InvoiceEvent as E;
    use InvoiceState as S;

    let next = match (state, event) {
        (S::Draft, E::Validate) => S::Validated,
        (S::Validated, E::Edit) => S::Draft,
        (S::Validated, E::Issue) => S::Issued,
        (S::Draft | S::Validated, E::Discard) => S::Discarded,
        (S::Issued | S::Sent, E::MarkSent) => S::Sent,
        (S::Issued | S::Sent, E::RecordPayment) => S::Paid,
        (S::Issued | S::Sent | S::Paid, E::Credit) => S::Credited,
        _ => return Err(TransitionError { from: state, event }),
    };
    Ok(next)
}

/// One line of a draft invoice.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DraftLine {
    /// Stable line identity, used by the UI and by sync.
    pub id: LineId,
    /// 1-based position in the document; unique within the draft.
    pub position: u32,
    /// Item name (BT-153).
    pub name: String,
    /// Optional longer description (BT-154).
    pub description: Option<String>,
    /// Quantity (BT-129).
    pub quantity: Quantity,
    /// Unit of measure (BT-130).
    pub unit: UnitCode,
    /// Unit price (BT-146).
    pub unit_price: Money,
    /// Price base quantity (BT-149), normally 1.
    pub price_base_quantity: Quantity,
    /// VAT treatment (BT-151 and friends).
    pub treatment: TaxTreatment,
}

impl DraftLine {
    /// Line net amount, rounded to the cent.
    pub fn net(&self) -> Result<Money, DomainError> {
        line_net(self.quantity, self.unit_price, self.price_base_quantity)
    }
}

/// A mutable invoice draft.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InvoiceDraft {
    /// Draft identity; replaced by a fresh id for the issued document.
    pub id: DocumentId,
    /// Seller.
    pub seller_id: SellerId,
    /// Customer.
    pub customer_id: CustomerId,
    /// Document type: invoice or credit note.
    pub doc_type: DocType,
    /// Document currency.
    pub currency: Currency,
    /// Business date (BT-2).
    pub issue_date: LocalDate,
    /// Due date (BT-9).
    pub due_date: Option<LocalDate>,
    /// Actual delivery date (BT-72).
    pub delivery_date: Option<LocalDate>,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<String>,
    /// Purchase order reference (BT-13).
    pub order_reference: Option<String>,
    /// Free-text note (BT-22).
    pub note: Option<String>,
    /// The invoice this credit note corrects (BG-3).
    pub preceding_invoice: Option<PrecedingInvoice>,
    /// Lines (BG-25).
    pub lines: Vec<DraftLine>,
}

impl InvoiceDraft {
    /// A new invoice draft with no lines.
    pub fn new(seller_id: SellerId, customer_id: CustomerId, currency: Currency, issue_date: LocalDate) -> Self {
        Self {
            id: DocumentId::new(),
            seller_id,
            customer_id,
            doc_type: DocType::Invoice,
            currency,
            issue_date,
            due_date: None,
            delivery_date: None,
            buyer_reference: None,
            order_reference: None,
            note: None,
            preceding_invoice: None,
            lines: Vec::new(),
        }
    }

    /// A credit note against `preceding`, with no lines.
    pub fn credit_note(
        seller_id: SellerId,
        customer_id: CustomerId,
        currency: Currency,
        issue_date: LocalDate,
        preceding: PrecedingInvoice,
    ) -> Self {
        let mut draft = Self::new(seller_id, customer_id, currency, issue_date);
        draft.doc_type = DocType::CreditNote;
        draft.preceding_invoice = Some(preceding);
        draft
    }

    /// Sum of line nets; `None` for an empty draft.
    pub fn net(&self) -> Result<Option<Money>, DomainError> {
        let Some(first) = self.lines.first() else {
            return Ok(None);
        };
        let mut total = Money::zero(first.unit_price.currency());
        for line in &self.lines {
            total = total.checked_add(line.net()?)?;
        }
        Ok(Some(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InvoiceEvent as E;
    use InvoiceState as S;

    #[test]
    fn happy_path() {
        assert_eq!(transition(S::Draft, E::Validate).unwrap(), S::Validated);
        assert_eq!(transition(S::Validated, E::Issue).unwrap(), S::Issued);
        assert_eq!(transition(S::Issued, E::MarkSent).unwrap(), S::Sent);
        assert_eq!(transition(S::Sent, E::RecordPayment).unwrap(), S::Paid);
        assert_eq!(transition(S::Paid, E::Credit).unwrap(), S::Credited);
    }

    #[test]
    fn editing_after_validation_returns_to_draft() {
        assert_eq!(transition(S::Validated, E::Edit).unwrap(), S::Draft);
    }

    #[test]
    fn issuing_requires_validation() {
        assert!(transition(S::Draft, E::Issue).is_err());
    }

    #[test]
    fn issued_documents_cannot_be_edited_or_discarded() {
        for state in [S::Issued, S::Sent, S::Paid] {
            assert!(transition(state, E::Edit).is_err());
            assert!(transition(state, E::Discard).is_err());
            assert!(transition(state, E::Validate).is_err());
            assert!(transition(state, E::Issue).is_err());
        }
        assert!(transition(S::Credited, E::Credit).is_err());
    }

    #[test]
    fn payment_before_delivery_is_allowed() {
        assert_eq!(transition(S::Issued, E::RecordPayment).unwrap(), S::Paid);
    }

    #[test]
    fn credit_note_can_follow_every_live_state() {
        for state in [S::Issued, S::Sent, S::Paid] {
            assert_eq!(transition(state, E::Credit).unwrap(), S::Credited);
        }
    }

    #[test]
    fn terminal_states_accept_nothing() {
        for state in [S::Credited, S::Discarded] {
            assert!(state.is_terminal());
            for event in [
                E::Validate,
                E::Edit,
                E::Issue,
                E::Discard,
                E::MarkSent,
                E::RecordPayment,
                E::Credit,
            ] {
                assert!(transition(state, event).is_err());
            }
        }
    }

    #[test]
    fn immutability_flags_match_the_legal_model() {
        assert!(!S::Draft.is_immutable());
        assert!(!S::Validated.is_immutable());
        for state in [S::Issued, S::Sent, S::Paid, S::Credited] {
            assert!(state.is_immutable());
            assert!(state.is_numbered());
        }
        assert!(!S::Discarded.is_numbered());
    }

    #[test]
    fn doc_type_codes() {
        assert_eq!(DocType::Invoice.code(), "380");
        assert_eq!(DocType::CreditNote.code(), "381");
    }

    #[test]
    fn error_renders_usefully() {
        let error = transition(S::Issued, E::Edit).unwrap_err();
        assert_eq!(error.to_string(), "cannot apply Edit to a document in Issued");
    }
}
