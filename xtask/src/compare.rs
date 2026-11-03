//! Compares our rule engine with the official validator on the same documents.
//!
//! Two validators read one document and report rule identifiers. Where they
//! disagree, the direction matters:
//!
//! * a rule only the official artefacts report is a **gap**: our engine does not
//!   implement it, so the live feedback is quieter than the authority. Expected —
//!   the engine deliberately covers the high-frequency rules first — but it has
//!   to be visible, or "we check before export" becomes a claim nobody tests;
//! * a rule only our engine reports is a **false positive**: we would block an
//!   invoice the authority accepts. That is the failure this task exits on.
//!
//! The document is read through the same reader the import path uses, so the
//! comparison covers the whole pipeline a received invoice takes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use conformance::{ImportedSemantics, rule::rule, validate};
use conformance_official::oracle::Oracle;

const DEFAULT_DIRECTORY: &str = "target/m1-validation/out";

/// What the two engines make of one document.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Outcome {
    /// Rules both report.
    pub agree: Vec<String>,
    /// Rules only the official validator reports.
    pub gaps: Vec<String>,
    /// Rules only our engine reports.
    pub extras: Vec<String>,
}

impl Outcome {
    /// True when our findings are a subset of the authority's.
    pub fn within_authority(&self) -> bool {
        self.extras.is_empty()
    }
}

/// Classifies two rule sets.
///
/// Both sides are deduplicated and sorted, because a rule can fire at several
/// places in one document and the comparison is about which rules disagree.
pub fn compare(ours: &[String], theirs: &[String]) -> Outcome {
    let ours: BTreeSet<&str> = ours.iter().map(String::as_str).collect();
    let theirs: BTreeSet<&str> = theirs.iter().map(String::as_str).collect();
    Outcome {
        agree: ours.intersection(&theirs).map(|code| (*code).to_owned()).collect(),
        gaps: theirs.difference(&ours).map(|code| (*code).to_owned()).collect(),
        extras: ours.difference(&theirs).map(|code| (*code).to_owned()).collect(),
    }
}

/// Entry point for `xtask compare [directory]`.
pub fn run(args: &[String]) -> ExitCode {
    let directory = match args {
        [] => PathBuf::from(DEFAULT_DIRECTORY),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("usage: cargo run -p xtask -- compare [directory]");
            println!("  runs our rule engine and the official validator over every .xml");
            println!("  exits non-zero when our engine reports a rule the authority does not");
            println!("  requires MUSTANG_JAR (and JAVA, default `java`)");
            return ExitCode::SUCCESS;
        }
        [path] => PathBuf::from(path),
        _ => {
            eprintln!("xtask compare: at most one directory argument");
            return ExitCode::from(2);
        }
    };

    let files = match xml_files(&directory) {
        Ok(files) if !files.is_empty() => files,
        Ok(_) => {
            eprintln!(
                "xtask compare: no .xml documents in {} (run tools/validate-m1.sh first)",
                directory.display()
            );
            return ExitCode::from(2);
        }
        Err(error) => {
            eprintln!("xtask compare: cannot read {}: {error}", directory.display());
            return ExitCode::from(2);
        }
    };

    let oracle = match Oracle::from_environment() {
        Ok(oracle) => oracle,
        Err(error) => {
            eprintln!("xtask compare: {error}");
            eprintln!("  set MUSTANG_JAR to the validator jar; see tools/validate-m1.sh");
            return ExitCode::from(2);
        }
    };

    let mut agreed = 0_usize;
    let mut gaps = 0_usize;
    let mut extras = 0_usize;
    let mut unreadable = 0_usize;
    let mut failure = false;

    for file in &files {
        let name = name(file);

        let xml = match std::fs::read_to_string(file) {
            Ok(xml) => xml,
            Err(error) => {
                eprintln!("unreadable {name}: {error}");
                unreadable += 1;
                failure = true;
                continue;
            }
        };
        let invoice = match syntax::read::read_invoice(&xml) {
            Ok(invoice) => invoice,
            Err(error) => {
                eprintln!("unreadable {name}: our reader refused it: {error}");
                unreadable += 1;
                failure = true;
                continue;
            }
        };
        let semantics = ImportedSemantics::new(&invoice);
        let ours: Vec<String> = validate(&semantics.view(&invoice))
            .into_iter()
            .map(|finding| finding.code.to_owned())
            .collect();

        let report = match oracle.validate(file) {
            Ok(report) => report,
            Err(error) => {
                eprintln!("xtask compare: {name} could not be validated: {error}");
                return ExitCode::from(1);
            }
        };
        let theirs = report.failed_codes();

        let outcome = compare(&ours, &theirs);
        if !outcome.within_authority() {
            extras += outcome.extras.len();
            failure = true;
            println!(
                "extra {name}: {} (the authority accepts this)",
                describe(&outcome.extras)
            );
        }
        if !outcome.gaps.is_empty() {
            gaps += outcome.gaps.len();
            println!("gap   {name}: {}", describe(&outcome.gaps));
        }
        if outcome.extras.is_empty() && outcome.gaps.is_empty() {
            agreed += 1;
            let said = if outcome.agree.is_empty() {
                "both clean".to_owned()
            } else {
                describe(&outcome.agree)
            };
            println!("agree {name}: {said}");
        }
    }

    println!(
        "engine: {agreed}/{} documents agree, {gaps} gap(s) where the authority is stricter, {extras} finding(s) the authority does not share{}",
        files.len(),
        if unreadable == 0 {
            String::new()
        } else {
            format!(", {unreadable} unreadable")
        }
    );

    if failure { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// One code with the rule's own summary, so the output says what was missed
/// rather than only which identifier was missed.
fn describe(codes: &[String]) -> String {
    let mut parts = Vec::with_capacity(codes.len());
    for code in codes {
        match rule(code) {
            Some(rule) => parts.push(format!("{code} ({})", rule.summary)),
            None => parts.push(code.clone()),
        }
    }
    parts.join("; ")
}

fn name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn xml_files(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "xml") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn two_clean_verdicts_agree() {
        let outcome = compare(&[], &[]);
        assert!(outcome.agree.is_empty());
        assert!(outcome.within_authority());
    }

    #[test]
    fn a_rule_only_the_authority_finds_is_a_gap() {
        let outcome = compare(&codes(&["BR-CO-14"]), &codes(&["BR-CO-14", "BR-S-08"]));
        assert_eq!(outcome.agree, ["BR-CO-14"]);
        assert_eq!(outcome.gaps, ["BR-S-08"]);
        assert!(outcome.extras.is_empty());
        assert!(outcome.within_authority(), "a gap is work, not a defect");
    }

    #[test]
    fn a_rule_only_we_find_is_a_false_positive() {
        let outcome = compare(&codes(&["BR-CL-04"]), &[]);
        assert_eq!(outcome.extras, ["BR-CL-04"]);
        assert!(!outcome.within_authority());
    }

    #[test]
    fn a_repeated_rule_is_one_disagreement() {
        let outcome = compare(
            &codes(&["BR-S-08", "BR-S-08"]),
            &codes(&["BR-S-08", "BR-CO-13", "BR-CO-13"]),
        );
        assert_eq!(outcome.agree, ["BR-S-08"]);
        assert_eq!(outcome.gaps, ["BR-CO-13"]);
        assert!(outcome.extras.is_empty());
    }
}
