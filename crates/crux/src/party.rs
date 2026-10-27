//! Parties, addresses, and identifier checks.

use core::fmt;

use crate::error::DomainError;
use crate::findings::Finding;

/// ISO 3166-1 alpha-2 country code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    /// The Netherlands.
    pub const NL: Self = Self(*b"NL");

    /// Parses exactly two uppercase ASCII letters.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let bytes = value.as_bytes();
        if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_uppercase) {
            return Err(DomainError::InvalidCountry(value.to_owned()));
        }
        Ok(Self([bytes[0], bytes[1]]))
    }

    /// The code as a string slice.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("??")
    }
}

impl fmt::Display for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CountryCode({})", self.as_str())
    }
}

impl std::str::FromStr for CountryCode {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A postal address (BG-5 / BG-8).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Address {
    /// Street and number.
    pub street: Option<String>,
    /// City; required by every profile we target.
    pub city: String,
    /// Postal code.
    pub postal_code: Option<String>,
    /// Country; required.
    pub country: CountryCode,
}

impl Address {
    /// Builds an address with the required fields.
    pub fn new(city: impl Into<String>, country: CountryCode) -> Self {
        Self {
            street: None,
            city: city.into(),
            postal_code: None,
            country,
        }
    }
}

/// A seller or customer party.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Party {
    /// Registered name (BT-27 / BT-44).
    pub legal_name: String,
    /// Trading name, when it differs.
    pub trading_name: Option<String>,
    /// VAT identifier (BT-31 / BT-48).
    pub vat_id: Option<String>,
    /// Dutch chamber of commerce number.
    pub kvk_number: Option<String>,
    /// Peppol participant identifier, `EAS:value`.
    pub peppol_id: Option<String>,
    /// Postal address.
    pub address: Address,
    /// Contact name.
    pub contact_name: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
}

impl Party {
    /// A party with only the required fields filled.
    pub fn new(legal_name: impl Into<String>, address: Address) -> Self {
        Self {
            legal_name: legal_name.into(),
            trading_name: None,
            vat_id: None,
            kvk_number: None,
            peppol_id: None,
            address,
            contact_name: None,
            email: None,
            phone: None,
        }
    }

    /// The Peppol endpoint identifier split into its EAS scheme and value.
    ///
    /// BT-34/BT-49 are written as `schemeID` plus a value. An identifier that
    /// does not have both parts cannot be serialised, so callers must treat a
    /// `None` here as "no usable endpoint", not as "no endpoint configured".
    pub fn peppol_endpoint(&self) -> Option<(&str, &str)> {
        let (scheme, value) = self.peppol_id.as_deref()?.split_once(':')?;
        if scheme.is_empty() || value.is_empty() {
            return None;
        }
        Some((scheme, value))
    }

    /// Validates the party, attributing findings to `role` (for example
    /// `seller` or `customer`).
    pub fn validate(&self, role: &str) -> Vec<Finding> {
        let mut findings = Vec::new();

        if self.legal_name.trim().is_empty() {
            findings.push(Finding::blocking(
                "T1-008",
                Some(if role == "seller" { "BT-27" } else { "BT-44" }),
                format!("{role}.legal_name"),
                "legal name is required",
            ));
        }
        if self.address.city.trim().is_empty() {
            findings.push(Finding::blocking(
                "T1-009",
                Some(if role == "seller" { "BT-37" } else { "BT-52" }),
                format!("{role}.city"),
                "city is required",
            ));
        }
        if let Some(email) = &self.email
            && !email.contains('@')
        {
            findings.push(Finding::warning(
                "T1-016",
                None,
                format!("{role}.email"),
                "email address does not contain '@'",
            ));
        }
        if let Some(vat_id) = &self.vat_id {
            let bt = if role == "seller" { "BT-31" } else { "BT-48" };
            match validate_vat_id(vat_id) {
                VatIdCheck::Valid => {}
                VatIdCheck::Malformed(reason) => findings.push(Finding::blocking(
                    "T1-010",
                    Some(bt),
                    format!("{role}.vat_id"),
                    format!("VAT identifier is malformed: {reason}"),
                )),
                VatIdCheck::ChecksumFailed => findings.push(Finding::warning(
                    "T1-011",
                    Some(bt),
                    format!("{role}.vat_id"),
                    "VAT identifier failed its checksum; verify against VIES",
                )),
            }
        }
        if let Some(kvk) = &self.kvk_number
            && (kvk.len() != 8 || !kvk.bytes().all(|byte| byte.is_ascii_digit()))
        {
            findings.push(Finding::warning(
                "T1-012",
                None,
                format!("{role}.kvk_number"),
                "Dutch chamber of commerce number should be 8 digits",
            ));
        }
        if let Some(peppol) = &self.peppol_id
            && !peppol.contains(':')
        {
            findings.push(Finding::warning(
                "T1-017",
                None,
                format!("{role}.peppol_id"),
                "Peppol identifier should be in EAS:value form",
            ));
        }

        findings
    }
}

/// Outcome of a VAT identifier check.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VatIdCheck {
    /// Structurally valid (and checksum-clean where we implement one).
    Valid,
    /// Wrong shape for the country.
    Malformed(&'static str),
    /// Right shape, failed the national checksum.
    ChecksumFailed,
}

/// Validates a VAT identifier without network access.
///
/// Dutch identifiers get the 11-proof. Other countries are checked for shape
/// only; VIES remains the authority and is queried separately.
pub fn validate_vat_id(value: &str) -> VatIdCheck {
    let value = value.trim().to_ascii_uppercase();
    let bytes = value.as_bytes();
    if bytes.len() < 4 {
        return VatIdCheck::Malformed("too short for a country prefix and number");
    }
    if !bytes[0].is_ascii_alphabetic() || !bytes[1].is_ascii_alphabetic() {
        return VatIdCheck::Malformed("must start with a two-letter country prefix");
    }

    if value.starts_with("NL") {
        // NL + 9 digits + B + 2 digits
        if bytes.len() != 14 {
            return VatIdCheck::Malformed("Dutch numbers are NL + 9 digits + B + 2 digits");
        }
        if bytes[11] != b'B' {
            return VatIdCheck::Malformed("Dutch numbers contain a literal B before the last two digits");
        }
        let digits = &value[2..11];
        if !digits.bytes().all(|byte| byte.is_ascii_digit()) || !value[12..].bytes().all(|byte| byte.is_ascii_digit()) {
            return VatIdCheck::Malformed("Dutch numbers contain non-digits");
        }
        return if eleven_proof(digits) {
            VatIdCheck::Valid
        } else {
            VatIdCheck::ChecksumFailed
        };
    }

    if bytes[2..].len() < 2 || bytes[2..].len() > 12 || !bytes[2..].iter().all(|byte| byte.is_ascii_alphanumeric()) {
        return VatIdCheck::Malformed("expected 2-12 alphanumeric characters after the prefix");
    }
    VatIdCheck::Valid
}

/// The Dutch 11-proof over the nine-digit part of a VAT identifier.
fn eleven_proof(digits: &str) -> bool {
    let mut sum = 0_u32;
    for (index, character) in digits.bytes().take(8).enumerate() {
        let Some(digit) = (character as char).to_digit(10) else {
            return false;
        };
        let weight = 9 - index as u32;
        sum += digit * weight;
    }

    let Some(check_digit) = digits.as_bytes().get(8).and_then(|byte| char::from(*byte).to_digit(10)) else {
        return false;
    };
    sum % 11 == check_digit
}

/// Findings for a party, with the role already applied.
pub fn validate_party(party: &Party, role: &str) -> Vec<Finding> {
    party.validate(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dutch_vat_numbers_use_the_eleven_proof() {
        assert_eq!(validate_vat_id("NL123456782B01"), VatIdCheck::Valid);
        assert_eq!(validate_vat_id("nl123456782b01"), VatIdCheck::Valid);
        assert_eq!(validate_vat_id("NL123456788B01"), VatIdCheck::ChecksumFailed);
    }

    #[test]
    fn peppol_endpoints_need_a_scheme_and_a_value() {
        let mut party = Party::new("Party BV", Address::new("Amsterdam", CountryCode::NL));
        assert_eq!(party.peppol_endpoint(), None);

        party.peppol_id = Some("0106:12345678".to_owned());
        assert_eq!(party.peppol_endpoint(), Some(("0106", "12345678")));

        party.peppol_id = Some("12345678".to_owned());
        assert_eq!(party.peppol_endpoint(), None, "a scheme is required");

        party.peppol_id = Some("0106:".to_owned());
        assert_eq!(party.peppol_endpoint(), None, "a value is required");

        party.peppol_id = Some(":12345678".to_owned());
        assert_eq!(party.peppol_endpoint(), None, "an empty scheme is not a scheme");
    }

    #[test]
    fn malformed_vat_numbers_are_rejected() {
        assert!(matches!(validate_vat_id("NL123"), VatIdCheck::Malformed(_)));
        assert!(matches!(validate_vat_id("NL123456782C01"), VatIdCheck::Malformed(_)));
        assert!(matches!(validate_vat_id("1L123456782B01"), VatIdCheck::Malformed(_)));
    }
}
