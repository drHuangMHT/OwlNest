use crate::net::p2p::swarm::manager::HandleBundle;
use futures::StreamExt;
use libp2p::PeerId;
use owlnest_core::alias::Callback;
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, sync::Arc};
use tokio::select;
use tracing::{trace, trace_span, warn};

/// Code used to compose the behaviour used in swarm.
#[allow(missing_docs)]
pub mod behaviour;

/// Adapter for the internal command line interface.
pub mod cli;

mod event_handlers;

/// Handle for the swarm itself.  
/// Doesn't include handles for the behaviours inside of the swarm.
pub mod handle;

/// Code used to compose the swarm manager.
pub mod manager;

/// Events that can be emitted by the swarm.
pub mod out_event;

pub use libp2p::core::ConnectedPoint;
pub use libp2p::swarm::ConnectionId;
pub use manager::Manager;

/// A broadcast sender that can be used to tap into swarm events.
pub type EventSender = tokio::sync::broadcast::Sender<Arc<SwarmEvent>>;

use super::{identity::IdentityUnion, SwarmConfig};
pub use behaviour::BehaviourEvent;
use event_handlers::*;

/// The libp2p swarm with generics filled with the behaviour.
pub type Swarm = libp2p::Swarm<behaviour::Behaviour>;
/// Events emitted by swarm with generics filled with the behaviour.
pub type SwarmEvent = libp2p::swarm::SwarmEvent<BehaviourEvent>;

/// Config that is used to *setup* the swarm.  
/// Don't confuse this with `libp2p::swarm::Config`, this doesn't contain
/// configurations for the swarm itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Path to identity file.  
    /// Will generate random identity if left blank.
    /// Will create the file if it doesn't exist.
    pub identity_path: String,
    /// Buffer size for the inner swarm event queue waiting to be processed.  
    /// Swarm events need to be consumed for the swarm to make progress,
    /// setting this value too low may result in low throughput.
    pub swarm_event_buffer_size: usize,
    /// When swarm event buffer is almost full,
    /// the swarm won't be polled(backpressure).
    /// This timeout(in milliseconds) make sure that the swarm will
    /// be polled again once buffer cleared.
    pub swarm_event_timeout: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            identity_path: String::new(),
            swarm_event_buffer_size: 16,
            swarm_event_timeout: 200,
        }
    }
}

/// Builder for the swarm.
#[derive(Default)]
pub struct Builder {
    config: SwarmConfig,
}
impl Builder {
    /// Create a new builder with the given `SwarmConfig`.
    pub fn new(config: SwarmConfig) -> Self {
        Self { config }
    }
    /// Build the swarm and expose the manager to the swarm.  
    /// You should use different executor for different swarms.
    pub fn build(self, ident: IdentityUnion, executor: tokio::runtime::Handle) -> Manager {
        let span = trace_span!("Swarm Spawn");
        let entered = span.enter();
        trace!("Building swarm");
        let guard = executor.enter();
        use crate::net::p2p::protocols::*;
        #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-kad"))]
        let kad_store = libp2p::kad::store::MemoryStore::new(ident.get_peer_id());
        let (swarm_event_out, _) =
            tokio::sync::broadcast::channel(self.config.swarm.swarm_event_buffer_size);
        let (handle_bundle, mut rx_bundle) = HandleBundle::new(&self.config, &swarm_event_out);
        let manager = manager::Manager::new(
            Arc::new(handle_bundle),
            ident.clone(),
            executor.clone(),
            swarm_event_out.clone(),
        );
        let manager_clone = manager.clone();
        drop(entered);
        tokio::spawn(async move {
            let span = span;
            let entered = span.enter();
            trace!("Swarm task spawned");
            let event_out = swarm_event_out;
            let _manager = manager_clone;
            let mut swarm = libp2p::SwarmBuilder::with_existing_identity(ident.get_keypair())
                .with_tokio()
                .with_tcp(
                    Default::default(),
                    libp2p::noise::Config::new,
                    libp2p::yamux::Config::default,
                )
                .expect("transport upgrade to succeed")
                .with_quic()
                .with_dns()
                .expect("upgrade to succeed")
                .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)
                .expect("transport upgrade to succeed")
                .with_behaviour(|_key, #[allow(unused)] relay| behaviour::Behaviour {
                    #[cfg(any(feature = "owlnest-protocols", feature = "owlnest-blob"))]
                    blob: blob::Behaviour::new(self.config.blob),
                    #[cfg(any(feature = "owlnest-protocols", feature = "owlnest-advertise"))]
                    advertise: advertise::Behaviour::new(self.config.advertise),
                    #[cfg(any(feature = "owlnest-protocols", feature = "owlnest-messaging"))]
                    messaging: messaging::Behaviour::new(self.config.messaging),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-kad"))]
                    kad: kad::Behaviour::with_config(
                        ident.get_peer_id(),
                        kad_store,
                        self.config.kad.into_config("/ipfs/kad/1.0.0".into()),
                    ),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-mdns"))]
                    mdns: mdns::Behaviour::new(self.config.mdns.into(), ident.get_peer_id())
                        .unwrap(),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-identify"))]
                    identify: identify::Behaviour::new(self.config.identify.into_config(&ident)),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-relay-server"))]
                    relay_server: libp2p::relay::Behaviour::new(
                        ident.get_peer_id(),
                        self.config.relay_server.into(),
                    ),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-relay-client"))]
                    relay_client: relay,
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-dcutr"))]
                    dcutr: dcutr::Behaviour::new(ident.get_peer_id()),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-autonat"))]
                    autonat: autonat::Behaviour::new(ident.get_peer_id(), Default::default()),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-upnp"))]
                    upnp: upnp::Behaviour::default(),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-ping"))]
                    ping: ping::Behaviour::new(Default::default()),
                    #[cfg(any(feature = "libp2p-protocols", feature = "libp2p-gossipsub"))]
                    gossipsub: gossipsub::Behaviour::new(
                        gossipsub::MessageAuthenticity::Signed(ident.get_keypair()),
                        Default::default(),
                    )
                    .unwrap(),
                    // hyper:hyper::Behaviour::new(Default::default())
                })
                .expect("behaviour incorporation to succeed")
                .build();
            trace!("Starting swarm event loop");
            drop(entered);
            drop(span);
            let swarm_event_buffer_upper_bound =
                (self.config.swarm.swarm_event_buffer_size >> 2) << 2;
            let swarm_event_buffer_high_mark = self.config.swarm.swarm_event_buffer_size / 2;
            loop {
                trace!("Swarm event loop entered");
                let timer = futures_timer::Delay::new(std::time::Duration::from_millis(
                    self.config.swarm.swarm_event_timeout,
                ));
                select! {
                    Some(ev) = rx_bundle.next() => {
                        trace!("Received incoming event {:?}",ev);
                        handle_incoming_event(ev, &mut swarm)
                    },
                    out_event = swarm.select_next_some(), if event_out.len() < swarm_event_buffer_upper_bound => {
                        trace!("Swarm generated an event {:?}",out_event);
                        handle_swarm_event(&out_event,&mut swarm).await;
                        let _ = event_out.send(Arc::new(out_event));
                    }
                    _ = timer =>{
                        trace!("timer polled, queue length {}", event_out.len());
                        if event_out.len() > swarm_event_buffer_high_mark {
                            warn!("Slow receiver for swarm events detected.")
                        }
                    }
                };
            }
        });
        drop(guard);
        manager
    }
}

use libp2p::swarm::{derive_prelude::ListenerId, DialError};
use libp2p::{Multiaddr, TransportError};

#[derive(Debug)]
pub(crate) enum InEvent {
    Dial {
        address: Multiaddr,
        callback: Callback<Result<(), DialError>>,
    },
    Listen {
        address: Multiaddr,
        callback: Callback<Result<ListenerId, TransportError<std::io::Error>>>,
    },
    ListListeners {
        callback: Callback<Box<[Multiaddr]>>,
    },
    RemoveListeners {
        listener_id: ListenerId,
        callback: Callback<bool>,
    },
    AddExternalAddress {
        address: Multiaddr,
        callback: Callback<()>,
    },
    RemoveExternalAddress {
        address: Multiaddr,
        callback: Callback<()>,
    },

    ListExternalAddresses {
        callback: Callback<Box<[Multiaddr]>>,
    },
    ListConnected {
        callback: Callback<Box<[PeerId]>>,
    },
    IsConnectedToPeerId {
        peer_id: PeerId,
        callback: Callback<bool>,
    },
    DisconnectFromPeerId {
        peer_id: PeerId,
        callback: Callback<Result<(), ()>>,
    },
}
