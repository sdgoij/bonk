//! JSON shapes used at the storage boundary.
//!
//! The domain has no `serde` derives on purpose; converting here keeps the
//! public model free of serialisation concerns and makes schema changes an
//! explicit, reviewable step.

use crux::{Address, CountryCode, Party};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::StoreError;

/// Address as stored in `address_json`.
#[derive(Serialize, Deserialize)]
pub(crate) struct AddressJson {
    #[serde(default)]
    pub street: Option<String>,
    pub city: String,
    #[serde(default)]
    pub postal_code: Option<String>,
    pub country: String,
}

/// Contact block as stored in `contact_json`.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct ContactJson {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
}

pub(crate) fn address_to_json(address: &Address) -> Result<String, StoreError> {
    let json = AddressJson {
        street: address.street.clone(),
        city: address.city.clone(),
        postal_code: address.postal_code.clone(),
        country: address.country.to_string(),
    };
    Ok(serde_json::to_string(&json)?)
}

pub(crate) fn address_from_json(raw: &str) -> Result<Address, StoreError> {
    let json: AddressJson = serde_json::from_str(raw)?;
    Ok(Address {
        street: json.street,
        city: json.city,
        postal_code: json.postal_code,
        country: CountryCode::parse(&json.country).map_err(|error| StoreError::Data(error.to_string()))?,
    })
}

pub(crate) fn contact_to_json(party: &Party) -> Result<Option<String>, StoreError> {
    if party.contact_name.is_none() && party.email.is_none() && party.phone.is_none() {
        return Ok(None);
    }
    let json = ContactJson {
        name: party.contact_name.clone(),
        email: party.email.clone(),
        phone: party.phone.clone(),
    };
    Ok(Some(serde_json::to_string(&json)?))
}

pub(crate) fn contact_from_json(raw: Option<&str>) -> Result<ContactJson, StoreError> {
    match raw {
        Some(text) => Ok(serde_json::from_str(text)?),
        None => Ok(ContactJson::default()),
    }
}

/// A party as a canonical JSON value, used for snapshots and the semantic form.
pub(crate) fn party_to_value(party: &Party) -> Result<serde_json::Value, StoreError> {
    Ok(serde_json::json!({
        "legal_name": party.legal_name,
        "trading_name": party.trading_name,
        "vat_id": party.vat_id,
        "kvk_number": party.kvk_number,
        "peppol_id": party.peppol_id,
        "address": {
            "street": party.address.street,
            "city": party.address.city,
            "postal_code": party.address.postal_code,
            "country": party.address.country.to_string(),
        },
        "contact_name": party.contact_name,
        "email": party.email,
        "phone": party.phone,
    }))
}

/// The inverse of [`party_to_value`], used when a screen reads a snapshot back.
pub(crate) fn party_from_value(value: &Value) -> Result<Party, StoreError> {
    let text = |container: &Value, key: &str| container.get(key).and_then(Value::as_str).map(str::to_owned);
    let legal_name =
        text(value, "legal_name").ok_or_else(|| StoreError::Data("a party snapshot has no legal name".to_owned()))?;
    let address = value
        .get("address")
        .ok_or_else(|| StoreError::Data("a party snapshot has no address".to_owned()))?;
    let country = text(address, "country").unwrap_or_default();
    Ok(Party {
        legal_name,
        trading_name: text(value, "trading_name"),
        vat_id: text(value, "vat_id"),
        kvk_number: text(value, "kvk_number"),
        peppol_id: text(value, "peppol_id"),
        address: Address {
            street: text(address, "street"),
            city: text(address, "city").unwrap_or_default(),
            postal_code: text(address, "postal_code"),
            country: CountryCode::parse(&country).map_err(|error| StoreError::Data(error.to_string()))?,
        },
        contact_name: text(value, "contact_name"),
        email: text(value, "email"),
        phone: text(value, "phone"),
    })
}
