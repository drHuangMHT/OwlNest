use error::SendError;
use owlnest_core::alias::Callback;
use owlnest_prelude::lib_prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::trace;

mod behaviour;
mod config;
pub mod error;
mod handler;
pub mod message;
mod op;

pub use behaviour::Behaviour;
pub use config::Config;
pub use error::Error;
pub use message::Message;
pub use protocol::PROTOCOL_NAME;

#[derive(Debug)]
pub enum InEvent {
    SendMessage {
        peer: PeerId,
        message: Message,
        callback: Callback<Result<Duration, SendError>>,
    },
    ListConnected {
        callback: Callback<Box<[PeerId]>>,
    },
}

#[derive(Debug)]
pub enum OutEvent {
    IncomingMessage { from: PeerId, msg: Message },
    Error(Error),
    InboundNegotiated(PeerId),
    OutboundNegotiated(PeerId),
    Unsupported(PeerId),
}

mod protocol {
    pub const PROTOCOL_NAME: &str = "/owlnest/messaging/0.0.1";
    pub use owlnest_prelude::utils::protocol::universal::*;
}
