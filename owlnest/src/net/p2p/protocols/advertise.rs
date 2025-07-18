use super::*;
use config::Config;
pub use owlnest_advertise::*;
use owlnest_core::error::OperationError;
use std::sync::{atomic::AtomicU64, Arc};

/// A handle that can communicate with the behaviour within the swarm.
#[derive(Debug, Clone)]
pub struct Handle {
    sender: mpsc::Sender<InEvent>,
    swarm_event_source: EventSender,
    #[allow(unused)]
    counter: Arc<AtomicU64>,
}
impl Handle {
    pub(crate) fn new(
        _config: &Config,
        buffer_size: usize,
        swarm_event_source: &EventSender,
    ) -> (Self, mpsc::Receiver<InEvent>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        (
            Self {
                sender: tx,
                swarm_event_source: swarm_event_source.clone(),
                counter: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }
    /// Send query to a remote for current advertisements.
    /// Will return `Err(Error::NotProviding)` for peers who don't support this protocol.
    pub async fn query_advertised_peer(
        &self,
        relay: PeerId,
    ) -> Result<Option<Box<[PeerId]>>, Error> {
        let mut listener = self.swarm_event_source.subscribe();
        let fut = listen_event!(listener for Advertise,
            OutEvent::QueryAnswered { from, list } => {
                if *from == relay {
                    return Ok(list.clone());
                }
            }
            OutEvent::Error(Error::NotProviding(peer)) => {
                if *peer == relay{
                    return Err(Error::NotProviding(*peer))
                }
            }
        );
        let ev = InEvent::QueryAdvertisedPeer { peer: relay };
        self.sender.send(ev).await.expect("");
        match future_timeout!(fut, 10000) {
            Ok(v) => v,
            Err(_) => Err(Error::Timeout),
        }
    }
    /// Remove advertisement on local peer.
    pub async fn remove_advertised(&self, peer_id: &PeerId) -> Result<bool, OperationError> {
        let ev = InEvent::RemoveAdvertised { peer: *peer_id };
        let mut listener = self.swarm_event_source.subscribe();
        let fut = listen_event!(listener for Advertise,
            OutEvent::AdvertisedPeerChanged(target,state)=>{
                if *target == *peer_id{
                    return *state
                }
        });
        send_swarm!(self.sender, ev);
        Ok(future_timeout!(fut, 1000)?)
    }
    generate_handler_method!(
        /// List all peers that supports and connected to this peer.
        ListConnected:list_connected()->Box<[PeerId]>;
        /// List all advertisement on local peer.
        ListAdvertised:list_advertised()->Box<[PeerId]>;
        /// Get provider state of local peer.
        /// Will return a immediate state report, e.g. only changes caused by this operation.
        GetProviderState:provider_state()->bool;
    );
    generate_handler_method!(
        /// Clear all advertisements on local peer.
        ClearAdvertised:clear_advertised();
    );
    #[allow(unused)]
    fn next_id(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.counter.fetch_add(1, Ordering::SeqCst)
    }
}

impl Handle {
    generate_handler_method!(
        /// Set provider state of local peer.
        /// Will return a recent(not immediate) state change.
        SetProviderState:set_provider_state(target_state: |bool|) -> bool;
        /// Set advertisement on a remote peer.
        /// This function will return immediately, the effect is not guaranteed:
        /// - peers that are not connected
        /// - peers that don't support this protocol
        /// - peers that are not providing
        /// ## Silent failure
        SetRemoteAdvertisement:set_remote_advertisement(remote: &PeerId, state: |bool|) -> ();
    );
}

pub mod cli {

    use super::*;
    use clap::Subcommand;
    use libp2p::PeerId;
    use prettytable::table;
    use printable::iter::PrintableIter;

    /// Subcommand for managing `owlnest-advertise` protocol.  
    /// `owlnest-advertise` intends to provide a machine-operable way
    /// of advertising some information about a peer.  
    /// Currenyly, it's used to advertise peers that are listening on
    /// the relay server.
    /// Peers can post an advertisement on other peers that support this protocol,
    /// then the AD can be seen by other peers through active query.
    #[derive(Debug, Subcommand)]
    pub enum Advertise {
        /// Post an AD on or retract an AD from the remote.  
        /// This command will return immediately, and it's effect is not guaranteed.
        /// The remote can still be showing the AD.
        SetRemoteAdvertisement {
            /// Peer ID of the remote peer.
            remote: PeerId,
            /// `true` to posting an AD, `false` to retract an AD.
            state: bool,
        },
        /// Query for all ADs on the remote peer.
        QueryAdvertised {
            /// Peer ID of the remote peer.
            remote: PeerId,
        },
        /// Subcommand for managing local provider, e.g whether or not to
        /// answer query from other peers.
        #[command(subcommand)]
        Provider(provider::Provider),
    }

    pub async fn handle_advertise(handle: &Handle, command: Advertise) {
        use Advertise::*;
        match command {
            Provider(command) => provider::handle_provider(handle, command).await,
            SetRemoteAdvertisement { remote, state } => {
                handle.set_remote_advertisement(&remote, state).await;
                println!("OK")
            }
            QueryAdvertised { remote } => {
                let result = handle.query_advertised_peer(remote).await;
                match result {
                    Ok(v) => {
                        if v.is_none() {
                            return println!("Remote {remote} is not providing");
                        }
                        let list = v.expect("Already handled");
                        let table = table!(
                            [format!("Peers advertised by\n{}", remote)],
                            [list
                                .iter()
                                .printable()
                                .with_left_bound("")
                                .with_right_bound("")
                                .with_separator("\n")]
                        );
                        table.printstd();
                    }
                    Err(_) => println!(
                        "Remote {remote} is not connected or doesn't support `owlput-advertise`."
                    ),
                }
            }
        }
    }

    mod provider {
        use clap::{arg, Subcommand};

        /// Commands for managing local provider.
        #[derive(Debug, Subcommand)]
        pub enum Provider {
            /// Start the local provider, e.g begin answering queries.
            Start,
            /// Stop the local provider, e.g stop answering queries.
            Stop,
            /// Get the current state of local provider.
            State,
            /// List all advertisement on local provider.
            ListAdvertised,
            /// Remove the AD of the given peer from local provider.
            RemoveAdvertise {
                /// The peer ID to remove
                #[arg(required = true)]
                peer: PeerId,
            },
            /// Remove all ADs from local provider
            ClearAdvertised,
        }

        use super::*;
        pub async fn handle_provider(handle: &Handle, command: Provider) {
            use Provider::*;
            match command {
                Start => println!(
                    "Local provider state is set to: {}",
                    handle.set_provider_state(true).await
                ),
                Stop => println!(
                    "Local provider state is set to: {}",
                    handle.set_provider_state(false).await
                ),
                State => println!("isProviding:{}", handle.provider_state().await),
                ListAdvertised => {
                    let list = handle.list_advertised().await;
                    println!("Advertising: \n{list:?}");
                }
                RemoveAdvertise { peer } => {
                    match handle.remove_advertised(&peer).await {
                        Ok(v) => println!("Local provider state is set to: {v}"),
                        Err(e) => println!("Cannot RemoveAdvertise: {e}"),
                    }
                    println!("Advertisement for peer {peer} is removed")
                }
                ClearAdvertised => {
                    handle.clear_advertised().await;
                    println!("All ADs has been cleared.")
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{net::p2p::test_suit::setup_default, sleep};
    use anyhow::Ok;
    use libp2p::Multiaddr;
    use serial_test::serial;
    use tracing_log::log::trace;

    #[test]
    #[serial]
    fn test() -> anyhow::Result<()> {
        let (peer1_m, _) = setup_default();
        let (peer2_m, _) = setup_default();
        peer1_m
            .swarm()
            .listen_blocking(&"/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>()?)?;
        trace!("peer 1 is listening");
        sleep!(200);
        let peer1_id = peer1_m.identity().get_peer_id();
        let peer2_id = peer2_m.identity().get_peer_id();
        peer2_m
            .swarm()
            .dial_blocking(&peer1_m.swarm().list_listeners_blocking()[0])?;
        trace!("peer 1 dialed");
        sleep!(200);
        assert!(peer1_m
            .executor()
            .block_on(peer1_m.advertise().set_provider_state(true)));
        trace!("provider state set");
        sleep!(200);
        peer2_m.executor().block_on(
            peer2_m
                .advertise()
                .set_remote_advertisement(&peer1_id, true),
        );
        assert!(peer2_m.swarm().is_connected_blocking(&peer1_id));
        trace!("peer 1 connected and advertisement set");
        sleep!(200);
        assert!(peer2_m
            .executor()
            .block_on(peer2_m.advertise().query_advertised_peer(peer1_id))?
            .expect("peer to exist")
            .contains(&peer2_id));
        trace!("found advertisement for peer2 on peer1");
        assert!(!peer1_m
            .executor()
            .block_on(peer1_m.advertise().set_provider_state(false)));
        sleep!(200);
        trace!("provider state of peer1 set to false");
        assert!(
            peer2_m
                .executor()
                .block_on(peer2_m.advertise().query_advertised_peer(peer1_id))?
                == None
        );
        trace!("advertisement no longer available");
        peer2_m.executor().block_on(
            peer2_m
                .advertise()
                .set_remote_advertisement(&peer1_id, false),
        );
        trace!("removed advertisement on peer1(testing presistence)");
        assert!(peer1_m
            .executor()
            .block_on(peer1_m.advertise().set_provider_state(true)));
        sleep!(200);
        trace!("turned peer1 provider back on");
        assert!(
            peer2_m
                .executor()
                .block_on(peer2_m.advertise().query_advertised_peer(peer1_id))?
                .expect("peer to exist")
                .len()
                == 0
        );
        Ok(())
    }

    // Attach when necessary
    #[allow(unused)]
    fn setup_logging() {
        use crate::net::p2p::protocols::SUBSCRIBER_CONFLICT_ERROR_MESSAGE;
        use std::sync::Mutex;
        use tracing::Level;
        use tracing_log::LogTracer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::Layer;
        let filter = tracing_subscriber::filter::Targets::new()
            .with_target("owlnest", Level::INFO)
            .with_target("owlnest::net::p2p::protocols::advertise", Level::TRACE)
            .with_target("owlnest_advertise", Level::TRACE)
            .with_target("", Level::WARN);
        let layer = tracing_subscriber::fmt::Layer::default()
            .with_ansi(false)
            .with_writer(Mutex::new(std::io::stdout()))
            .with_filter(filter);
        let reg = tracing_subscriber::registry().with(layer);
        tracing::subscriber::set_global_default(reg).expect(SUBSCRIBER_CONFLICT_ERROR_MESSAGE);
        LogTracer::init().expect("log integration to be initialized correctly");
    }
}
