#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Developer automation: artifact fetching, conformance runs, packaging.

mod codelists;
mod conformance;

fn tasks() -> &'static [(&'static str, &'static str)] {
    &[
        ("conformance", "validate a directory with the official artefacts"),
        ("codelists", "regenerate the vendored EN 16931 code lists"),
        ("spikes", "re-run the M0 risk spikes"),
        ("package", "build signed installers (M6/M7)"),
    ]
}

fn main() -> std::process::ExitCode {
    let arguments: Vec<String> = std::env::args().collect();
    match arguments.get(1).map(String::as_str) {
        None | Some("help") | Some("--help") => {
            println!("xtask: developer automation");
            for (name, description) in tasks() {
                println!("  {name:<14} {description}");
            }
            std::process::ExitCode::SUCCESS
        }
        Some("codelists") => codelists::run(arguments.get(2..).unwrap_or_default()),
        Some("conformance") => conformance::run(arguments.get(2..).unwrap_or_default()),
        Some(other) => {
            eprintln!("xtask: unknown task {other:?}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tasks_are_listed() {
        assert!(super::tasks().iter().any(|(name, _)| *name == "conformance"));
    }
}
