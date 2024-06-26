use crate::net::p2p::swarm::EventSender;
use libp2p::PeerId;
use owlnest_macro::{generate_handler_method, listen_event, with_timeout};
pub use owlnest_messaging::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct Handle {
    sender: mpsc::Sender<InEvent>,
    event_tx: EventSender,
    counter: Arc<AtomicU64>,
}
impl Handle {
    pub fn new(buffer: usize, event_tx: &EventSender) -> (Self, mpsc::Receiver<InEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (
            Self {
                sender: tx,
                event_tx: event_tx.clone(),
                counter: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }
    pub async fn send_message(
        &self,
        peer_id: PeerId,
        message: Message,
    ) -> Result<Duration, error::SendError> {
        let op_id = self.next_id();
        let ev = InEvent::SendMessage(peer_id, message, op_id);
        let mut listener = self.event_tx.subscribe();
        let fut = listen_event!(listener for Messaging, OutEvent::SendResult(result, id)=>{
            if *id != op_id {
                continue;
            }
            return result.clone();
        });
        self.sender.send(ev).await.expect("send to succeed");
        match with_timeout!(fut, 10) {
            Ok(v) => v,
            Err(_) => {
                warn!("timeout reached for a timed future");
                Err(error::SendError::Timeout)
            }
        }
    }
    generate_handler_method!(ListConnected:list_connected()->Box<[PeerId]>;);
    fn next_id(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}

pub(crate) mod cli {
    use clap::Subcommand;
    use libp2p::PeerId;

    use super::{Handle, Message, OutEvent};
    use crate::net::p2p::identity::IdentityUnion;
    use crate::net::p2p::swarm;
    use crate::net::p2p::swarm::behaviour::BehaviourEvent;
    use crate::net::p2p::swarm::manager::Manager;

    /// Subcommand for interacting with `owlnest-messaging` protocol.  
    /// You can use this protocol to send real-time text messages(IM)
    /// to another peer that supports this protocol and you have a connection to.
    #[derive(Debug, Subcommand)]
    pub enum Messaging {
        /// Send the text message to the given peer.
        Send {
            /// The peer to send the message to.
            #[arg(required = true)]
            peer_id: PeerId,
            /// Your text message to send.
            #[arg(required = true)]
            message: String,
        },
    }

    pub fn setup(manager: &Manager) {
        let mut listener = manager.event_subscriber().subscribe();
        manager.executor().spawn(async move {
            while let Ok(ev) = listener.recv().await {
                if let swarm::SwarmEvent::Behaviour(BehaviourEvent::Messaging(
                    OutEvent::IncomingMessage { from, msg },
                )) = ev.as_ref()
                {
                    println!("Incoming message from {}: {}", from, msg.msg)
                }
            }
        });
    }

    pub async fn handle_messaging(handle: &Handle, ident: &IdentityUnion, command: Messaging) {
        use Messaging::*;
        match command {
            Send { peer_id, message } => {
                let msg = Message::new(ident.get_peer_id(), peer_id, message);
                let result = handle.send_message(peer_id, msg).await;
                match result {
                    Ok(_) => println!("Message has been successfully sent"),
                    Err(e) => println!("Error occurred when sending message: {}", e),
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::net::p2p::{swarm::Manager, test_suit::setup_default};
    use libp2p::Multiaddr;
    use std::{io::stdout, thread};

    #[test]
    fn test_sigle_send_recv() {
        setup_logging();
        let (peer1, _) = setup_default();
        let (peer2, _) = setup_default();
        peer1
            .swarm()
            .listen_blocking(&"/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>().unwrap())
            .unwrap();
        let mut peer1_message_watcher = spawn_watcher(&peer1);
        let mut peer2_message_watcher = spawn_watcher(&peer2);
        thread::sleep(Duration::from_millis(100));
        peer2
            .swarm()
            .dial_blocking(&peer1.swarm().list_listeners_blocking()[0])
            .unwrap();
        let peer1_id = peer1.identity().get_peer_id();
        let peer2_id = peer2.identity().get_peer_id();
        thread::sleep(Duration::from_millis(1000));
        assert!(
            peer2.swarm().is_connected_blocking(peer1_id)
                && peer1.swarm().is_connected_blocking(peer2_id)
        );
        single_send_recv(&peer1, &peer2, &mut peer2_message_watcher);
        single_send_recv(&peer2, &peer1, &mut peer1_message_watcher);
        thread::sleep(Duration::from_millis(500));
    }

    fn eq_message(lhs: &Message, rhs: &Message) -> bool {
        lhs.from == rhs.from && lhs.to == rhs.to && lhs.msg == rhs.msg
    }
    fn spawn_watcher(manager: &Manager) -> mpsc::Receiver<(PeerId, Message)> {
        manager.executor().block_on(async {
            let mut listener = manager.event_subscriber().subscribe();
            let (tx, rx) = mpsc::channel(8);

            tokio::spawn(
                listen_event!(listener for Messaging, OutEvent::IncomingMessage { from, msg }=>{
                    tx.send((*from, msg.clone())).await.unwrap();
                }),
            );
            rx
        })
    }
    fn single_send_recv(
        from: &Manager,
        to: &Manager,
        watcher: &mut mpsc::Receiver<(PeerId, Message)>,
    ) {
        let from_peer_id = from.identity().get_peer_id();
        let to_peer_id = to.identity().get_peer_id();
        from.executor()
            .block_on(from.messaging().send_message(
                to_peer_id,
                Message::new(from_peer_id, to_peer_id, "Test MESSAGE 测试信息。"),
            ))
            .unwrap();
        let message_received = watcher.blocking_recv().unwrap();
        assert!(
            message_received.0 == from_peer_id
                && eq_message(
                    &message_received.1,
                    &Message::new(from_peer_id, to_peer_id, "Test MESSAGE 测试信息。")
                )
        );
    }

    #[allow(unused)]
    fn setup_logging() {
        use std::sync::Mutex;
        use tracing::Level;
        use tracing_log::LogTracer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::Layer;
        let filter = tracing_subscriber::filter::Targets::new()
            .with_target("owlnest_messaging", Level::TRACE)
            .with_target("owlnest_blob", Level::INFO)
            .with_target("owlnest::net::p2p::swarm", Level::INFO)
            .with_target("owlnest", Level::TRACE)
            .with_target("multistream_select", Level::WARN)
            .with_target("libp2p_core::transport::choice", Level::WARN)
            .with_target("", Level::DEBUG);
        let layer = tracing_subscriber::fmt::Layer::default()
            .with_ansi(false)
            .with_writer(Mutex::new(stdout()))
            .with_filter(filter);
        let reg = tracing_subscriber::registry().with(layer);
        tracing::subscriber::set_global_default(reg).expect("you can only set global default once");
        LogTracer::init().unwrap()
    }
}
