use super::{protocol, Error};
use crate::net::p2p::handler_prelude::*;
use futures_timer::Delay;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Duration};
use tracing::{debug, warn, trace};

#[derive(Debug)]
pub enum FromBehaviourEvent {
    QueryAdvertisedPeer,
    AnswerAdvertisedPeer(Vec<PeerId>),
    StartAdvertiseSelf,
    StopAdvertiseSelf,
}
#[derive(Debug)]
pub enum ToBehaviourEvent {
    IncomingQuery,
    QueryAnswered(Vec<PeerId>),
    IncomingAdvertiseReq(bool),
    Error(Error),
    Unsupported,
    InboundNegotiated,
    OutboundNegotiated,
}

pub enum State {
    Inactive { reported: bool },
    Active,
}

#[derive(Debug, Serialize, Deserialize)]
enum Packet {
    AdvertiseSelf(bool),
    QueryAdvertisedPeer,
    AnswerAdvertisedPeer(Vec<PeerId>),
}
impl Packet {
    #[inline]
    pub fn as_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
}
impl Into<ToBehaviourEvent> for Packet {
    fn into(self) -> ToBehaviourEvent {
        match self {
            Packet::AdvertiseSelf(bool) => ToBehaviourEvent::IncomingAdvertiseReq(bool),
            Packet::QueryAdvertisedPeer => ToBehaviourEvent::IncomingQuery,
            Packet::AnswerAdvertisedPeer(result) => ToBehaviourEvent::QueryAnswered(result),
        }
    }
}

pub struct Handler {
    state: State,
    pending_in_events: VecDeque<FromBehaviourEvent>,
    pending_out_events: VecDeque<ToBehaviourEvent>,
    timeout: Duration,
    inbound: Option<PendingVerf>,
    outbound: Option<OutboundState>,
}

use libp2p::swarm::{handler::DialUpgradeError, StreamUpgradeError};
impl Handler {
    pub fn new() -> Self {
        Self {
            state: State::Active,
            pending_in_events: VecDeque::new(),
            pending_out_events: VecDeque::new(),
            timeout: Duration::from_secs(20),
            inbound: None,
            outbound: None,
        }
    }
    #[inline]
    fn on_dial_upgrade_error(
        &mut self,
        DialUpgradeError { error, .. }: DialUpgradeError<
            <Self as ConnectionHandler>::OutboundOpenInfo,
            <Self as ConnectionHandler>::OutboundProtocol,
        >,
    ) {
        self.outbound = None;
        match error {
            StreamUpgradeError::NegotiationFailed => {
                self.state = State::Inactive { reported: false };
            }
            e => {
                warn!(
                    "Error occurred when negotiating protocol {}: {:?}",
                    protocol::PROTOCOL_NAME,
                    e
                )
            }
        }
    }
}

use libp2p::core::upgrade::ReadyUpgrade;
use libp2p::swarm::{ConnectionHandlerEvent, KeepAlive, SubstreamProtocol};
impl ConnectionHandler for Handler {
    type FromBehaviour = FromBehaviourEvent;
    type ToBehaviour = ToBehaviourEvent;
    type Error = Error;
    type InboundProtocol = ReadyUpgrade<&'static str>;
    type OutboundProtocol = ReadyUpgrade<&'static str>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();
    fn listen_protocol(
        &self,
    ) -> libp2p::swarm::SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(ReadyUpgrade::new(protocol::PROTOCOL_NAME), ())
    }
    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        debug!("Received event {:#?}", event);
        self.pending_in_events.push_front(event)
    }
    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }
    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<
        libp2p::swarm::ConnectionHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::ToBehaviour,
            Self::Error,
        >,
    > {
        match self.state {
            State::Inactive { reported: true } => return Poll::Pending,
            State::Inactive { reported: false } => {
                self.state = State::Inactive { reported: true };
                return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviourEvent::Unsupported,
                ));
            }
            State::Active => {}
        };
        if let Some(fut) = self.inbound.as_mut() {
            match fut.poll_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(Err(e)) => {
                    let error = Error::IO(format!("IO Error: {:?}", e));
                    self.pending_out_events
                        .push_front(ToBehaviourEvent::Error(error));
                    self.inbound = None;
                }
                Poll::Ready(Ok((stream, bytes))) => {
                    self.inbound = Some(super::protocol::recv(stream).boxed());
                    match serde_json::from_slice::<Packet>(&bytes) {
                        Ok(packet) => self.pending_out_events.push_front(packet.into()),
                        Err(e) => self.pending_out_events.push_front(ToBehaviourEvent::Error(
                            Error::UnrecognizedMessage(format!(
                                "Unrecognized message: {}, raw data: {}",
                                e,
                                String::from_utf8_lossy(&bytes)
                            )),
                        )),
                    }
                    let event =
                        ConnectionHandlerEvent::NotifyBehaviour(ToBehaviourEvent::IncomingQuery);
                    return Poll::Ready(event);
                }
            }
        }
        loop {
            match self.outbound.take() {
                Some(OutboundState::Busy(mut task, mut timer)) => {
                    match task.poll_unpin(cx) {
                        Poll::Pending => {
                            if timer.poll_unpin(cx).is_ready() {
                                self.pending_out_events
                                    .push_back(ToBehaviourEvent::Error(Error::Timeout))
                            } else {
                                // Put the future back
                                self.outbound = Some(OutboundState::Busy(task, timer));
                                // End the loop because the outbound is busy
                                break;
                            }
                        }
                        // Ready
                        Poll::Ready(Ok((stream, rtt))) => {
                            trace!("Successful IO send with rtt of {}ms",rtt.as_millis());
                            // Free the outbound
                            self.outbound = Some(OutboundState::Idle(stream));
                        }
                        // Ready but resolved to an error
                        Poll::Ready(Err(e)) => {
                            self.pending_out_events
                                .push_back(ToBehaviourEvent::Error(Error::IO(format!(
                                    "IO Error: {:?}",
                                    e
                                ))));
                        }
                    }
                }
                // Outbound is free, get the next message sent
                Some(OutboundState::Idle(stream)) => {
                    if let Some(ev) = self.pending_in_events.pop_back() {
                        use FromBehaviourEvent::*;
                        match ev {
                            QueryAdvertisedPeer => {
                                self.outbound = Some(OutboundState::Busy(
                                    protocol::send(stream, Packet::QueryAdvertisedPeer.as_bytes())
                                        .boxed(),
                                    Delay::new(self.timeout),
                                ))
                            }
                            AnswerAdvertisedPeer(result) => {
                                self.outbound = Some(OutboundState::Busy(protocol::send(stream, Packet::AnswerAdvertisedPeer(result).as_bytes()).boxed(), Delay::new(self.timeout)))
                            }
                            StartAdvertiseSelf=>{}
                            StopAdvertiseSelf=>{}
                        }
                    } else {
                        self.outbound = Some(OutboundState::Idle(stream));
                        break;
                    }
                }
                Some(OutboundState::OpenStream) => {
                    self.outbound = Some(OutboundState::OpenStream);
                    break;
                }
                None => {
                    self.outbound = Some(OutboundState::OpenStream);
                    let protocol =
                        SubstreamProtocol::new(ReadyUpgrade::new(protocol::PROTOCOL_NAME), ());
                    let event = ConnectionHandlerEvent::OutboundSubstreamRequest { protocol };
                    return Poll::Ready(event);
                }
            }
            if let Some(ev) = self.pending_out_events.pop_back() {
                return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(ev));
            }
        }
        Poll::Pending
    }
    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: stream,
                info: (),
            }) => {
                self.pending_out_events
                    .push_front(ToBehaviourEvent::InboundNegotiated);
                self.inbound = Some(super::protocol::recv(stream).boxed());
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: stream,
                ..
            }) => {
                self.pending_out_events
                    .push_front(ToBehaviourEvent::OutboundNegotiated);
                self.outbound = Some(OutboundState::Idle(stream));
            }
            ConnectionEvent::DialUpgradeError(e) => {
                self.on_dial_upgrade_error(e);
            }
            ConnectionEvent::AddressChange(_) | ConnectionEvent::ListenUpgradeError(_) => {}
            ConnectionEvent::LocalProtocolsChange(_) => {}
            ConnectionEvent::RemoteProtocolsChange(_) => {}
        }
    }
}

type PendingVerf = BoxFuture<'static, Result<(Stream, Vec<u8>), io::Error>>;
type PendingSend = BoxFuture<'static, Result<(Stream, Duration), io::Error>>;

enum OutboundState {
    OpenStream,
    Idle(Stream),
    Busy(PendingSend, Delay),
}
