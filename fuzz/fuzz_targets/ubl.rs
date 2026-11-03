//! Fuzz target for the UBL 2.1 reader.
//!
//! The reader takes `&str`, so bytes that are not UTF-8 cannot be a document:
//! the import path checks the encoding before the reader runs. The property the
//! target guards is that any UTF-8 document the reader is offered is answered or
//! refused — never a panic, never a hang.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(xml) = std::str::from_utf8(data) {
        let _ = syntax::read::read_invoice(xml);
    }
});
