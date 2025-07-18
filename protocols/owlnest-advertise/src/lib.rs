use owlnest_core::alias::Callback;
use owlnest_prelude::lib_prelude::*;
use serde::{Deserialize, Serialize};

pub mod behaviour;
pub mod config;
mod handler;

pub use behaviour::Behaviour;
pub use protocol::PROTOCOL_NAME;

#[derive(Debug, Clone)]
pub enum OutEvent {
    /// A query sent to a remote peer is answered.
    QueryAnswered {
        from: PeerId,
        list: Option<Box<[PeerId]>>,
    },
    /// A advertisement result from remote peer arrived.
    RemoteAdvertisementResult {
        from: PeerId,
        result: Result<(), ()>,
    },
    /// Local provider state.
    ProviderState(bool),
    AdvertisedPeerChanged(PeerId, bool),
    Error(Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Error {
    ConnectionClosed,
    VerifierMismatch,
    /// Queried peer is not providing or doesn't support this protocol.
    NotProviding(PeerId),
    Timeout,
    UnrecognizedMessage(String), // Serialzied not available on the original type
    IO(String),                  // Serialize not available on the original type
    Channel,
}
impl std::error::Error for Error {}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Error::*;
        match self {
            ConnectionClosed => f.write_str("Connection Closed"),
            VerifierMismatch => f.write_str("Message verifier mismatch"),
            Timeout => f.write_str("Message timed out"),
            NotProviding(peer) => write!(f, "Peer {peer} is not providing"),
            UnrecognizedMessage(msg) => f.write_str(msg),
            IO(msg) => f.write_str(msg),
            Channel => f.write_str("Callback channel closed unexpectedly"),
        }
    }
}

mod protocol {
    pub const PROTOCOL_NAME: &str = "/owlnest/advertise/0.0.1";
    pub use owlnest_prelude::utils::protocol::universal::*;
}

#[derive(Debug)]
pub enum InEvent {
    /// Set local provider state.
    SetProviderState {
        target_state: bool,
        callback: Callback<bool>,
    },
    /// Get local provider state.
    GetProviderState {
        callback: Callback<bool>,
    },
    /// Send a query to a remote peer for advertised peers.
    QueryAdvertisedPeer {
        peer: PeerId,
    },
    /// Set remote provider state to advertise or stop advertising local peer.
    SetRemoteAdvertisement {
        remote: PeerId,
        state: bool,
        callback: Callback<()>,
    },
    /// Remove a advertised peer from local provider.
    RemoveAdvertised {
        peer: PeerId,
    },
    /// Remove all advertised peers from local provider.
    ClearAdvertised {},
    ListAdvertised {
        callback: Callback<Box<[PeerId]>>,
    },
    ListConnected {
        callback: Callback<Box<[PeerId]>>,
    },
}
