//! Fuzz target for the Factur-X PDF extractor.
//!
//! The extractor is the container path: it opens a PDF, finds the embedded
//! XML, and refuses a bomb, an inconsistency, or anything that is not an
//! invoice. The corpus is seeded with a real Factur-X container so mutation can
//! reach the attachment logic rather than only the PDF header checks.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = import::extract_invoice(data);
});
