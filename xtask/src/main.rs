#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Developer automation: artifact fetching, conformance runs, packaging.

fn tasks() -> &'static [(&'static str, &'static str)] {
    &[
        ("conformance", "run the conformance corpus (M2)"),
        ("spikes", "re-run the M0 risk spikes"),
        ("package", "build signed installers (M6/M7)"),
    ]
}

fn main() {
    let argument = std::env::args().nth(1);
    match argument.as_deref() {
        None | Some("help") | Some("--help") => {
            println!("xtask: developer automation");
            for (name, description) in tasks() {
                println!("  {name:<14} {description}");
            }
        }
        Some(other) => {
            eprintln!("xtask: unknown task {other:?}");
            std::process::exit(2);
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
