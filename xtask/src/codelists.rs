//! Regenerates the vendored EN 16931 code lists from the official Schematron.
//!
//! The official codelist Schematron embeds every code list as one
//! space-delimited literal inside the `test` attribute of its assertion.
//! Extracting them mechanically keeps the bundles reproducible and turns an
//! upstream release into a reviewable diff instead of a hand-copied list.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

/// Upstream location of the codelist Schematron.
pub const SOURCE_URL: &str = "https://raw.githubusercontent.com/ConnectingEurope/eInvoicing-EN16931/master/ubl/schematron/codelist/EN16931-UBL-codes.sch";

/// SHA-256 of the upstream file this bundle was extracted from.
///
/// Extraction refuses a different file so an upstream change is a deliberate
/// act: bump this constant and the bundle label together.
pub const SOURCE_SHA256: &str = "933c2ec261f7fc219aa0c7fa9f0b67c7fe156600b6fbb9a0eb045f583d173bd9";

/// Date the pinned upstream file was retrieved.
pub const RETRIEVED: &str = "2026-10-27";

/// Snapshot label; also the bundle directory name.
pub const BUNDLE: &str = "en16931-ubl-codes-2026-10";

const DEFAULT_SOURCE: &str = "target/validator/EN16931-UBL-codes.sch";

/// One vendored list and where it comes from in the Schematron.
struct Bundle {
    /// File name inside the bundle directory.
    file: &'static str,
    /// Rule whose assertion carries the list.
    rule: &'static str,
    /// Index among the multi-word literals of that assertion.
    literal: usize,
    /// Human-readable codelist name for the manifest.
    codelist: &'static str,
}

const BUNDLES: &[Bundle] = &[
    Bundle {
        file: "untdid-1001-invoice.txt",
        rule: "BR-CL-01",
        literal: 0,
        codelist: "UN/TDID 1001 invoice type codes",
    },
    Bundle {
        file: "untdid-1001-credit-note.txt",
        rule: "BR-CL-01",
        literal: 1,
        codelist: "UN/TDID 1001 credit note type codes",
    },
    Bundle {
        file: "iso-4217.txt",
        rule: "BR-CL-04",
        literal: 0,
        codelist: "ISO 4217 alpha-3 currency codes",
    },
    Bundle {
        file: "iso-3166-1.txt",
        rule: "BR-CL-14",
        literal: 0,
        codelist: "ISO 3166-1 country codes",
    },
    Bundle {
        file: "uncl5305.txt",
        rule: "BR-CL-17",
        literal: 0,
        codelist: "UNCL 5305 VAT category codes",
    },
    Bundle {
        file: "vatex.txt",
        rule: "BR-CL-22",
        literal: 0,
        codelist: "CEF VATEX exemption reason codes",
    },
    Bundle {
        file: "un-ece-rec20-21.txt",
        rule: "BR-CL-23",
        literal: 0,
        codelist: "UN/ECE Recommendation 20 unit codes with Recommendation 21 extensions",
    },
    Bundle {
        file: "eas.txt",
        rule: "BR-CL-25",
        literal: 0,
        codelist: "CEF EAS endpoint scheme identifiers",
    },
];

/// Entry point for `xtask codelists`.
pub fn run(args: &[String]) -> ExitCode {
    let mut check = false;
    let mut source = None;
    let mut accept_new_source = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => check = true,
            "--accept-new-source" => accept_new_source = true,
            "--source" => {
                index += 1;
                match args.get(index) {
                    Some(path) => source = Some(PathBuf::from(path)),
                    None => {
                        eprintln!("xtask codelists: --source needs a path");
                        return ExitCode::from(2);
                    }
                }
            }
            other => {
                eprintln!("xtask codelists: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }

    let root = workspace_root();
    let source = source.unwrap_or_else(|| root.join(DEFAULT_SOURCE));
    let text = match fs::read_to_string(&source) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("xtask codelists: cannot read {}: {error}", source.display());
            eprintln!("  fetch it from {SOURCE_URL} (or run tools/fetch-codelists.sh)");
            return ExitCode::from(1);
        }
    };

    let digest = sha256_hex(text.as_bytes());
    if digest != SOURCE_SHA256 && !accept_new_source {
        eprintln!("xtask codelists: {} is not the pinned upstream file", source.display());
        eprintln!("  expected sha256 {SOURCE_SHA256}");
        eprintln!("  found    sha256 {digest}");
        eprintln!("  review the change, then bump SOURCE_SHA256 and BUNDLE, or pass --accept-new-source");
        return ExitCode::from(1);
    }

    let directory = root.join("crates").join("conformance").join("codelists").join(BUNDLE);
    let mut files = Vec::new();
    for bundle in BUNDLES {
        match extract(&text, bundle) {
            Ok(codes) => files.push((bundle, render(&codes))),
            Err(error) => {
                eprintln!("xtask codelists: {error}");
                return ExitCode::from(1);
            }
        }
    }

    let manifest = render_manifest(&digest, &files);

    if check {
        return check_bundle(&directory, &manifest, &files);
    }

    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!("xtask codelists: cannot create {}: {error}", directory.display());
        return ExitCode::from(1);
    }
    for (bundle, content) in &files {
        let path = directory.join(bundle.file);
        if let Err(error) = fs::write(&path, content) {
            eprintln!("xtask codelists: cannot write {}: {error}", path.display());
            return ExitCode::from(1);
        }
    }
    let path = directory.join("manifest.json");
    if let Err(error) = fs::write(&path, &manifest) {
        eprintln!("xtask codelists: cannot write {}: {error}", path.display());
        return ExitCode::from(1);
    }

    println!("wrote {} files to {}", files.len() + 1, directory.display());
    ExitCode::SUCCESS
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Extracts the space-delimited literals of the assertion carrying `rule`.
fn literals<'a>(source: &'a str, rule: &str) -> Result<Vec<&'a str>, String> {
    let needle = format!("id=\"{rule}\"");
    let id = source
        .find(&needle)
        .ok_or_else(|| format!("{rule}: no assertion in the source"))?;
    let assertion = source[..id]
        .rfind("<assert")
        .ok_or_else(|| format!("{rule}: no <assert> element before the id"))?;
    let element_end = source[assertion..]
        .find('>')
        .map(|offset| assertion + offset)
        .ok_or_else(|| format!("{rule}: unterminated <assert> element"))?;
    let start = source[assertion..element_end]
        .find("test=\"")
        .map(|offset| assertion + offset + "test=\"".len())
        .ok_or_else(|| format!("{rule}: assertion has no test attribute"))?;
    let end = start
        + source[start..]
            .find('"')
            .ok_or_else(|| format!("{rule}: unterminated test attribute"))?;
    Ok(source[start..end]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|literal| literal.split_whitespace().count() > 1)
        .collect())
}

fn extract<'a>(source: &'a str, bundle: &Bundle) -> Result<BTreeSet<&'a str>, String> {
    let literals = literals(source, bundle.rule)?;
    let literal = literals
        .get(bundle.literal)
        .ok_or_else(|| format!("{}: {} has no literal {}", bundle.file, bundle.rule, bundle.literal))?;
    let codes = literal.split_whitespace().collect::<BTreeSet<_>>();
    if codes.is_empty() {
        return Err(format!("{}: extracted an empty list", bundle.file));
    }
    Ok(codes)
}

fn render(codes: &BTreeSet<&str>) -> String {
    let mut text = String::with_capacity(codes.iter().map(|code| code.len() + 1).sum());
    for code in codes {
        text.push_str(code);
        text.push('\n');
    }
    text
}

fn render_manifest(source_digest: &str, files: &[(&Bundle, String)]) -> String {
    let mut entries = String::new();
    for (index, (bundle, content)) in files.iter().enumerate() {
        if index > 0 {
            entries.push_str(",\n");
        }
        entries.push_str(&format!(
            "    {{\n      \"name\": \"{}\",\n      \"codelist\": \"{}\",\n      \"rule\": \"{}\",\n      \"literal\": {},\n      \"entries\": {},\n      \"sha256\": \"{}\"\n    }}",
            bundle.file,
            bundle.codelist,
            bundle.rule,
            bundle.literal,
            content.lines().count(),
            sha256_hex(content.as_bytes()),
        ));
    }
    format!(
        "{{\n  \"bundle\": \"{BUNDLE}\",\n  \"source\": {{\n    \"url\": \"{SOURCE_URL}\",\n    \"retrieved\": \"{RETRIEVED}\",\n    \"sha256\": \"{source_digest}\"\n  }},\n  \"files\": [\n{entries}\n  ]\n}}\n"
    )
}

fn check_bundle(directory: &Path, manifest: &str, files: &[(&Bundle, String)]) -> ExitCode {
    let mut stale = Vec::new();
    for (bundle, content) in files {
        let path = directory.join(bundle.file);
        // A checkout may have given the file the platform's line endings; the
        // comparison is about the codes, not about how they were transported.
        match fs::read_to_string(&path) {
            Ok(committed) if committed.replace("\r\n", "\n") == *content => {}
            Ok(_) => stale.push(bundle.file),
            Err(_) => stale.push(bundle.file),
        }
    }
    let manifest_path = directory.join("manifest.json");
    match fs::read_to_string(&manifest_path) {
        Ok(committed) if committed.replace("\r\n", "\n") == manifest => {}
        _ => stale.push("manifest.json"),
    }

    if stale.is_empty() {
        println!("codelists: {} files match the pinned source", files.len() + 1);
        ExitCode::SUCCESS
    } else {
        for file in &stale {
            eprintln!("codelists: {file} is stale");
        }
        eprintln!("run `cargo run -p xtask -- codelists` to regenerate the bundle");
        ExitCode::from(1)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_embedded_literal_of_a_rule() {
        let source = r#"<assert
      test="((not(contains(normalize-space(.), ' ')) and contains(' AE L M E S Z G O K B ', concat(' ', normalize-space(.), ' '))))"
      id="BR-CL-17"
      flag="fatal">x</assert>"#;
        let found = literals(source, "BR-CL-17").expect("literals");
        assert_eq!(found, vec!["AE L M E S Z G O K B"]);
    }

    #[test]
    fn keeps_multiword_literals_in_order() {
        let source = r#"<assert
      test="(contains(' 71 80 ', concat(' ', normalize-space(.), ' '))) or (contains(' 81 83 ', concat(' ', normalize-space(.), ' ')))"
      id="BR-CL-01"
      flag="fatal">x</assert>"#;
        assert_eq!(literals(source, "BR-CL-01").expect("literals"), vec!["71 80", "81 83"]);
    }

    #[test]
    fn an_unknown_rule_is_an_error() {
        assert!(literals("<assert test=\"' a b '\" id=\"BR-CL-17\"/>", "BR-CL-99").is_err());
    }

    #[test]
    fn renders_codes_sorted_and_newline_terminated() {
        let codes = ["B", "A"].into_iter().collect::<BTreeSet<_>>();
        assert_eq!(render(&codes), "A\nB\n");
    }

    #[test]
    fn every_bundle_has_a_distinct_file() {
        let mut names = BTreeSet::new();
        for bundle in BUNDLES {
            assert!(names.insert(bundle.file), "duplicate {}", bundle.file);
        }
    }
}
