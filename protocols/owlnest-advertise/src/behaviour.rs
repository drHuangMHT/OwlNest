use super::*;
use owlnest_macro::handle_callback_sender;
use owlnest_prelude::behaviour_prelude::*;
use std::collections::{HashSet, VecDeque};
use tracing::{debug, trace};

#[derive(Debug, Default)]
pub struct Behaviour {
    /// Pending events to emit to `Swarm`
    pending_out_events: VecDeque<OutEvent>,
    /// Pending events to be processed by this `Behaviour`.
    in_events: VecDeque<InEvent>,
    /// A set for all connected peers.
    advertised_peers: HashSet<PeerId>,
    pending_query_answer: VecDeque<PeerId>,
    connected_peers: HashSet<PeerId>,
    is_providing: bool,
}

impl Behaviour {
    pub fn new() -> Self {
        Default::default()
    }
    pub fn push_event(&mut self, ev: InEvent) {
        trace!("receive event {:?}", ev);
        self.in_events.push_back(ev)
    }
    pub fn is_advertising(&self, peer: &PeerId) -> bool {
        self.advertised_peers.contains(peer)
    }
    pub fn advertised_peers(&self) -> &HashSet<PeerId> {
        &self.advertised_peers
    }
    pub fn set_provider_status(&mut self, status: bool) {
        self.is_providing = status
    }
    pub fn get_provider_status(&self) -> bool {
        self.is_providing
    }
    pub fn remove_advertised(&mut self, peer_id: &PeerId) -> bool {
        self.advertised_peers.remove(peer_id)
    }
    pub fn clear_advertised(&mut self) {
        self.advertised_peers.clear()
    }
    pub fn new_pending_query(&mut self, peer: &PeerId) {
        self.pending_query_answer.push_back(*peer)
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = handler::Handler;
    type ToSwarm = OutEvent;

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: <Self::ConnectionHandler as ConnectionHandler>::ToBehaviour,
    ) {
        use handler::ToBehaviour::*;
        match event {
            IncomingQuery => {
                trace!(
                    "incoming query from {} on connection {}",
                    peer_id,
                    connection_id
                );
                self.pending_query_answer.push_back(peer_id);
            }
            IncomingAdvertiseReq(bool) => {
                if bool {
                    if self.advertised_peers.insert(peer_id) {
                        debug!("Now advertising peer {}", peer_id);
                    }
                } else if self.advertised_peers.remove(&peer_id) {
                    debug!("Stopped advertising peer {}", peer_id);
                };
            }
            QueryAnswered(result) => self.pending_out_events.push_back(OutEvent::QueryAnswered {
                from: peer_id,
                list: result,
            }),
            Error(e) => self.pending_out_events.push_back(OutEvent::Error(e)),
            InboundNegotiated => {}
            OutboundNegotiated => {
                self.connected_peers.insert(peer_id);
            }
        }
    }
    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<ToSwarm<super::OutEvent, handler::FromBehaviour>> {
        if let Some(peer_id) = self.pending_query_answer.pop_front() {
            trace!("Answering query from {}", peer_id);
            if self.is_providing {
                return Poll::Ready(ToSwarm::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::Any,
                    event: handler::FromBehaviour::AnswerAdvertisedPeer(
                        self.advertised_peers.iter().cloned().collect(),
                    ),
                });
            }
            return Poll::Ready(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::Any,
                event: handler::FromBehaviour::AnswerAdvertisedPeer(Vec::new()),
            });
        }
        if let Some(ev) = self.pending_out_events.pop_front() {
            return Poll::Ready(ToSwarm::GenerateEvent(ev));
        }
        while let Some(ev) = self.in_events.pop_front() {
            trace!("got command {:?}", ev);
            use InEvent::*;
            match ev {
                QueryAdvertisedPeer(relay) => {
                    if self.connected_peers.contains(&relay) {
                        return Poll::Ready(ToSwarm::NotifyHandler {
                            peer_id: relay,
                            handler: NotifyHandler::Any,
                            event: handler::FromBehaviour::QueryAdvertisedPeer,
                        });
                    }
                    self.pending_out_events
                        .push_back(OutEvent::Error(Error::NotProviding(relay)))
                }
                GetProviderState(id) => {
                    return Poll::Ready(ToSwarm::GenerateEvent(OutEvent::ProviderState(
                        self.is_providing,
                        id,
                    )))
                }
                SetRemoteAdvertisement { remote, state, id } => {
                    return Poll::Ready(ToSwarm::NotifyHandler {
                        peer_id: remote,
                        handler: NotifyHandler::Any,
                        event: handler::FromBehaviour::SetAdvertiseSelf(state, id),
                    })
                }
                SetProviderState(status, id) => {
                    self.set_provider_status(status);
                    return Poll::Ready(ToSwarm::GenerateEvent(OutEvent::ProviderState(
                        status, id,
                    )));
                }
                RemoveAdvertised(peer_id) => {
                    let result = self.advertised_peers.remove(&peer_id);
                    return Poll::Ready(ToSwarm::GenerateEvent(OutEvent::AdvertisedPeerChanged(
                        peer_id, result,
                    )));
                }
                ListAdvertised(callback) => {
                    handle_callback_sender!(self.advertised_peers.iter().cloned().collect()=>callback);
                }
                ClearAdvertised() => self.advertised_peers.clear(),
                ListConnected(callback) => {
                    handle_callback_sender!(self.connected_peers.iter().copied().collect() => callback);
                }
            }
        }
        Poll::Pending
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionClosed(closed) => {
                if closed.remaining_established < 1 {
                    self.advertised_peers.remove(&closed.peer_id);
                    self.connected_peers.remove(&closed.peer_id);
                }
            }
            FromSwarm::ConnectionEstablished(established) => {
                self.connected_peers.insert(established.peer_id);
            }
            _ => {}
        }
    }

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(handler::Handler::new())
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(handler::Handler::new())
    }
}
