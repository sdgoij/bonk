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

/// The Factur-X version whose XMP namespace and attachment name are used.
pub const VERSION: &str = "1.0";

/// Writes the document as PDF/A-3 with `xml` attached at a stated level.
///
/// `xml` is the CII invoice, which must already have been validated: this layer
/// draws and carries, it does not judge. `conformance_level` is the level the
/// payload was written at, as Factur-X spells it (`MINIMUM`, `BASIC WL`, `BASIC`,
/// `EN 16931`, `EXTENDED`); it comes from `crux::FacturXProfile`, and the packet
/// would claim something the file is not if it were invented here.
pub(crate) fn write(document: &Document, xml: &[u8], conformance_level: &str) -> Result<Vec<u8>, RenderError> {
    let attachment = pdf::Attachment {
        name: FILE_NAME,
        mime: "text/xml",
        description: "Factur-X invoice",
        data: xml,
    };
    let bytes = pdf::write_with(document, Some(&attachment))?;
    declare(&bytes, conformance_level)
}

/// Adds what Factur-X says about the attached file to the XMP packet.
///
/// krilla writes the PDF/A XMP and its extension schema, but it has no hook for a
/// format-specific schema, and Factur-X needs one: the packet says which file is
/// attached and which level it was written at. The packet krilla writes is an
/// uncompressed stream, so this splices the two statements in at fixed anchors
/// rather than parsing and rewriting the whole file. A packet without those
/// anchors is refused rather than shipped: a container that claims Factur-X and
/// does not say so in its metadata is worse than a failed render.
///
/// The metadata object is then rebuilt around the longer packet, and every object
/// after it moves by however much the packet grew. A PDF's cross-reference table
/// stores byte offsets rather than order, so the table is corrected by the same
/// amount: rewriting the object in place would leave `/Length` describing the
/// bytes that used to be there, and a strict reader stops at that byte.
fn declare(pdf: &[u8], conformance_level: &str) -> Result<Vec<u8>, RenderError> {
    let (packet_start, packet_end) = packet(pdf)?;
    let object = object_around(pdf, packet_start)?;

    let mut xmp = pdf[packet_start..packet_end].to_vec();
    xmp = splice(xmp, SCHEMA_ANCHOR, SCHEMA.as_bytes())?;
    xmp = splice(xmp, DESCRIPTION_ANCHOR, description(conformance_level).as_bytes())?;

    let mut rebuilt = format!(
        "{} 0 obj\n<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n",
        object.number,
        xmp.len()
    )
    .into_bytes();
    rebuilt.extend_from_slice(&xmp);
    rebuilt.extend_from_slice(b"\nendstream\nendobj");

    let delta = rebuilt.len() as isize - (object.end - object.start) as isize;
    let mut bytes = Vec::with_capacity((pdf.len() as isize + delta).max(0) as usize);
    bytes.extend_from_slice(&pdf[..object.start]);
    bytes.extend_from_slice(&rebuilt);
    bytes.extend_from_slice(&pdf[object.end..]);

    let xref = startxref(pdf)?;
    let moved = (xref as isize + delta) as usize;
    correct_xref(&mut bytes, moved, object.start, delta)?;
    correct_startxref(&mut bytes, moved)?;
    Ok(bytes)
}

/// The metadata object: its number, and where its text starts and ends.
struct Object {
    number: usize,
    start: usize,
    end: usize,
}

/// Finds the object that contains the XMP packet that starts at `at`.
fn object_around(pdf: &[u8], at: usize) -> Result<Object, RenderError> {
    let marker = rposition(&pdf[..at], b" 0 obj")?;
    let mut start = marker;
    while start > 0 && pdf[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let number = std::str::from_utf8(&pdf[start..marker])
        .ok()
        .and_then(|text| text.parse::<usize>().ok())
        .ok_or_else(|| RenderError::Pdf("the object around the XMP packet has no number".to_owned()))?;
    let end = position(&pdf[at..], b"endobj")? + at + b"endobj".len();
    Ok(Object { number, start, end })
}

/// The bounds of the XMP packet: from `<?xpacket begin` to the end of `<?xpacket end…?>`.
fn packet(pdf: &[u8]) -> Result<(usize, usize), RenderError> {
    let start = position(pdf, b"<?xpacket begin")?;
    let declared = position(&pdf[start..], b"<?xpacket end")? + start;
    let end = position(&pdf[declared..], b"?>")? + declared + 2;
    Ok((start, end))
}

/// Inserts `addition` before the first occurrence of `anchor`.
fn splice(bytes: Vec<u8>, anchor: &[u8], addition: &[u8]) -> Result<Vec<u8>, RenderError> {
    let at = position(&bytes, anchor)?;
    let mut out = Vec::with_capacity(bytes.len() + addition.len());
    out.extend_from_slice(&bytes[..at]);
    out.extend_from_slice(addition);
    out.extend_from_slice(&bytes[at..]);
    Ok(out)
}

/// The first offset of `needle`, or an error naming it.
fn position(haystack: &[u8], needle: &[u8]) -> Result<usize, RenderError> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .ok_or_else(|| {
            RenderError::Pdf(format!(
                "the PDF has no {:?}, so the Factur-X statements have nowhere to go",
                String::from_utf8_lossy(needle)
            ))
        })
}

/// The last offset of `needle`, or an error naming it.
fn rposition(haystack: &[u8], needle: &[u8]) -> Result<usize, RenderError> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
        .ok_or_else(|| {
            RenderError::Pdf(format!(
                "the PDF has no {:?}, so it cannot be extended",
                String::from_utf8_lossy(needle)
            ))
        })
}

/// The offset the trailer's `startxref` names: where the cross-reference table is.
fn startxref(pdf: &[u8]) -> Result<usize, RenderError> {
    let at = rposition(pdf, b"startxref")?;
    let digits: Vec<u8> = pdf[at + b"startxref".len()..]
        .iter()
        .skip_while(|byte| !byte.is_ascii_digit())
        .take_while(|byte| byte.is_ascii_digit())
        .copied()
        .collect();
    let text = String::from_utf8_lossy(&digits);
    text.parse()
        .map_err(|_| RenderError::Pdf(format!("the trailer's startxref is not an offset: {text:?}")))
}

/// Rewrites the entries whose objects moved when the packet grew.
///
/// A classic table is a sequence of 20-byte entries: a ten-digit offset, a
/// five-digit generation, and `n` or `f`. The subsection headers between them are
/// left alone; the walk stops at the trailer.
fn correct_xref(bytes: &mut [u8], xref: usize, moved_from: usize, delta: isize) -> Result<(), RenderError> {
    if !bytes.get(xref..).is_some_and(|rest| rest.starts_with(b"xref")) {
        return Err(RenderError::Pdf(
            "the cross-reference table is not where startxref says, so it cannot be corrected".to_owned(),
        ));
    }
    let mut cursor = xref + b"xref".len();
    // Skip the line break after the keyword.
    while bytes.get(cursor).is_some_and(|byte| *byte == b'\r' || *byte == b'\n') {
        cursor += 1;
    }
    loop {
        let (line_end, offset) = {
            let line = &bytes[cursor..];
            if line.starts_with(b"trailer") {
                return Ok(());
            }
            // Every line in the table ends with a line break; a table without one
            // is not one we should be editing.
            let Some(end) = line.iter().position(|byte| *byte == b'\n') else {
                return Err(RenderError::Pdf(
                    "the cross-reference table ends without a trailer".to_owned(),
                ));
            };
            let offset = if is_entry(line) {
                std::str::from_utf8(&line[..10])
                    .ok()
                    .and_then(|text| text.parse::<usize>().ok())
            } else {
                None
            };
            (end, offset)
        };
        if let Some(offset) = offset.filter(|offset| *offset > moved_from) {
            let moved = (offset as isize + delta) as usize;
            let rewritten = format!("{moved:010}");
            if rewritten.len() != 10 {
                return Err(RenderError::Pdf(
                    "the corrected offset no longer fits an entry".to_owned(),
                ));
            }
            bytes[cursor..cursor + 10].copy_from_slice(rewritten.as_bytes());
        }
        cursor += line_end + 1;
    }
}

/// True when the line is a cross-reference entry rather than a subsection header.
fn is_entry(line: &[u8]) -> bool {
    line.len() >= 18
        && line[..10].iter().all(u8::is_ascii_digit)
        && line[10] == b' '
        && line[11..16].iter().all(u8::is_ascii_digit)
        && line[16] == b' '
        && (line[17] == b'n' || line[17] == b'f')
}

/// Points the trailer at the table's new position.
///
/// The value may be a digit longer than before, which is why the tail is rebuilt
/// rather than patched in place: nothing follows it but the end-of-file marker.
fn correct_startxref(bytes: &mut Vec<u8>, xref: usize) -> Result<(), RenderError> {
    let at = rposition(bytes, b"startxref")?;
    bytes.truncate(at + b"startxref".len());
    bytes.extend_from_slice(format!("\n{xref}\n%%EOF\n").as_bytes());
    Ok(())
}

/// The anchor the extension schema is written before.
const SCHEMA_ANCHOR: &[u8] = b"</rdf:Bag></pdfaExtension:schemas>";

/// The anchor the Factur-X description is written before: the end of the packet.
const DESCRIPTION_ANCHOR: &[u8] = b"</rdf:RDF>";

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
/// version and level it was written at.
fn description(conformance_level: &str) -> String {
    format!(
        concat!(
            r#"<rdf:Description xmlns:fx="urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#" rdf:about="">"#,
            "<fx:DocumentType>INVOICE</fx:DocumentType>",
            "<fx:DocumentFileName>{}</fx:DocumentFileName>",
            "<fx:Version>{}</fx:Version>",
            "<fx:ConformanceLevel>{}</fx:ConformanceLevel></rdf:Description>",
        ),
        FILE_NAME, VERSION, conformance_level
    )
}

#[cfg(test)]
mod tests {
    use super::{is_entry, startxref};
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
        let bytes = crate::renderer().render_facturx(&sample(), xml, "EN 16931").unwrap();

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
    fn the_packet_states_the_level_it_was_handed() {
        // Factur-X names the levels with spaces and case that matter, so the
        // string has to be carried through untouched rather than normalised.
        for level in ["MINIMUM", "BASIC WL", "BASIC", "EN 16931", "EXTENDED"] {
            let bytes = crate::renderer()
                .render_facturx(&sample(), b"<rsm:CrossIndustryInvoice/>", level)
                .unwrap();
            let claim = format!("<fx:ConformanceLevel>{level}</fx:ConformanceLevel>");
            assert!(contains(&bytes, claim.as_bytes()), "{level} is not in the packet");
        }
    }

    #[test]
    fn the_container_is_byte_stable() {
        let xml = b"<rsm:CrossIndustryInvoice/>";
        let renderer = crate::renderer();
        let first = renderer.render_facturx(&sample(), xml, "EN 16931").unwrap();
        let second = renderer.render_facturx(&sample(), xml, "EN 16931").unwrap();
        assert_eq!(first, second, "an artefact is hashed, so it has to be stable");
    }

    #[test]
    fn every_object_is_where_the_cross_reference_table_says() {
        // The Factur-X statements are spliced into the XMP packet, which moves
        // every byte after it. A reader believes the offsets, not the object
        // order, so this is the invariant that splice has to maintain.
        let bytes = crate::renderer()
            .render_facturx(&sample(), b"<rsm:CrossIndustryInvoice/>", "EN 16931")
            .unwrap();
        let checked = xref_points_at_its_objects(&bytes);
        assert!(checked >= 20, "only {checked} objects were checked");
    }

    #[test]
    fn the_check_would_notice_a_broken_table() {
        // The gate is only worth having if it fails on the mistake it exists for:
        // shift the file without moving the offsets and the walk must complain.
        let bytes = crate::renderer()
            .render_facturx(&sample(), b"<rsm:CrossIndustryInvoice/>", "EN 16931")
            .unwrap();
        let mut broken = bytes.clone();
        let xref = startxref(&bytes).unwrap();
        broken.insert(xref - 1, b' ');
        let outcome = std::panic::catch_unwind(|| xref_points_at_its_objects(&broken));
        assert!(outcome.is_err(), "a moved table still looked correct");
    }

    /// Walks the cross-reference table the way a strict reader does and returns
    /// how many objects it checked.
    fn xref_points_at_its_objects(pdf: &[u8]) -> usize {
        let xref = startxref(pdf).expect("the trailer names the table");
        assert!(pdf[xref..].starts_with(b"xref"), "the table is where startxref says");

        let mut cursor = xref + b"xref".len();
        while pdf.get(cursor).is_some_and(|byte| *byte == b'\r' || *byte == b'\n') {
            cursor += 1;
        }
        let mut expected = 0_usize;
        let mut checked = 0_usize;
        loop {
            let end = cursor + pdf[cursor..].iter().position(|byte| *byte == b'\n').expect("a line");
            let line = &pdf[cursor..end];
            if line.starts_with(b"trailer") {
                return checked;
            }
            if is_entry(line) {
                let target: usize = std::str::from_utf8(&line[..10])
                    .expect("digits")
                    .parse()
                    .expect("an offset");
                if line[17] == b'n' {
                    let rest = &pdf[target..];
                    let header = rest.iter().position(|byte| *byte == b' ').expect("a header");
                    let number: usize = std::str::from_utf8(&rest[..header])
                        .expect("digits")
                        .parse()
                        .expect("an object number");
                    assert!(
                        rest[header..].starts_with(b" 0 obj"),
                        "offset {target} is not an object"
                    );
                    assert_eq!(number, expected, "offset {target} names the wrong object");
                    checked += 1;
                }
                expected += 1;
            } else {
                // A subsection header: the first object number it covers.
                expected = std::str::from_utf8(line)
                    .expect("text")
                    .split_whitespace()
                    .next()
                    .expect("a start")
                    .parse()
                    .expect("a number");
            }
            cursor = end + 1;
        }
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
