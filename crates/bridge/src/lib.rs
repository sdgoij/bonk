#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Connector ports, outbox dispatch, plugin host, sidecar protocol.
//!
//! M7 implements the Peppol adapter. The port below is the contract every
//! integration must satisfy: idempotent sends, explicit capabilities, and no
//! implicit network access.

use crux::DocumentId;

/// What a connector can do. The UI renders permissions from this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    /// Can deliver an issued document.
    pub send: bool,
    /// Can poll for inbound documents.
    pub poll_inbound: bool,
    /// Can look up a participant or identifier.
    pub lookup: bool,
}

impl Capabilities {
    /// Nothing.
    pub const NONE: Self = Self {
        send: false,
        poll_inbound: false,
        lookup: false,
    };
    /// Outbound delivery only.
    pub const SEND_ONLY: Self = Self {
        send: true,
        poll_inbound: false,
        lookup: false,
    };
}

/// A connector's self-description, shown before it is enabled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// Stable identifier, e.g. `peppol-provider-x`.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// What it is allowed to do.
    pub capabilities: Capabilities,
    /// Hosts it needs to reach; empty means no network.
    pub egress_hosts: Vec<String>,
}

/// Idempotency key for a send: same document and connector, same key.
pub fn send_key(connector_id: &str, document: DocumentId, content_hash: &str) -> String {
    format!("{connector_id}:{document}:{content_hash}")
}

/// Errors a connector may return, classified by what the user should do.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ConnectorError {
    /// Try again later.
    #[error("temporary failure: {0}")]
    Retryable(String),
    /// The peer rejected the document; the code is theirs.
    #[error("rejected by peer ({code}): {message}")]
    RejectedByPeer {
        /// Provider error code.
        code: String,
        /// Provider message.
        message: String,
    },
    /// Bad configuration (missing credentials, wrong endpoint).
    #[error("configuration error: {0}")]
    Configuration(String),
    /// Credentials rejected.
    #[error("authentication failed")]
    Auth,
    /// Not implemented in this build.
    #[error("{0} arrives in {1}")]
    NotYet(&'static str, &'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_keys_are_stable_and_specific() {
        let document = DocumentId::new();
        let first = send_key("peppol-x", document, "abc");
        let second = send_key("peppol-x", document, "abc");
        let other_connector = send_key("peppol-y", document, "abc");
        assert_eq!(first, second);
        assert_ne!(first, other_connector);
    }

    #[test]
    fn capabilities_are_explicit() {
        let none = Capabilities {
            send: false,
            poll_inbound: false,
            lookup: false,
        };
        let send_only = Capabilities {
            send: true,
            poll_inbound: false,
            lookup: false,
        };
        assert!(!none.send);
        assert!(send_only.send);
        assert!(!send_only.poll_inbound);
    }
}
