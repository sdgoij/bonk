//! Prints the S5 artefact hashes on the host.
//!
//! Run with `cargo run -p ffi --example artefact_probe` and compare the output
//! with the values the Android probe prints.

fn main() -> Result<(), bonkc::AppError> {
    let hashes = bonkc::artefact_hashes()?;
    println!("ubl_sha256={}", hashes.ubl_sha256);
    println!("pdf_sha256={}", hashes.pdf_sha256);
    println!("ubl_len={}", hashes.ubl_len);
    println!("pdf_len={}", hashes.pdf_len);
    Ok(())
}
