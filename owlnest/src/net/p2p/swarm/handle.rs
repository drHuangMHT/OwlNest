use crate::net::p2p::swarm::InEvent;
use libp2p::{
    swarm::{derive_prelude::ListenerId, DialError},
    Multiaddr, PeerId, TransportError,
};
use owlnest_macro::{generate_handler_method, generate_handler_method_blocking};
use tokio::sync::{mpsc, oneshot::*};

#[derive(Debug, Clone)]
pub struct SwarmHandle {
    sender: mpsc::Sender<InEvent>,
}
impl SwarmHandle {
    pub fn new(buffer: usize) -> (Self, mpsc::Receiver<InEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { sender: tx }, rx)
    }
    pub fn dial_blocking(&self, addr: &Multiaddr) -> Result<(), DialError> {
        let (tx, rx) = channel();
        let ev = InEvent::Dial(addr.clone(), tx);
        self.sender.blocking_send(ev).unwrap();
        rx.blocking_recv().unwrap()
    }
    pub fn listen_blocking(
        &self,
        addr: &Multiaddr,
    ) -> Result<ListenerId, TransportError<std::io::Error>> {
        let (tx, rx) = channel();
        let ev = InEvent::Listen(addr.clone(), tx);
        self.sender.blocking_send(ev).unwrap();
        rx.blocking_recv().unwrap()
    }
    pub async fn dial(&self, addr: &Multiaddr) -> Result<(), DialError> {
        let (tx, rx) = channel();
        let ev = InEvent::Dial(addr.clone(), tx);
        self.sender.send(ev).await.unwrap();
        rx.await.unwrap()
    }
    pub async fn listen(
        &self,
        addr: &Multiaddr,
    ) -> Result<ListenerId, TransportError<std::io::Error>> {
        let (tx, rx) = channel();
        let ev = InEvent::Listen(addr.clone(), tx);
        self.sender.send(ev).await.unwrap();
        rx.await.unwrap()
    }
    generate_handler_method_blocking!(
        AddExternalAddress:add_external_address_blocking(addr:Multiaddr)->();
        IsConnectedToPeerId:is_connected_blocking(peer_id: PeerId) -> bool;
        ListListeners:list_listeners_blocking()->Box<[Multiaddr]>;
        ListExternalAddresses:list_external_addresses_blocking()->Box<[Multiaddr]>;
        DisconnectFromPeerId:disconnect_peer_id_blocking(peer_id:PeerId)->Result<(),()>;
        RemoveExternalAddress:remove_external_address_blocking(addr:Multiaddr)->();
    );
    generate_handler_method!(
        AddExternalAddress:add_external_address(addr:Multiaddr)->();
        IsConnectedToPeerId:is_connected(peer_id: PeerId) -> bool;
        ListListeners:list_listeners()->Box<[Multiaddr]>;
        ListExternalAddresses:list_external_addresses()->Box<[Multiaddr]>;
        DisconnectFromPeerId:disconnect_peer_id(peer_id:PeerId)->Result<(),()>;
        RemoveExternalAddress:remove_external_address(addr:Multiaddr)->();
    );
}
