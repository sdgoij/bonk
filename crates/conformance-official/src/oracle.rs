//! Runs the official validator as an external process.
//!
//! The published EN 16931 validation artefacts are XSLT 2.0 (`xsl:function`,
//! `for-each-group`); libxslt implements XSLT 1.0 only, and there is no XSLT 2.0
//! engine we can embed. The oracle therefore runs a Java validator that carries
//! the artefacts, exactly as CI does. It is a developer and release tool, not
//! part of the shipped product, which is why it sits behind a feature.
//!
//! The validator never sees anything but a document the user asked to check.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::report::{Report, ReportError, parse_report};

/// Environment variable holding the validator jar path.
pub const JAR_VARIABLE: &str = "MUSTANG_JAR";

/// Environment variable holding the Java executable (defaults to `java`).
pub const JAVA_VARIABLE: &str = "JAVA";

/// A configured external validator.
pub struct Oracle {
    java: PathBuf,
    jar: PathBuf,
}

/// Why the oracle could not run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OracleError {
    /// A required environment variable is missing.
    MissingEnvironment(&'static str),
    /// The process could not be started or its output could not be read.
    Spawn(String),
    /// The validator produced no report.
    NoReport,
    /// The report could not be parsed.
    Report(ReportError),
}

impl std::fmt::Display for OracleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnvironment(name) => write!(f, "{name} is not set"),
            Self::Spawn(detail) => write!(f, "the validator could not be started: {detail}"),
            Self::NoReport => f.write_str("the validator produced no report"),
            Self::Report(error) => write!(f, "the validator report is unreadable: {error}"),
        }
    }
}

impl std::error::Error for OracleError {}

impl Oracle {
    /// Reads the validator configuration from the environment.
    pub fn from_environment() -> Result<Self, OracleError> {
        let jar = std::env::var_os(JAR_VARIABLE)
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .ok_or(OracleError::MissingEnvironment(JAR_VARIABLE))?;
        let java = std::env::var_os(JAVA_VARIABLE)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("java"));
        Ok(Self { java, jar })
    }

    /// Builds an oracle from explicit paths.
    pub fn new(java: impl Into<PathBuf>, jar: impl Into<PathBuf>) -> Self {
        Self {
            java: java.into(),
            jar: jar.into(),
        }
    }

    /// Validates one UBL document.
    pub fn validate(&self, source: &Path) -> Result<Report, OracleError> {
        let output = Command::new(&self.java)
            .arg("-jar")
            .arg(&self.jar)
            .args(["--action", "validate", "--source"])
            .arg(source)
            .args(["--no-notices", "--disable-file-logging"])
            .output()
            .map_err(|error| OracleError::Spawn(error.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let xml = stdout
            .find("<?xml")
            .map(|at| &stdout[at..])
            .ok_or(OracleError::NoReport)?;
        parse_report(xml).map_err(OracleError::Report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_jar_is_reported_by_name() {
        // `from_environment` must name the variable rather than panic; the test
        // only asserts the error shape when the variable is absent.
        if std::env::var_os(JAR_VARIABLE).is_none() {
            assert_eq!(
                Oracle::from_environment().err(),
                Some(OracleError::MissingEnvironment(JAR_VARIABLE))
            );
        }
    }

    #[test]
    fn the_oracle_can_be_built_from_explicit_paths() {
        let oracle = Oracle::new("java", "/tmp/validator.jar");
        assert_eq!(oracle.jar, PathBuf::from("/tmp/validator.jar"));
    }
}
