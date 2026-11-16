//! Factur-X: a PDF/A-3 container that carries the invoice as CII.
//!
//! A Factur-X file is three things at once: the page a person reads, a PDF/A-3
//! document, and a container for the XML an accounting system reads. The layout
//! and the PDF/A serialisation are already in `pdf`; this module adds the
//! attachment and the Factur-X statements in the XMP packet that name it.

use crate::layout::Document;
use crate::{RenderError, pdf};

/// The attachment name Factur-X readers look for.
///
/// The specification fixes it: a reader that finds `factur-x.xml` knows the file
/// is an invoice without reading anything else.
pub const FILE_NAME: &str = "factur-x.xml";

/// The profile this writer produces: EN 16931, the level Factur-X calls
/// `EN 16931` and ZUGFeRD calls `COMFORT`.
pub const PROFILE: &str = "EN 16931";

/// The Factur-X version whose XMP namespace and attachment name are used.
pub const VERSION: &str = "1.0";

/// Writes the document as PDF/A-3 with `xml` attached.
///
/// `xml` is the CII invoice, which must already have been validated: this layer
/// draws and carries, it does not judge.
pub(crate) fn write(document: &Document, xml: &[u8]) -> Result<Vec<u8>, RenderError> {
    let attachment = pdf::Attachment {
        name: FILE_NAME,
        mime: "text/xml",
        description: "Factur-X invoice",
        data: xml,
    };
    let bytes = pdf::write_with(document, Some(&attachment))?;
    declare(&bytes)
}

/// Adds what Factur-X says about the attached file to the XMP packet.
///
/// krilla writes the PDF/A XMP and its extension schema, but it has no hook for a
/// format-specific schema, and Factur-X needs one: the packet says which file is
/// attached and which profile it is. The packet krilla writes is an uncompressed
/// stream, so this splices the two statements in at fixed anchors rather than
/// parsing and rewriting the whole file. A packet without those anchors is refused
/// rather than shipped: a container that claims Factur-X and does not say so in its
/// metadata is worse than a failed render.
fn declare(pdf: &[u8]) -> Result<Vec<u8>, RenderError> {
    let bytes = splice(pdf.to_vec(), b"</rdf:Bag></pdfaExtension:schemas>", SCHEMA.as_bytes())?;
    splice(bytes, b"</rdf:RDF>", description().as_bytes())
}

/// Inserts `addition` before the first occurrence of `anchor`.
fn splice(bytes: Vec<u8>, anchor: &[u8], addition: &[u8]) -> Result<Vec<u8>, RenderError> {
    let position = bytes
        .windows(anchor.len())
        .position(|window| window == anchor)
        .ok_or_else(|| {
            RenderError::Pdf(format!(
                "the XMP packet has no {:?} to write the Factur-X statements beside",
                String::from_utf8_lossy(anchor)
            ))
        })?;
    let mut out = Vec::with_capacity(bytes.len() + addition.len());
    out.extend_from_slice(&bytes[..position]);
    out.extend_from_slice(addition);
    out.extend_from_slice(&bytes[position..]);
    Ok(out)
}

/// The extension schema that declares the `fx` namespace, which PDF/A-3 requires
/// for a file format it does not know.
const SCHEMA: &str = concat!(
    r#"<rdf:li rdf:parseType="Resource"><pdfaSchema:schema>Factur-X PDFA Extension Schema</pdfaSchema:schema>"#,
    r#"<pdfaSchema:namespaceURI>urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#</pdfaSchema:namespaceURI>"#,
    r#"<pdfaSchema:prefix>fx</pdfaSchema:prefix><pdfaSchema:property><rdf:Seq>"#,
    r#"<rdf:li rdf:parseType="Resource"><pdfaProperty:name>DocumentFileName</pdfaProperty:name>"#,
    r#"<pdfaProperty:valueType>Text</pdfaProperty:valueType><pdfaProperty:category>external</pdfaProperty:category>"#,
    r#"<pdfaProperty:description>name of the embedded XML invoice file</pdfaProperty:description></rdf:li>"#,
    r#"<rdf:li rdf:parseType="Resource"><pdfaProperty:name>DocumentType</pdfaProperty:name>"#,
    r#"<pdfaProperty:valueType>Text</pdfaProperty:valueType><pdfaProperty:category>external</pdfaProperty:category>"#,
    r#"<pdfaProperty:description>INVOICE</pdfaProperty:description></rdf:li>"#,
    r#"<rdf:li rdf:parseType="Resource"><pdfaProperty:name>Version</pdfaProperty:name>"#,
    r#"<pdfaProperty:valueType>Text</pdfaProperty:valueType><pdfaProperty:category>external</pdfaProperty:category>"#,
    r#"<pdfaProperty:description>The actual version of the Factur-X XML schema</pdfaProperty:description></rdf:li>"#,
    r#"<rdf:li rdf:parseType="Resource"><pdfaProperty:name>ConformanceLevel</pdfaProperty:name>"#,
    r#"<pdfaProperty:valueType>Text</pdfaProperty:valueType><pdfaProperty:category>external</pdfaProperty:category>"#,
    r#"<pdfaProperty:description>The conformance level of the embedded Factur-X data</pdfaProperty:description></rdf:li>"#,
    r#"</rdf:Seq></pdfaSchema:property></rdf:li>"#,
);

/// What the packet says about this document: the attached file, its type, and the
/// version and level this writer produces.
fn description() -> String {
    format!(
        concat!(
            r#"<rdf:Description xmlns:fx="urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#" rdf:about="">"#,
            "<fx:DocumentType>INVOICE</fx:DocumentType>",
            "<fx:DocumentFileName>{}</fx:DocumentFileName>",
            "<fx:Version>{}</fx:Version>",
            "<fx:ConformanceLevel>{}</fx:ConformanceLevel></rdf:Description>",
        ),
        FILE_NAME, VERSION, PROFILE
    )
}

#[cfg(test)]
mod tests {
    use crate::{InvoiceData, Language, PartyData, PdfRenderer};

    fn sample() -> InvoiceData {
        InvoiceData {
            title: "Invoice".to_owned(),
            labels: Language::English.labels(),
            number: "2026-0001".to_owned(),
            issue_date: "2026-10-27".to_owned(),
            due_date: Some("2026-11-26".to_owned()),
            currency: "EUR".to_owned(),
            seller: PartyData {
                name: "Seller BV".to_owned(),
                address: "Main 1, Amsterdam, NL".to_owned(),
                kvk: None,
                vat: None,
            },
            customer: PartyData {
                name: "Customer BV".to_owned(),
                address: "Rotterdam, NL".to_owned(),
                kvk: None,
                vat: None,
            },
            lines: Vec::new(),
            net: "100.00".to_owned(),
            tax: "21.00".to_owned(),
            gross: "121.00".to_owned(),
            notes: None,
        }
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn the_container_carries_the_xml_and_declares_the_profile() {
        let xml = b"<rsm:CrossIndustryInvoice/>";
        let bytes = crate::renderer().render_facturx(&sample(), xml).unwrap();

        // The file is inside, attached as the alternative representation of the
        // page, and the XML is stored as it was handed over.
        assert!(contains(&bytes, b"factur-x.xml"), "the attachment is named");
        assert!(contains(&bytes, b"/Alternative"), "it is an alternative, not data");
        assert!(contains(&bytes, xml), "the XML is not re-encoded on the way in");

        // PDF/A-3B, and the Factur-X statements in the same packet.
        assert!(contains(&bytes, b"<pdfaid:part>3</pdfaid:part>"), "PDF/A-3");
        assert!(
            contains(&bytes, b"<pdfaid:conformance>B</pdfaid:conformance>"),
            "conformance level B"
        );
        assert!(
            contains(&bytes, b"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"),
            "the fx namespace, declared in the extension schema"
        );
        assert!(
            contains(&bytes, b"<fx:DocumentFileName>factur-x.xml</fx:DocumentFileName>"),
            "the packet names the file a reader should look for"
        );
        assert!(
            contains(&bytes, b"<fx:ConformanceLevel>EN 16931</fx:ConformanceLevel>"),
            "and the profile this writer produces"
        );
    }

    #[test]
    fn the_container_is_byte_stable() {
        let xml = b"<rsm:CrossIndustryInvoice/>";
        let renderer = crate::renderer();
        let first = renderer.render_facturx(&sample(), xml).unwrap();
        let second = renderer.render_facturx(&sample(), xml).unwrap();
        assert_eq!(first, second, "an artefact is hashed, so it has to be stable");
    }

    #[test]
    fn a_plain_pdf_is_not_declared_as_pdf_a() {
        let bytes = crate::renderer().render(&sample()).unwrap();
        assert!(
            !contains(&bytes, b"<pdfaid:part>"),
            "a print-ready PDF makes no archival claim"
        );
    }
}
