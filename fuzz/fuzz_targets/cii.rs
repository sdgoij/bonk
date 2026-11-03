//! Fuzz target for the CII D16B reader.
//!
//! `read_document` dispatches on the root element, so this target exercises the
//! CII path the way the import pipeline reaches it. The corpus is seeded with a
//! Peppol CII invoice so mutation stays near the shape the reader understands.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(xml) = std::str::from_utf8(data) {
        let _ = syntax::read::read_document(xml);
    }
});
