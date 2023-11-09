use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tracing::{debug, trace, warn};

mod behaviour;
mod cli;
mod config;
mod error;
mod handler;
mod message;
mod op;

pub use behaviour::Behaviour;
pub(crate) use cli::handle_messaging;
pub use config::Config;
pub use error::Error;
pub use message::Message;
pub use protocol::PROTOCOL_NAME;

#[derive(Debug)]
pub enum InEvent {
    SendMessage(PeerId, Message, u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutEvent {
    IncomingMessage { from: PeerId, msg: Message },
    SuccessfulSend(u64),
    Error(Error),
    Unsupported(PeerId),
    InboundNegotiated(PeerId),
    OutboundNegotiated(PeerId),
}
impl Listenable for OutEvent {
    fn as_event_identifier() -> String {
        format!("{}:OutEvent", PROTOCOL_NAME)
    }
}

pub async fn ev_dispatch(ev: &OutEvent, ev_tap: &EventTap) {
    use OutEvent::*;
    ev_tap
        .send(ev.clone().into_listened())
        .await
        .expect("Event sent to tap to succeed");
    match ev {
        IncomingMessage { .. } => {
            println!("Incoming message: {:?}\n", ev);
        }
        Error(e) => debug!("{:#?}", e),
        Unsupported(peer) => {
            trace!("Peer {} doesn't support /owlput/messaging/0.0.1", peer)
        }
        InboundNegotiated(peer) => trace!(
            "Successfully negotiated inbound connection from peer {}",
            peer
        ),
        OutboundNegotiated(peer) => trace!(
            "Successfully negotiated outbound connection to peer {}",
            peer
        ),
        SuccessfulSend(_) => {}
    }
}

mod protocol {
    pub const PROTOCOL_NAME: &str = "/owlnest/messaging/0.0.1";
    pub use crate::net::p2p::protocols::universal::protocol::{recv, send};
}

use tokio::sync::mpsc;

use crate::{
    event_bus::{bus::EventTap, listened_event::Listenable},
    single_value_filter, with_timeout,
};
#[derive(Debug, Clone)]
pub struct Handle {
    sender: mpsc::Sender<InEvent>,
    event_bus_handle: crate::event_bus::Handle,
    counter: Arc<AtomicU64>,
}
impl Handle {
    pub fn new(
        buffer: usize,
        event_bus_handle: &crate::event_bus::Handle,
    ) -> (Self, mpsc::Receiver<InEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (
            Self {
                sender: tx,
                event_bus_handle: event_bus_handle.clone(),
                counter: Arc::new(AtomicU64::new(1)),
            },
            rx,
        )
    }
    pub async fn send_message(&self, peer_id: PeerId, message: Message) -> Result<(), Error> {
        let op_id = self.counter.fetch_add(1, Ordering::SeqCst);
        let ev = InEvent::SendMessage(peer_id, message, op_id);
        let mut listener = self
            .event_bus_handle
            .add(OutEvent::as_event_identifier())
            .await
            .expect("listener registartion to succeed");
        self.sender.send(ev).await.expect("send to succeed");
        let fut = single_value_filter!(listener::<OutEvent>, |ev| {
            if let OutEvent::SuccessfulSend(id) = &ev {
                return *id == op_id;
            };
            if let OutEvent::Error(Error::PeerNotFound(ev_peer_id)) = &ev {
                return *ev_peer_id == peer_id;
            };
            false
        });
        match with_timeout!(fut, 10) {
            Ok(v) => {
                if let OutEvent::Error(e) = v.expect("listen to succeed") {
                    return Err(e);
                }
                Ok(())
            }
            Err(_) => {
                warn!("a timeout reached for a timed future");
                Err(Error::Timeout)
            }
        }
    }
}
