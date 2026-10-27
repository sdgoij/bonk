//! Typed identifiers.
//!
//! Every aggregate gets its own newtype so a customer id can never be passed
//! where an invoice id is expected. Identifiers are UUIDv7: globally unique,
//! time-ordered for index locality, and safe to generate offline.

use crate::error::DomainError;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(uuid::Uuid);

        impl $name {
            /// Generates a new time-ordered identifier.
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7())
            }

            /// Wraps an existing UUID.
            pub const fn from_uuid(value: uuid::Uuid) -> Self {
                Self(value)
            }

            /// The underlying UUID.
            pub const fn as_uuid(&self) -> &uuid::Uuid {
                &self.0
            }

            /// Parses the canonical hyphenated form.
            pub fn parse_str(value: &str) -> Result<Self, DomainError> {
                uuid::Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|_| DomainError::InvalidId(value.to_owned()))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = DomainError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse_str(s)
            }
        }
    };
}

uuid_id!(
    /// An issued invoice or other legal document.
    DocumentId
);
uuid_id!(
    /// A seller (legal entity).
    SellerId
);
uuid_id!(
    /// A customer or supplier.
    CustomerId
);
uuid_id!(
    /// A line inside a draft or issued invoice.
    LineId
);
uuid_id!(
    /// A device participating in a sync group.
    DeviceId
);
uuid_id!(
    /// A journal entry (v1.1 accounting layer).
    JournalEntryId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_unique_and_parseable() {
        let first = DocumentId::new();
        let second = DocumentId::new();
        assert_ne!(first, second);
        assert_eq!(DocumentId::parse_str(&first.to_string()).unwrap(), first);
    }

    #[test]
    fn generations_are_time_ordered() {
        let mut previous = DocumentId::new();
        for _ in 0..32 {
            let next = DocumentId::new();
            assert!(next.as_uuid() > previous.as_uuid());
            previous = next;
        }
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert!(DocumentId::parse_str("not-a-uuid").is_err());
        assert!(SellerId::parse_str("").is_err());
    }

    #[test]
    fn distinct_types_do_not_interchange() {
        let seller = SellerId::new();
        let customer = CustomerId::from_uuid(*seller.as_uuid());
        // Same underlying UUID, different types; this compiles because the
        // conversion is explicit and the types stay distinct.
        assert_eq!(seller.as_uuid(), customer.as_uuid());
    }
}
