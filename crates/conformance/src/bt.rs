//! EN 16931 business terms and groups.
//!
//! Only terms used by the current rule set are listed. Finding messages carry
//! the BT code so the UI can focus the field that violated the rule.

use core::fmt;

/// A business group from EN 16931.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Bg {
    /// Seller party (BG-4).
    Seller,
    /// Buyer party (BG-7).
    Buyer,
    /// Invoice line (BG-25).
    InvoiceLine,
    /// Document totals (BG-22).
    DocumentTotals,
    /// VAT breakdown (BG-23).
    VatBreakdown,
    /// Document-level allowance (BG-20).
    DocumentAllowance,
    /// Document-level charge (BG-21).
    DocumentCharge,
}

impl Bg {
    /// The EN 16931 code, e.g. `BG-25`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Seller => "BG-4",
            Self::Buyer => "BG-7",
            Self::InvoiceLine => "BG-25",
            Self::DocumentTotals => "BG-22",
            Self::VatBreakdown => "BG-23",
            Self::DocumentAllowance => "BG-20",
            Self::DocumentCharge => "BG-21",
        }
    }

    /// Human-readable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Seller => "Seller",
            Self::Buyer => "Buyer",
            Self::InvoiceLine => "Invoice line",
            Self::DocumentTotals => "Document totals",
            Self::VatBreakdown => "VAT breakdown",
            Self::DocumentAllowance => "Document level allowance",
            Self::DocumentCharge => "Document level charge",
        }
    }
}

impl fmt::Display for Bg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// A business term from EN 16931.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Bt {
    /// Invoice number (BT-1).
    InvoiceNumber,
    /// Invoice issue date (BT-2).
    IssueDate,
    /// Invoice type code (BT-3).
    InvoiceTypeCode,
    /// Invoice currency code (BT-5).
    Currency,
    /// Payment due date (BT-9).
    DueDate,
    /// Buyer reference (BT-10).
    BuyerReference,
    /// Purchase order reference (BT-13).
    OrderReference,
    /// Invoice note (BT-22).
    Note,
    /// Specification identifier (BT-24).
    SpecificationIdentifier,
    /// Seller name (BT-27).
    SellerName,
    /// Seller VAT identifier (BT-31).
    SellerVatId,
    /// Seller tax registration identifier (BT-32).
    SellerTaxRegistrationId,
    /// Seller endpoint identifier (BT-34).
    SellerEndpointId,
    /// Seller address country code (BT-40).
    SellerCountry,
    /// Seller tax representative VAT identifier (BT-63).
    SellerTaxRepresentativeVatId,
    /// Buyer name (BT-44).
    BuyerName,
    /// Buyer legal registration identifier (BT-47).
    BuyerLegalRegistrationId,
    /// Buyer VAT identifier (BT-48).
    BuyerVatId,
    /// Buyer endpoint identifier (BT-49).
    BuyerEndpointId,
    /// Buyer address country code (BT-55).
    BuyerCountry,
    /// Actual delivery date (BT-72).
    ActualDeliveryDate,
    /// Deliver-to country code (BT-80).
    DeliverToCountry,
    /// Sum of invoice line net amounts (BT-106).
    SumLineNet,
    /// Sum of document-level allowances (BT-107).
    SumAllowances,
    /// Sum of document-level charges (BT-108).
    SumCharges,
    /// Invoice total without VAT (BT-109).
    TaxExclusiveAmount,
    /// Invoice total VAT amount (BT-110).
    TaxTotal,
    /// Invoice total with VAT (BT-112).
    TaxInclusiveAmount,
    /// Paid amount (BT-113).
    PaidAmount,
    /// Rounding amount (BT-114).
    RoundingAmount,
    /// Amount due for payment (BT-115).
    AmountDue,
    /// VAT category taxable amount (BT-116).
    VatTaxableAmount,
    /// VAT category tax amount (BT-117).
    VatTaxAmount,
    /// VAT category code (BT-118).
    VatCategoryCode,
    /// VAT category rate (BT-119).
    VatCategoryRate,
    /// VAT exemption reason text (BT-120).
    VatExemptionText,
    /// VAT exemption reason code (BT-121).
    VatExemptionCode,
    /// Invoiced quantity (BT-129).
    InvoicedQuantity,
    /// Invoiced quantity unit of measure code (BT-130).
    UnitCode,
    /// Invoice line net amount (BT-131).
    LineNetAmount,
    /// Item net price (BT-146).
    ItemNetPrice,
    /// Item price base quantity (BT-149).
    PriceBaseQuantity,
    /// Invoiced item VAT category code (BT-151).
    ItemVatCategory,
    /// Invoiced item VAT rate (BT-152).
    ItemVatRate,
    /// Item name (BT-153).
    ItemName,
    /// Item description (BT-154).
    ItemDescription,
}

impl Bt {
    /// The EN 16931 code, e.g. `BT-131`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvoiceNumber => "BT-1",
            Self::IssueDate => "BT-2",
            Self::InvoiceTypeCode => "BT-3",
            Self::Currency => "BT-5",
            Self::DueDate => "BT-9",
            Self::BuyerReference => "BT-10",
            Self::OrderReference => "BT-13",
            Self::Note => "BT-22",
            Self::SpecificationIdentifier => "BT-24",
            Self::SellerName => "BT-27",
            Self::SellerVatId => "BT-31",
            Self::SellerTaxRegistrationId => "BT-32",
            Self::SellerEndpointId => "BT-34",
            Self::SellerCountry => "BT-40",
            Self::SellerTaxRepresentativeVatId => "BT-63",
            Self::BuyerName => "BT-44",
            Self::BuyerLegalRegistrationId => "BT-47",
            Self::BuyerVatId => "BT-48",
            Self::BuyerEndpointId => "BT-49",
            Self::BuyerCountry => "BT-55",
            Self::ActualDeliveryDate => "BT-72",
            Self::DeliverToCountry => "BT-80",
            Self::SumLineNet => "BT-106",
            Self::SumAllowances => "BT-107",
            Self::SumCharges => "BT-108",
            Self::TaxExclusiveAmount => "BT-109",
            Self::TaxTotal => "BT-110",
            Self::TaxInclusiveAmount => "BT-112",
            Self::PaidAmount => "BT-113",
            Self::RoundingAmount => "BT-114",
            Self::AmountDue => "BT-115",
            Self::VatTaxableAmount => "BT-116",
            Self::VatTaxAmount => "BT-117",
            Self::VatCategoryCode => "BT-118",
            Self::VatCategoryRate => "BT-119",
            Self::VatExemptionText => "BT-120",
            Self::VatExemptionCode => "BT-121",
            Self::InvoicedQuantity => "BT-129",
            Self::UnitCode => "BT-130",
            Self::LineNetAmount => "BT-131",
            Self::ItemNetPrice => "BT-146",
            Self::PriceBaseQuantity => "BT-149",
            Self::ItemVatCategory => "BT-151",
            Self::ItemVatRate => "BT-152",
            Self::ItemName => "BT-153",
            Self::ItemDescription => "BT-154",
        }
    }

    /// Human-readable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvoiceNumber => "Invoice number",
            Self::IssueDate => "Invoice issue date",
            Self::InvoiceTypeCode => "Invoice type code",
            Self::Currency => "Invoice currency code",
            Self::DueDate => "Payment due date",
            Self::BuyerReference => "Buyer reference",
            Self::OrderReference => "Purchase order reference",
            Self::Note => "Invoice note",
            Self::SpecificationIdentifier => "Specification identifier",
            Self::SellerName => "Seller name",
            Self::SellerVatId => "Seller VAT identifier",
            Self::SellerTaxRegistrationId => "Seller tax registration identifier",
            Self::SellerEndpointId => "Seller endpoint identifier",
            Self::SellerCountry => "Seller address country code",
            Self::SellerTaxRepresentativeVatId => "Seller tax representative VAT identifier",
            Self::BuyerName => "Buyer name",
            Self::BuyerLegalRegistrationId => "Buyer legal registration identifier",
            Self::BuyerVatId => "Buyer VAT identifier",
            Self::BuyerEndpointId => "Buyer endpoint identifier",
            Self::BuyerCountry => "Buyer address country code",
            Self::ActualDeliveryDate => "Actual delivery date",
            Self::DeliverToCountry => "Deliver-to country code",
            Self::SumLineNet => "Sum of invoice line net amounts",
            Self::SumAllowances => "Sum of document-level allowances",
            Self::SumCharges => "Sum of document-level charges",
            Self::TaxExclusiveAmount => "Invoice total amount without VAT",
            Self::TaxTotal => "Invoice total VAT amount",
            Self::TaxInclusiveAmount => "Invoice total amount with VAT",
            Self::PaidAmount => "Paid amount",
            Self::RoundingAmount => "Rounding amount",
            Self::AmountDue => "Amount due for payment",
            Self::VatTaxableAmount => "VAT category taxable amount",
            Self::VatTaxAmount => "VAT category tax amount",
            Self::VatCategoryCode => "VAT category code",
            Self::VatCategoryRate => "VAT category rate",
            Self::VatExemptionText => "VAT exemption reason text",
            Self::VatExemptionCode => "VAT exemption reason code",
            Self::InvoicedQuantity => "Invoiced quantity",
            Self::UnitCode => "Invoiced quantity unit of measure code",
            Self::LineNetAmount => "Invoice line net amount",
            Self::ItemNetPrice => "Item net price",
            Self::PriceBaseQuantity => "Item price base quantity",
            Self::ItemVatCategory => "Invoiced item VAT category code",
            Self::ItemVatRate => "Invoiced item VAT rate",
            Self::ItemName => "Item name",
            Self::ItemDescription => "Item description",
        }
    }

    /// The business group this term belongs to, when it is inside one.
    pub const fn group(self) -> Option<Bg> {
        match self {
            Self::SellerName
            | Self::SellerVatId
            | Self::SellerTaxRegistrationId
            | Self::SellerEndpointId
            | Self::SellerCountry
            | Self::SellerTaxRepresentativeVatId => Some(Bg::Seller),
            Self::BuyerName
            | Self::BuyerLegalRegistrationId
            | Self::BuyerVatId
            | Self::BuyerEndpointId
            | Self::BuyerCountry => Some(Bg::Buyer),
            Self::SumLineNet
            | Self::SumAllowances
            | Self::SumCharges
            | Self::TaxExclusiveAmount
            | Self::TaxTotal
            | Self::TaxInclusiveAmount
            | Self::PaidAmount
            | Self::RoundingAmount
            | Self::AmountDue => Some(Bg::DocumentTotals),
            Self::VatTaxableAmount
            | Self::VatTaxAmount
            | Self::VatCategoryCode
            | Self::VatCategoryRate
            | Self::VatExemptionText
            | Self::VatExemptionCode => Some(Bg::VatBreakdown),
            Self::InvoicedQuantity
            | Self::UnitCode
            | Self::LineNetAmount
            | Self::ItemNetPrice
            | Self::PriceBaseQuantity
            | Self::ItemVatCategory
            | Self::ItemVatRate
            | Self::ItemName
            | Self::ItemDescription => Some(Bg::InvoiceLine),
            Self::InvoiceNumber
            | Self::IssueDate
            | Self::InvoiceTypeCode
            | Self::Currency
            | Self::DueDate
            | Self::BuyerReference
            | Self::OrderReference
            | Self::Note
            | Self::SpecificationIdentifier
            | Self::ActualDeliveryDate
            | Self::DeliverToCountry => None,
        }
    }
}

impl fmt::Display for Bt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_grouped() {
        assert_eq!(Bt::InvoiceNumber.code(), "BT-1");
        assert_eq!(Bt::LineNetAmount.code(), "BT-131");
        assert_eq!(Bt::LineNetAmount.group(), Some(Bg::InvoiceLine));
        assert_eq!(Bg::InvoiceLine.code(), "BG-25");
        assert_eq!(Bt::InvoiceNumber.group(), None);
    }
}
