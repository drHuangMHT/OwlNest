use crate::event_bus::EventBusHandle;

use super::*;
use behaviour::Behaviour;
use libp2p::{
    core::{muxing::StreamMuxerBox, transport::Boxed, ConnectedPoint},
    swarm::{AddressRecord, AddressScore},
    Swarm as Libp2pSwarm,
};
use std::fmt::Debug;
use tracing::{debug, info, warn};

mod behaviour;
pub mod cli;
pub mod event_listener;
mod in_event;
pub mod manager;
pub mod out_event;
pub mod swarm_op;
mod select;

pub use event_listener::*;
pub use in_event::*;
pub use manager::Manager;
pub use out_event::OutEvent;

pub type Swarm = Libp2pSwarm<Behaviour>;
pub type SwarmOp = swarm_op::Op;
pub type SwarmOpResult = swarm_op::OpResult;
type SwarmTransport = Boxed<(PeerId, StreamMuxerBox)>;
pub(crate) type SwarmEvent = libp2p::swarm::SwarmEvent<OutEvent,<<Behaviour as libp2p::swarm::NetworkBehaviour>::ConnectionHandler as libp2p::swarm::ConnectionHandler>::Error>;

pub struct Builder {
    config: SwarmConfig,
}
impl Builder {
    pub fn new(config: SwarmConfig) -> Self {
        Self { config }
    }
    pub fn build(self, buffer_size: usize, mut ev_bus_handle: EventBusHandle) -> Manager {
        let ident = self.config.local_ident.clone();

        let (swarm_tx, mut swarm_rx) = mpsc::channel(buffer_size);
        let (ev_tx, mut ev_rx) = mpsc::channel(buffer_size);
        let (protocol_tx, mut protocol_rx) = mpsc::channel(buffer_size);
        let manager = Manager {
            swarm_sender: swarm_tx,
            behaviour_sender: protocol_tx,
        };
        let (behaviour, transport) = behaviour::Behaviour::new(self.config);
        tokio::spawn(async move {
            let mut swarm = libp2p::swarm::SwarmBuilder::with_tokio_executor(
                transport,
                behaviour,
                ident.get_peer_id(),
            )
            .build();
            loop {
                select! {
                    Some(ev) = swarm_rx.recv() => swarm_op_exec(&mut swarm, ev),
                    Some(ev) = protocol_rx.recv() => map_protocol_ev(&mut swarm,&mut ev_bus_handle ,ev).await,
                    out_event = swarm.select_next_some() => {handle_swarm_event(&out_event,&mut swarm);ev_tx.send(out_event).await.unwrap()}
                };
            }
        });
        tokio::spawn(async move {
            let (kad_ev_in, kad_op_in) = kad::event_listener::setup_event_listener(8);
            let op_in_bundle = (kad_op_in);
            loop {
                select! {
                    Some(ev) = ev_rx.recv() =>{
                        match ev{
                            SwarmEvent::Behaviour(ev)=>{
                                match ev{
                            OutEvent::Messaging(_) => todo!(),
                            OutEvent::Tethering(ev) => todo!(),
                            OutEvent::RelayServer(_) => todo!(),
                            OutEvent::RelayClient(_) => todo!(),
                            OutEvent::Kad(ev) => kad_ev_in.send(ev).await.unwrap(),
                            OutEvent::Identify(_) => todo!(),
                            OutEvent::Mdns(_) => todo!(),}
                            }
                            ev=>{
                                todo!()
                            }
                        }
                    }
                }
            }
        });
        manager
    }
}

#[inline]
fn handle_swarm_event(ev: &SwarmEvent, swarm: &mut Swarm) {
    match ev {
        SwarmEvent::Behaviour(event) => handle_behaviour_event(swarm, &event),
        SwarmEvent::NewListenAddr { address, .. } => info!("Listening on {:?}", address),
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => kad_add(swarm, peer_id.clone(), endpoint.clone()),
        SwarmEvent::ConnectionClosed {
            peer_id, endpoint, ..
        } => kad_remove(swarm, peer_id.clone(), endpoint.clone()),
        SwarmEvent::IncomingConnection {
            send_back_addr,
            local_addr,
        } => debug!(
            "Incoming connection from {} on local address {}",
            send_back_addr, local_addr
        ),
        SwarmEvent::IncomingConnectionError {
            local_addr,
            send_back_addr,
            error,
        } => info!(
            "Incoming connection error from {} on local address {}, error: {:?}",
            send_back_addr, local_addr, error
        ),
        SwarmEvent::OutgoingConnectionError { peer_id, error } => info!(
            "Outgoing connection error to peer {:?}: {:?}",
            peer_id, error
        ),
        SwarmEvent::ExpiredListenAddr { address, .. } => {
            info!("Expired listen address: {}", address)
        }
        SwarmEvent::ListenerClosed {
            addresses, reason, ..
        } => info!("Listener on address {:?} closed: {:?}", addresses, reason),
        SwarmEvent::ListenerError { listener_id, error } => {
            info!("Listener {:?} reported an error {}", listener_id, error)
        }
        SwarmEvent::Dialing(peer_id) => debug!("Dailing peer {}", peer_id),
    }
}

#[inline]
fn swarm_op_exec(swarm: &mut Swarm, ev: in_event::InEvent) {
    use swarm_op::*;
    let (op, callback) = ev.into_inner();
    match op {
        Op::Dial(addr) => {
            let result = OpResult::Dial(swarm.dial(addr.clone()).map_err(|e| e.into()));
            handle_callback(callback, result)
        }
        Op::Listen(addr) => {
            let result = OpResult::Listen(swarm.listen_on(addr.clone()).map_err(|e| e.into()));
            handle_callback(callback, result)
        }
        Op::AddExternalAddress(addr, score) => {
            let score = match score {
                Some(v) => AddressScore::Finite(v),
                None => AddressScore::Infinite,
            };
            let result = match swarm.add_external_address(addr.clone(), score.clone()) {
                libp2p::swarm::AddAddressResult::Inserted { .. } => {
                    OpResult::AddExternalAddress(AddExternalAddressResult::Inserted)
                }
                libp2p::swarm::AddAddressResult::Updated { .. } => {
                    OpResult::AddExternalAddress(AddExternalAddressResult::Updated)
                }
            };
            handle_callback(callback, result)
        }
        Op::RemoveExternalAddress(addr) => {
            let result = OpResult::RemoveExternalAddress(swarm.remove_external_address(&addr));
            handle_callback(callback, result)
        }
        // Op::BanByPeerId(peer_id) => {
        //     swarm.ban_peer_id(peer_id.clone());
        //     let result = OpResult::BanByPeerId;
        //     handle_callback(callback, result)
        // }
        // Op::UnbanByPeerId(peer_id) => {
        //     swarm.unban_peer_id(peer_id.clone());
        //     let result = OpResult::UnbanByPeerId;
        //     handle_callback(callback, result)
        // }
        Op::DisconnectFromPeerId(peer_id) => {
            let result = OpResult::DisconnectFromPeerId(swarm.disconnect_peer_id(peer_id.clone()));
            handle_callback(callback, result)
        }
        Op::ListExternalAddresses => {
            let addr_list = swarm
                .external_addresses()
                .map(|record| record.clone())
                .collect::<Vec<AddressRecord>>();
            let result = OpResult::ListExternalAddresses(
                addr_list.into_iter().map(|v| v.into()).collect::<_>(),
            );
            handle_callback(callback, result)
        }
        Op::ListListeners => {
            let listener_list = swarm
                .listeners()
                .map(|addr| addr.clone())
                .collect::<Vec<Multiaddr>>();
            let result = OpResult::ListListeners(listener_list);
            handle_callback(callback, result)
        }
        Op::IsConnectedToPeerId(peer_id) => {
            let result = OpResult::IsConnectedToPeerId(swarm.is_connected(&peer_id));
            handle_callback(callback, result)
        }
    }
}

#[inline]
async fn map_protocol_ev(swarm: &mut Swarm, manager: &mut EventBusHandle, ev: BehaviourInEvent) {
    match ev {
        BehaviourInEvent::Messaging(ev) => swarm.behaviour_mut().messaging.push_event(ev),
        BehaviourInEvent::Tethering(ev) => swarm.behaviour_mut().tethering.push_event(ev),
        BehaviourInEvent::Kad(ev) => {
            kad::map_in_event(&mut swarm.behaviour_mut().kad, manager, ev).await
        }
    }
}

#[inline]
fn handle_behaviour_event(swarm: &mut Swarm, ev: &OutEvent) {
    match ev {
        OutEvent::Kad(ev) => kad::ev_dispatch(ev),
        OutEvent::Identify(ev) => identify::ev_dispatch(ev),
        OutEvent::Mdns(ev) => mdns::ev_dispatch(&ev, swarm),
        OutEvent::Messaging(ev) => messaging::ev_dispatch(ev),
        OutEvent::Tethering(ev) => tethering::ev_dispatch(ev),
        OutEvent::RelayServer(ev) => relay_server::ev_dispatch(ev),
        OutEvent::RelayClient(ev) => relay_client::ev_dispatch(ev),
    }
}

#[inline]
fn kad_add(swarm: &mut Swarm, peer_id: PeerId, endpoint: ConnectedPoint) {
    match endpoint {
        libp2p::core::ConnectedPoint::Dialer { address, .. } => {
            swarm.behaviour_mut().kad.add_address(&peer_id, address);
        }
        libp2p::core::ConnectedPoint::Listener { send_back_addr, .. } => {
            swarm
                .behaviour_mut()
                .kad
                .add_address(&peer_id, send_back_addr);
        }
    }
}

#[inline]
fn kad_remove(swarm: &mut Swarm, peer_id: PeerId, endpoint: ConnectedPoint) {
    match endpoint {
        libp2p::core::ConnectedPoint::Dialer { address, .. } => {
            swarm.behaviour_mut().kad.remove_address(&peer_id, &address);
        }
        libp2p::core::ConnectedPoint::Listener { send_back_addr, .. } => {
            swarm
                .behaviour_mut()
                .kad
                .remove_address(&peer_id, &send_back_addr);
        }
    }
}

#[inline]
fn handle_callback<T>(callback: oneshot::Sender<T>, result: T)
where
    T: Debug,
{
    if let Err(v) = callback.send(result) {
        warn!("Failed to send callback: {:?}", v)
    }
}
