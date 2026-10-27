//! Runs the official EN 16931 validator over a directory of UBL documents.
//!
//! This is the authority side of the two validation paths: the Rust rule engine
//! gives live feedback, and the official artefacts decide. The oracle needs Java
//! and the validator jar, so this task is a developer and release gate, never
//! part of the shipped binary.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use conformance_official::oracle::Oracle;

const DEFAULT_DIRECTORY: &str = "target/m1-validation/out";

/// Entry point for `xtask conformance [directory]`.
pub fn run(args: &[String]) -> ExitCode {
    let directory = match args {
        [] => PathBuf::from(DEFAULT_DIRECTORY),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("usage: cargo run -p xtask -- conformance [directory]");
            println!("  validates every .xml in the directory with the official artefacts");
            println!("  requires MUSTANG_JAR (and JAVA, default `java`)");
            return ExitCode::SUCCESS;
        }
        [path] => PathBuf::from(path),
        _ => {
            eprintln!("xtask conformance: at most one directory argument");
            return ExitCode::from(2);
        }
    };

    let files = match xml_files(&directory) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("xtask conformance: cannot read {}: {error}", directory.display());
            return ExitCode::from(2);
        }
    };
    if files.is_empty() {
        eprintln!(
            "xtask conformance: no .xml documents in {} (run tools/validate-m1.sh first)",
            directory.display()
        );
        return ExitCode::from(2);
    }

    let oracle = match Oracle::from_environment() {
        Ok(oracle) => oracle,
        Err(error) => {
            eprintln!("xtask conformance: {error}");
            eprintln!("  set MUSTANG_JAR to the validator jar; see tools/validate-m1.sh");
            return ExitCode::from(2);
        }
    };

    let mut valid = 0;
    let mut rejected = Vec::new();
    let mut version = None;

    for file in &files {
        match oracle.validate(file) {
            Ok(report) => {
                if version.is_none() {
                    version = report.validator_version.clone();
                }
                if report.valid {
                    valid += 1;
                    println!("ok   {} ({} rules fired)", name(file), report.fired);
                } else {
                    println!("FAIL {} -> {}", name(file), report.failed_codes().join(", "));
                    rejected.push((file.clone(), report));
                }
            }
            Err(error) => {
                eprintln!("xtask conformance: {} could not be validated: {error}", name(file));
                return ExitCode::from(1);
            }
        }
    }

    for (file, report) in &rejected {
        for code in report.failed_codes() {
            let mut matches = report.failures.iter().filter(|failure| failure.code == code);
            let Some(first) = matches.next() else {
                continue;
            };
            let count = 1 + matches.count();
            if count > 1 {
                println!("  {} {code} ({count}x): {}", name(file), first.message);
            } else {
                println!("  {} {code}: {}", name(file), first.message);
            }
        }
    }

    let validator = version.unwrap_or_else(|| "unknown validator".to_owned());
    if rejected.is_empty() {
        println!(
            "official: {valid}/{} documents valid ({validator}, EN 16931 + Peppol BIS 3.0)",
            files.len()
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("official: {valid}/{} documents valid ({validator})", files.len());
        ExitCode::from(1)
    }
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

fn name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn only_xml_documents_are_collected_and_sorted() {
        let directory = std::env::temp_dir().join(format!("bonk-conformance-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("b.xml"), "<x/>").unwrap();
        fs::write(directory.join("a.xml"), "<x/>").unwrap();
        fs::write(directory.join("notes.txt"), "ignore").unwrap();

        let names: Vec<String> = xml_files(&directory).unwrap().iter().map(|path| name(path)).collect();
        assert_eq!(names, vec!["a.xml", "b.xml"]);

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_empty_directory_is_not_a_success() {
        let directory = std::env::temp_dir().join(format!("bonk-conformance-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        assert_eq!(xml_files(&directory).unwrap().len(), 0);
        fs::remove_dir_all(&directory).unwrap();
    }
}
