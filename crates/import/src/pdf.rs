//! Factur-X extraction: find the invoice XML inside a PDF/A-3 container.
//!
//! Only the container is read — the embedded file is located through the
//! catalog's name tree, with a scan of `/EmbeddedFile` streams as a fallback for
//! producers that omit the tree. The visual layer is never parsed: its text is
//! not the invoice, and treating it as one would invent terms the sender never
//! asserted in the XML.
//!
//! Everything here is bounded, because the file comes from a stranger: the input
//! size, the number of attachments, the depth of the name tree, and the size of
//! every decoded stream are capped. An encrypted container is refused rather
//! than guessed at.

use lopdf::{Dictionary, Document, LoadOptions, Object, Stream};

use syntax::read::MAX_DOCUMENT_BYTES;

/// Largest embedded file we will decode.
pub const MAX_EMBEDDED_BYTES: usize = MAX_DOCUMENT_BYTES;

/// Largest XMP packet we will decode. Packets are kilobytes, not megabytes.
pub const MAX_XMP_BYTES: usize = 256 * 1024;

/// Deepest name tree we will walk.
const MAX_TREE_DEPTH: usize = 8;

/// Most attachments we will consider before calling the container hostile.
const MAX_ATTACHMENTS: usize = 32;

/// Most references we will follow looking for a value.
const MAX_INDIRECTION: usize = 16;

/// Why a PDF did not yield an invoice.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// The file exceeds [`MAX_DOCUMENT_BYTES`].
    #[error("the PDF is {bytes} bytes; the limit is {limit}")]
    TooLarge {
        /// Size that was offered.
        bytes: usize,
        /// Size we accept.
        limit: usize,
    },
    /// The file could not be read as a PDF.
    #[error("the PDF cannot be read: {0}")]
    Malformed(String),
    /// The file is encrypted.
    #[error("encrypted PDFs cannot be read; ask the sender for an unencrypted copy")]
    Encrypted,
    /// Nothing inside looks like an invoice.
    #[error("{0}")]
    NoEmbeddedInvoice(String),
    /// An attachment could not be decoded within the size bound.
    #[error("cannot decode the embedded file {name:?} within {MAX_EMBEDDED_BYTES} bytes: {detail}")]
    EmbeddedTooLarge {
        /// Attachment that failed.
        name: String,
        /// Decoder detail.
        detail: String,
    },
    /// The XMP metadata names a different document than the container holds.
    #[error("the XMP metadata names {declared:?} but the embedded file is {found:?}")]
    XmpMismatch {
        /// File name declared in the XMP packet.
        declared: String,
        /// File name on the attachment.
        found: String,
    },
    /// The XMP metadata declares a document this reader does not handle.
    #[error("the XMP metadata declares a {declared:?} document, not an invoice")]
    NotAnInvoice {
        /// `fx:DocumentType` as written.
        declared: String,
    },
}

/// The invoice XML recovered from a PDF.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExtractedInvoice {
    /// The embedded document, as bytes.
    pub xml: Vec<u8>,
    /// File name the PDF gives the attachment, when it gives one.
    pub file_name: String,
}

/// One candidate attachment in the container.
struct Attachment<'a> {
    name: String,
    subtype: Option<String>,
    stream: &'a Stream,
}

impl Attachment<'_> {
    /// True when the name or the subtype says this is XML.
    fn looks_like_xml(&self) -> bool {
        self.name.to_ascii_lowercase().ends_with(".xml")
            || self
                .subtype
                .as_deref()
                .is_some_and(|subtype| subtype.to_ascii_lowercase().contains("xml"))
    }
}

/// Finds and decodes the invoice embedded in `pdf`.
///
/// A PDF without any attachment, or with attachments that are not XML, is an
/// error rather than a silent "nothing to do": the caller was told this file was
/// an invoice.
pub fn extract_invoice(pdf: &[u8]) -> Result<ExtractedInvoice, PdfError> {
    if pdf.len() > MAX_DOCUMENT_BYTES {
        return Err(PdfError::TooLarge {
            bytes: pdf.len(),
            limit: MAX_DOCUMENT_BYTES,
        });
    }

    let document = Document::load_mem_with_options(
        pdf,
        LoadOptions {
            max_decompressed_size: Some(MAX_EMBEDDED_BYTES),
            ..LoadOptions::default()
        },
    )
    .map_err(|error| PdfError::Malformed(error.to_string()))?;

    // `/Encrypt` means the object streams and strings are covered by a security
    // handler; we do not have the key and will not pretend the bytes are clear.
    if document.trailer.has(b"Encrypt") {
        return Err(PdfError::Encrypted);
    }

    let catalog = document
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|root| dict(&document, root))
        .ok_or_else(|| PdfError::Malformed("the container has no document catalog".to_owned()))?;

    let mut attachments = Vec::new();
    if let Some(names) = get(&document, catalog, b"Names").and_then(|names| dict(&document, names))
        && let Some(tree) = get(&document, names, b"EmbeddedFiles")
    {
        collect(&document, tree, 0, &mut attachments)?;
    }
    if attachments.is_empty() {
        attachments = scan(&document);
    }
    if attachments.is_empty() {
        return Err(PdfError::NoEmbeddedInvoice(
            "the PDF has no embedded document".to_owned(),
        ));
    }

    let chosen = choose(&attachments)?;
    let xml = chosen
        .stream
        .decompressed_content_with_limit(MAX_EMBEDDED_BYTES)
        .map_err(|error| PdfError::EmbeddedTooLarge {
            name: chosen.name.clone(),
            detail: error.to_string(),
        })?;

    cross_check_xmp(&document, catalog, chosen)?;

    Ok(ExtractedInvoice {
        xml,
        file_name: chosen.name.clone(),
    })
}

/// Picks the attachment that can be the invoice.
fn choose<'a, 'b>(attachments: &'b [Attachment<'a>]) -> Result<&'b Attachment<'a>, PdfError> {
    if let Some(xml) = attachments.iter().find(|attachment| attachment.looks_like_xml()) {
        return Ok(xml);
    }
    if attachments.len() == 1 {
        // A single attachment gets the benefit of the doubt: the XML reader will
        // say precisely why it is not an invoice, which is more useful than a
        // guess made here.
        return Ok(&attachments[0]);
    }
    Err(PdfError::NoEmbeddedInvoice(format!(
        "the PDF has {} embedded files and none of them is XML",
        attachments.len()
    )))
}

/// Collects attachments from a `/EmbeddedFiles` name tree.
fn collect<'a>(
    document: &'a Document,
    tree: &'a Object,
    depth: usize,
    out: &mut Vec<Attachment<'a>>,
) -> Result<(), PdfError> {
    if depth > MAX_TREE_DEPTH {
        return Err(PdfError::Malformed(
            "the embedded file name tree is too deep".to_owned(),
        ));
    }
    let Some(tree) = dict(document, tree) else {
        return Ok(());
    };

    if let Some(Object::Array(entries)) = get(document, tree, b"Names") {
        for pair in entries.chunks(2) {
            let [_, specification] = pair else {
                break;
            };
            if let Some(attachment) = attachment(document, specification) {
                out.push(attachment);
                if out.len() >= MAX_ATTACHMENTS {
                    return Err(PdfError::Malformed(format!(
                        "the PDF carries more than {MAX_ATTACHMENTS} attachments"
                    )));
                }
            }
        }
    }

    if let Some(Object::Array(kids)) = get(document, tree, b"Kids") {
        for kid in kids {
            collect(document, kid, depth + 1, out)?;
        }
    }
    Ok(())
}

/// Reads one file specification into its stream.
fn attachment<'a>(document: &'a Document, specification: &'a Object) -> Option<Attachment<'a>> {
    let specification = dict(document, specification)?;
    let name = get(document, specification, b"UF")
        .or_else(|| get(document, specification, b"F"))
        .and_then(text_of)
        .unwrap_or_default();
    let stream = get(document, specification, b"EF")
        .and_then(|files| dict(document, files))
        .and_then(|files| get(document, files, b"F"))
        .and_then(|file| match file {
            Object::Stream(stream) => Some(stream),
            _ => None,
        })?;
    Some(Attachment {
        name,
        subtype: subtype(stream),
        stream,
    })
}

/// Fallback for producers that attach a file without a name tree.
fn scan(document: &Document) -> Vec<Attachment<'_>> {
    document
        .objects
        .values()
        .filter_map(|object| match object {
            Object::Stream(stream) if stream.dict.has_type(b"EmbeddedFile") => Some(Attachment {
                name: String::new(),
                subtype: subtype(stream),
                stream,
            }),
            _ => None,
        })
        .take(MAX_ATTACHMENTS)
        .collect()
}

fn subtype(stream: &Stream) -> Option<String> {
    stream.dict.get(b"Subtype").ok().and_then(text_of)
}

/// Compares the XMP packet's claim with what the container actually holds.
///
/// The packet is metadata written by the producer and the attachment is the
/// payload; when they disagree about the file name the reader cannot tell which
/// one the counterparty meant, so it refuses instead of picking a favourite.
fn cross_check_xmp(document: &Document, catalog: &Dictionary, chosen: &Attachment<'_>) -> Result<(), PdfError> {
    let Some(metadata) = get(document, catalog, b"Metadata") else {
        return Ok(());
    };
    let Object::Stream(metadata) = metadata else {
        return Ok(());
    };
    let Ok(packet) = metadata.decompressed_content_with_limit(MAX_XMP_BYTES) else {
        return Ok(());
    };
    let packet = String::from_utf8_lossy(&packet);

    if let Some(declared) = element(&packet, "fx:DocumentType")
        && !declared.eq_ignore_ascii_case("INVOICE")
    {
        return Err(PdfError::NotAnInvoice { declared });
    }

    if let Some(declared) = element(&packet, "fx:DocumentFileName")
        && !chosen.name.is_empty()
        && declared != chosen.name
    {
        return Err(PdfError::XmpMismatch {
            declared,
            found: chosen.name.clone(),
        });
    }
    Ok(())
}

/// Text of the first `<tag>…</tag>`, which is all the XMP packet needs here.
fn element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let value = xml[start..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn get<'a>(document: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    resolve(document, dict.get(key).ok()?)
}

fn dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve(document, object)? {
        Object::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    let mut current = object;
    for _ in 0..MAX_INDIRECTION {
        match current {
            Object::Reference(id) => current = document.get_object(*id).ok()?,
            resolved => return Some(resolved),
        }
    }
    None
}

fn text_of(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use lopdf::{Dictionary, Document, Object, Stream, dictionary};

    /// A container with a name tree holding `files` as `(name, subtype, bytes)`.
    pub(crate) fn document_with(files: &[(&str, &str, &[u8])], metadata: Option<&str>, encrypted: bool) -> Vec<u8> {
        let mut document = Document::with_version("1.7");

        let mut entries = Vec::new();
        for (name, subtype, content) in files {
            let stream_id = document.add_object(Stream::new(
                dictionary! {
                    "Type" => "EmbeddedFile",
                    "Subtype" => *subtype,
                },
                content.to_vec(),
            ));
            let specification_id = document.add_object(dictionary! {
                "Type" => "Filespec",
                "F" => Object::string_literal(*name),
                "UF" => Object::string_literal(*name),
                "EF" => dictionary! { "F" => stream_id },
            });
            entries.push(Object::string_literal(*name));
            entries.push(Object::Reference(specification_id));
        }

        let names_id = document.add_object(dictionary! {
            "EmbeddedFiles" => dictionary! { "Names" => entries },
        });
        let pages_id = document.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => Vec::<Object>::new(),
            "Count" => 0,
        });

        let mut catalog: Dictionary = dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "Names" => names_id,
        };
        if let Some(metadata) = metadata {
            let metadata_id = document.add_object(Stream::new(
                dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
                metadata.as_bytes().to_vec(),
            ));
            catalog.set("Metadata", metadata_id);
        }
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", catalog_id);
        if encrypted {
            document.trailer.set("Encrypt", dictionary! {});
        }

        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("write the container");
        bytes
    }

    /// A normal Factur-X container: one XML attachment, optionally with XMP.
    pub(crate) fn container(xml: &str, metadata: Option<&str>) -> Vec<u8> {
        document_with(&[("factur-x.xml", "text/xml", xml.as_bytes())], metadata, false)
    }

    /// The Factur-X XMP packet, with the two fields this reader cross-checks.
    pub(crate) fn xmp(file_name: &str, document_type: &str) -> String {
        format!(
            r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:fx="urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#" rdf:about="">
      <fx:DocumentType>{document_type}</fx:DocumentType>
      <fx:DocumentFileName>{file_name}</fx:DocumentFileName>
      <fx:Version>1.0</fx:Version>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use lopdf::{Document, Object, Stream, dictionary};

    use super::*;

    const XML: &str = "<Invoice><ID>SUP-1</ID></Invoice>";

    #[test]
    fn finds_the_embedded_invoice() {
        let pdf = fixtures::container(XML, None);
        let extracted = extract_invoice(&pdf).expect("extract");
        assert_eq!(extracted.xml, XML.as_bytes());
        assert_eq!(extracted.file_name, "factur-x.xml");
    }

    #[test]
    fn accepts_xmp_that_names_the_same_file() {
        let metadata = fixtures::xmp("factur-x.xml", "INVOICE");
        let pdf = fixtures::container(XML, Some(&metadata));
        assert!(extract_invoice(&pdf).is_ok());
    }

    #[test]
    fn refuses_when_the_xmp_names_another_file() {
        let metadata = fixtures::xmp("other.xml", "INVOICE");
        let pdf = fixtures::container(XML, Some(&metadata));
        assert!(matches!(extract_invoice(&pdf), Err(PdfError::XmpMismatch { .. })));
    }

    #[test]
    fn refuses_a_document_type_that_is_not_an_invoice() {
        let metadata = fixtures::xmp("factur-x.xml", "ORDER");
        let pdf = fixtures::container(XML, Some(&metadata));
        assert!(matches!(extract_invoice(&pdf), Err(PdfError::NotAnInvoice { .. })));
    }

    #[test]
    fn a_pdf_without_an_attachment_is_reported() {
        let pdf = fixtures::document_with(&[], None, false);
        assert!(matches!(extract_invoice(&pdf), Err(PdfError::NoEmbeddedInvoice(_))));
    }

    #[test]
    fn several_attachments_without_xml_are_reported() {
        let pdf = fixtures::document_with(
            &[
                ("notes.txt", "text/plain", b"hello"),
                ("image.png", "image/png", b"pixels"),
            ],
            None,
            false,
        );
        let error = extract_invoice(&pdf).expect_err("no invoice here");
        assert!(error.to_string().contains("none of them is XML"), "{error}");
    }

    #[test]
    fn an_encrypted_container_is_refused() {
        let pdf = fixtures::container(XML, None);
        let mut document = Document::load_mem(&pdf).expect("load");
        document.trailer.set("Encrypt", dictionary! {});
        let mut encrypted = Vec::new();
        document.save_to(&mut encrypted).expect("save");
        assert!(matches!(extract_invoice(&encrypted), Err(PdfError::Encrypted)));
    }

    #[test]
    fn falls_back_to_scanning_embedded_file_streams() {
        // A producer that attaches the file but omits the name tree.
        let mut document = Document::with_version("1.7");
        document.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile", "Subtype" => "text/xml" },
            XML.as_bytes().to_vec(),
        ));
        let pages_id = document.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => Vec::<Object>::new(),
            "Count" => 0,
        });
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("save");

        let extracted = extract_invoice(&bytes).expect("scan finds the stream");
        assert_eq!(extracted.xml, XML.as_bytes());
    }

    #[test]
    fn decodes_a_compressed_attachment() {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(XML.as_bytes()).expect("compress");
        let compressed = encoder.finish().expect("finish");

        let mut document = Document::with_version("1.7");
        let stream_id = document.add_object(Stream::new(
            dictionary! {
                "Type" => "EmbeddedFile",
                "Subtype" => "text/xml",
                "Filter" => "FlateDecode",
            },
            compressed,
        ));
        let specification_id = document.add_object(dictionary! {
            "Type" => "Filespec",
            "UF" => Object::string_literal("factur-x.xml"),
            "EF" => dictionary! { "F" => stream_id },
        });
        let names_id = document.add_object(dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![Object::string_literal("factur-x.xml"), Object::Reference(specification_id)],
            },
        });
        let pages_id = document.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => Vec::<Object>::new(),
            "Count" => 0,
        });
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "Names" => names_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("save");

        let extracted = extract_invoice(&bytes).expect("extract");
        assert_eq!(extracted.xml, XML.as_bytes());
    }

    #[test]
    fn an_oversized_pdf_is_refused_before_parsing() {
        let pdf = vec![b'%'; MAX_DOCUMENT_BYTES + 1];
        assert!(matches!(extract_invoice(&pdf), Err(PdfError::TooLarge { .. })));
    }

    #[test]
    fn a_truncated_pdf_is_reported() {
        let pdf = b"%PDF-1.7\nthis is not a PDF";
        assert!(matches!(extract_invoice(pdf), Err(PdfError::Malformed(_))));
    }
}
