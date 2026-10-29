//! Phone numbers, normalised to E.164.
//!
//! A number is stored the way it can be dialled from anywhere: `+`, a country
//! calling code, then the national number with no trunk prefix and no
//! separators. Typing `(06) 12 34 56 78` for a Dutch party and `+31 6 12345678`
//! for the same party must produce one stored value, because a customer file is
//! compared by what it says, and two spellings of one number are two customers.

use core::fmt;

use crate::error::DomainError;
use crate::party::CountryCode;

/// A phone number in E.164 form: a `+`, then up to fifteen digits.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PhoneNumber(String);

impl PhoneNumber {
    /// Parses and normalises a typed number.
    ///
    /// Accepts spaces, dashes, dots, slashes, and parentheses as separators, a
    /// leading `+` or `00` for international form, and a leading `0` for a
    /// national number, which is completed with `country`'s calling code.
    pub fn parse(value: &str, country: CountryCode) -> Result<Self, DomainError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidPhone("the number is empty".to_owned()));
        }

        let mut digits = String::new();
        let mut international = false;
        for character in trimmed.chars() {
            match character {
                '+' => {
                    if !digits.is_empty() || international {
                        return Err(DomainError::InvalidPhone(format!(
                            "a plus sign only belongs at the start of {trimmed:?}"
                        )));
                    }
                    international = true;
                }
                '0'..='9' => digits.push(character),
                ' ' | '-' | '.' | '/' | '(' | ')' | '\u{a0}' | '\u{2011}' | '\u{2013}' => {}
                other => {
                    return Err(DomainError::InvalidPhone(format!(
                        "{other:?} is not part of a phone number"
                    )));
                }
            }
        }

        if !international && digits.starts_with("00") {
            international = true;
            digits.drain(..2);
        }

        let normalised = if international {
            digits
        } else if country.as_str() == "IT" {
            // Italy keeps its leading zero, which is the one documented
            // exception among the countries this application files under.
            format!("{}{digits}", calling_code(country)?)
        } else if let Some(rest) = digits.strip_prefix('0') {
            format!("{}{rest}", calling_code(country)?)
        } else {
            // Nothing marks this as national or international; treating it as a
            // national significant number is what a spreadsheet export means.
            format!("{}{digits}", calling_code(country)?)
        };

        let e164 = format!("+{normalised}");
        let count = e164.len() - 1;
        if !(6..=15).contains(&count) {
            return Err(DomainError::InvalidPhone(format!(
                "{value:?} has {count} digits; a phone number has 6 to 15"
            )));
        }
        if e164.as_bytes()[1] == b'0' {
            return Err(DomainError::InvalidPhone(format!(
                "{value:?} has no country calling code"
            )));
        }
        Ok(Self(e164))
    }

    /// The number as stored, `+` included.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PhoneNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The country calling code for the countries this application files under.
///
/// A number typed in international form needs no table, so an unlisted country
/// is only a problem for a national number; saying so is better than guessing.
fn calling_code(country: CountryCode) -> Result<&'static str, DomainError> {
    let code = match country.as_str() {
        "AT" => "43",
        "BE" => "32",
        "BG" => "359",
        "CH" => "41",
        "CY" => "357",
        "CZ" => "420",
        "DE" => "49",
        "DK" => "45",
        "EE" => "372",
        "ES" => "34",
        "FI" => "358",
        "FR" => "33",
        "GB" => "44",
        "GR" => "30",
        "HR" => "385",
        "HU" => "36",
        "IE" => "353",
        "IS" => "354",
        "IT" => "39",
        "LI" => "423",
        "LT" => "370",
        "LU" => "352",
        "LV" => "371",
        "MT" => "356",
        "NL" => "31",
        "NO" => "47",
        "PL" => "48",
        "PT" => "351",
        "RO" => "40",
        "SE" => "46",
        "SI" => "386",
        "SK" => "421",
        other => {
            return Err(DomainError::InvalidPhone(format!(
                "no known calling code for {other}; type the number in + form"
            )));
        }
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nl(value: &str) -> String {
        PhoneNumber::parse(value, CountryCode::NL).unwrap().as_str().to_owned()
    }

    #[test]
    fn a_dutch_mobile_loses_its_trunk_prefix() {
        assert_eq!(nl("06 12 34 56 78"), "+31612345678");
        assert_eq!(nl("0612345678"), "+31612345678");
        assert_eq!(nl("(06) 1234-5678"), "+31612345678");
        assert_eq!(nl("+31 6 1234 5678"), "+31612345678");
        assert_eq!(nl("0031 6 1234 5678"), "+31612345678");
    }

    #[test]
    fn a_number_without_a_marker_is_read_as_national() {
        assert_eq!(nl("612345678"), "+31612345678");
    }

    #[test]
    fn another_country_uses_its_own_code() {
        let german = CountryCode::parse("DE").unwrap();
        assert_eq!(
            PhoneNumber::parse("030 1234567", german).unwrap().as_str(),
            "+49301234567"
        );
    }

    #[test]
    fn italy_keeps_its_leading_zero() {
        let italian = CountryCode::parse("IT").unwrap();
        assert_eq!(
            PhoneNumber::parse("06 1234 5678", italian).unwrap().as_str(),
            "+390612345678"
        );
    }

    #[test]
    fn rubbish_is_refused_with_a_reason() {
        assert!(PhoneNumber::parse("", CountryCode::NL).is_err());
        assert!(PhoneNumber::parse("call me", CountryCode::NL).is_err());
        assert!(PhoneNumber::parse("06 12 34+56", CountryCode::NL).is_err());
        assert!(PhoneNumber::parse("123", CountryCode::NL).is_err());
        assert!(PhoneNumber::parse("+1234567890123456", CountryCode::NL).is_err());
    }

    #[test]
    fn an_unlisted_country_needs_international_form() {
        let country = CountryCode::parse("US").unwrap();
        assert!(PhoneNumber::parse("020 7946 0958", country).is_err());
        assert_eq!(
            PhoneNumber::parse("+1 202 555 0147", country).unwrap().as_str(),
            "+12025550147"
        );
    }
}
