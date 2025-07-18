use super::{protocol, Error, PeerId};
use futures_timer::Delay;
use owlnest_prelude::handler_prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Duration};
use tracing::trace;

#[derive(Debug)]
pub enum FromBehaviour {
    QueryAdvertisedPeer,
    AnswerAdvertisedPeer(Option<Box<[PeerId]>>),
    SetAdvertiseSelf(bool),
}
#[derive(Debug)]
pub enum ToBehaviour {
    IncomingQuery,
    QueryAnswered(Option<Box<[PeerId]>>),
    IncomingAdvertiseReq(bool),
    Error(Error),
    InboundNegotiated,
    OutboundNegotiated,
}
impl From<Packet> for ToBehaviour {
    fn from(value: Packet) -> Self {
        match value {
            Packet::AdvertiseSelf(bool) => ToBehaviour::IncomingAdvertiseReq(bool),
            Packet::QueryAdvertisedPeer => ToBehaviour::IncomingQuery,
            Packet::AnswerAdvertisedPeer(result) => ToBehaviour::QueryAnswered(result),
        }
    }
}

pub enum State {
    Inactive { reported: bool },
    Active,
}
impl Default for State {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Packet {
    AdvertiseSelf(bool),
    QueryAdvertisedPeer,
    AnswerAdvertisedPeer(Option<Box<[PeerId]>>),
}
impl Packet {
    #[inline]
    pub fn as_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
}

pub struct Handler {
    state: State,
    pending_in_events: VecDeque<FromBehaviour>,
    pending_out_events: VecDeque<ToBehaviour>,
    timeout: Duration,
    inbound: Option<PendingVerf>,
    outbound: Option<OutboundState>,
}
impl Default for Handler {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            pending_in_events: Default::default(),
            pending_out_events: Default::default(),
            state: Default::default(),
            inbound: Default::default(),
            outbound: Default::default(),
        }
    }
}

impl Handler {
    pub fn new() -> Self {
        Default::default()
    }
}

impl ConnectionHandler for Handler {
    type FromBehaviour = FromBehaviour;
    type ToBehaviour = ToBehaviour;
    type InboundProtocol = ReadyUpgrade<&'static str>;
    type OutboundProtocol = ReadyUpgrade<&'static str>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(ReadyUpgrade::new(protocol::PROTOCOL_NAME), ())
    }
    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        trace!("Got an event {:?} from behaviour", event);
        self.pending_in_events.push_back(event)
    }
    fn connection_keep_alive(&self) -> bool {
        true
    }
    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        match self.state {
            State::Inactive { reported: true } => return Poll::Pending,
            State::Inactive { reported: false } => {
                self.state = State::Inactive { reported: true };
            }
            State::Active => {}
        };
        self.poll_inbound(cx);
        if let Some(poll) = self.poll_outbound(cx) {
            return Poll::Ready(poll);
        }
        if let Some(ev) = self.pending_out_events.pop_front() {
            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(ev));
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
                self.inbound = Some(super::protocol::recv(stream).boxed());
                self.pending_out_events
                    .push_back(ToBehaviour::InboundNegotiated);
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: stream,
                ..
            }) => {
                self.outbound = Some(OutboundState::Idle(stream));
                self.pending_out_events
                    .push_back(ToBehaviour::OutboundNegotiated)
            }
            ConnectionEvent::DialUpgradeError(e) => {
                self.on_dial_upgrade_error(e);
            }
            // ConnectionEvent::AddressChange(_) | ConnectionEvent::ListenUpgradeError(_) => {}
            ConnectionEvent::LocalProtocolsChange(_) => {}
            ConnectionEvent::RemoteProtocolsChange(_) => {}
            uncovered => unimplemented!("New branch {:?} not covered", uncovered),
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

type PollResult = ConnectionHandlerEvent<
    <Handler as ConnectionHandler>::OutboundProtocol,
    <Handler as ConnectionHandler>::OutboundOpenInfo,
    <Handler as ConnectionHandler>::ToBehaviour,
>;

impl Handler {
    fn poll_inbound(&mut self, cx: &mut std::task::Context<'_>) {
        if let Some(fut) = self.inbound.as_mut() {
            match fut.poll_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(Err(e)) => {
                    let error = Error::IO(format!("IO Error: {e:?}"));
                    self.pending_out_events.push_back(ToBehaviour::Error(error));
                    self.inbound = None;
                }
                Poll::Ready(Ok((stream, bytes))) => {
                    self.inbound = Some(super::protocol::recv(stream).boxed());
                    match serde_json::from_slice::<Packet>(&bytes) {
                        Ok(packet) => {
                            self.pending_out_events.push_back(packet.into());
                        }
                        Err(e) => self.pending_out_events.push_back(ToBehaviour::Error(
                            Error::UnrecognizedMessage(format!(
                                "Unrecognized message: {e}, raw data: {}",
                                String::from_utf8_lossy(&bytes)
                            )),
                        )),
                    }
                }
            }
        }
    }
    fn poll_outbound(&mut self, cx: &mut std::task::Context<'_>) -> Option<PollResult> {
        loop {
            match self.outbound.take() {
                Some(OutboundState::Busy(mut task, mut timer)) => {
                    match task.poll_unpin(cx) {
                        Poll::Pending => {
                            if timer.poll_unpin(cx).is_ready() {
                                self.pending_out_events
                                    .push_back(ToBehaviour::Error(Error::Timeout))
                            } else {
                                // Put the future back
                                self.outbound = Some(OutboundState::Busy(task, timer));
                                // End the loop because the outbound is busy
                                break;
                            }
                        }
                        // Ready
                        Poll::Ready(Ok((stream, rtt))) => {
                            trace!("Successful IO send with rtt of {}ms", rtt.as_millis());
                            // Free the outbound
                            self.outbound = Some(OutboundState::Idle(stream));
                        }
                        // Ready but resolved to an error
                        Poll::Ready(Err(e)) => {
                            self.pending_out_events
                                .push_back(ToBehaviour::Error(Error::IO(format!(
                                    "IO Error: {e:?}"
                                ))));
                        }
                    }
                }
                // Outbound is free, get the next message sent
                Some(OutboundState::Idle(stream)) => {
                    if self.pending_in_events.is_empty() {
                        self.outbound = Some(OutboundState::Idle(stream));
                        break;
                    }
                    let ev = self.pending_in_events.pop_front().expect("already checked");
                    trace!("Taking out event {:?} from behaviour", ev);
                    use FromBehaviour::*;
                    match ev {
                        QueryAdvertisedPeer => {
                            self.outbound = Some(OutboundState::Busy(
                                protocol::send(stream, Packet::QueryAdvertisedPeer.as_bytes())
                                    .boxed(),
                                Delay::new(self.timeout),
                            ))
                        }
                        AnswerAdvertisedPeer(result) => {
                            self.outbound = Some(OutboundState::Busy(
                                protocol::send(
                                    stream,
                                    Packet::AnswerAdvertisedPeer(result).as_bytes(),
                                )
                                .boxed(),
                                Delay::new(self.timeout),
                            ))
                        }
                        SetAdvertiseSelf(state) => {
                            self.outbound = Some(OutboundState::Busy(
                                protocol::send(stream, Packet::AdvertiseSelf(state).as_bytes())
                                    .boxed(),
                                Delay::new(self.timeout),
                            ))
                        }
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
                    return Some(event);
                }
            }
        }
        None
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
                tracing::debug!(
                    "Error occurred when negotiating protocol {}: {:?}",
                    protocol::PROTOCOL_NAME,
                    e
                )
            }
        }
    }
}
