use super::error::SendError;
use super::{protocol, Config, Error, PROTOCOL_NAME};
use owlnest_prelude::handler_prelude::*;
use std::{collections::VecDeque, time::Duration};

#[derive(Debug)]
pub enum FromBehaviourEvent {
    // TODO
}
#[derive(Debug)]
pub enum ToBehaviourEvent {
    //TODO
}

enum State {
    Inactive { reported: bool },
    Active,
}

pub struct Handler {
    state: State,
    pending_in_events: VecDeque<FromBehaviourEvent>,
    pending_out_events: VecDeque<ToBehaviourEvent>,
    timeout: Duration,
    inbound: Option<PendingVerf>,
    outbound: Option<OutboundState>,
    // TODO
}

impl Handler {
    pub fn new(config: Config) -> Self {
        Self {
            state: State::Active,
            pending_in_events: VecDeque::new(),
            pending_out_events: VecDeque::new(),
            timeout: config.timeout,
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
                let e = format!("{:?}", e);
                if !e.contains("Timeout") {
                    // debug!(
                    //     "Error occurred when negotiating protocol {}: {:?}",
                    //     PROTOCOL_NAME, e
                    // )
                    todo!()
                }
            }
        }
    }
}

impl ConnectionHandler for Handler {
    type FromBehaviour = FromBehaviourEvent;
    type ToBehaviour = ToBehaviourEvent;
    type InboundProtocol = ReadyUpgrade<&'static str>;
    type OutboundProtocol = ReadyUpgrade<&'static str>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();
    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ())
    }
    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        // trace!("Received event {:#?}", event);
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
                todo!()
                // return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                //     ToBehaviourEvent::Unsupported,
                // ));
            }
            State::Active => {}
        };
        if let Some(fut) = self.inbound.as_mut() {
            match fut.poll_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(Err(e)) => {
                    // let error = Error::IO(format!("IO Error: {:?}", e));
                    // self.pending_out_events
                    //     .push_back(ToBehaviourEvent::Error(error));
                    // self.inbound = None;
                    todo!()
                }
                Poll::Ready(Ok((stream, bytes))) => {
                    // self.inbound = Some(super::protocol::recv(stream).boxed());
                    // let event = ConnectionHandlerEvent::NotifyBehaviour(
                    //     ToBehaviourEvent,
                    // );
                    // return Poll::Ready(event);
                    todo!()
                }
            }
        }
        loop {
            match self.outbound.take() {
                Some(OutboundState::Busy(mut task, id)) => {
                    match task.poll_unpin(cx) {
                        Poll::Pending => {
                            //     if timer.poll_unpin(cx).is_ready() {
                            //         self.pending_out_events
                            //             .push_back(ToBehaviourEvent::SendResult(
                            //                 Err(SendError::Timeout),
                            //                 id,
                            //             ))
                            //     } else {
                            //         // Put the future back
                            //         self.outbound = Some(OutboundState::Busy);
                            //         // End the loop because the outbound is busy
                            //         break;
                            // }
                        }
                        // Ready
                        Poll::Ready(Ok((stream, rtt))) => {
                            // self.pending_out_events
                            //     .push_back(ToBehaviourEvent);
                            // Free the outbound
                            self.outbound = Some(OutboundState::Idle(stream));
                        }
                        // Ready but resolved to an error
                        Poll::Ready(Err(e)) => {
                            // self.pending_out_events
                            //     .push_back(ToBehaviourEvent::Error);
                            todo!()
                        }
                    }
                }
                // Outbound is free, get the next message sent
                Some(OutboundState::Idle(stream)) => {
                    if let Some(ev) = self.pending_in_events.pop_front() {
                        match ev {
                            //FromBehaviourEvent::PostMessage(msg, id) => {
                            // Put Outbound into send state
                            // self.outbound = Some(OutboundState::Busy(
                            //     protocol::send(stream, msg.as_bytes()).boxed(),
                            // ))
                            _ => todo!(),
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
        #[allow(unused)]
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: stream,
                info: (),
            }) => {
                // self.inbound = Some(super::protocol::recv(stream).boxed());
                // self.pending_out_events
                //     .push_back(ToBehaviourEvent::InboundNegotiated)
            }
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol: stream,
                ..
            }) => {
                // self.outbound = Some(OutboundState::Idle(stream));
                // self.pending_out_events
                //     .push_back(ToBehaviourEvent::OutboundNegotiated)
            }
            ConnectionEvent::DialUpgradeError(e) => {
                self.on_dial_upgrade_error(e);
            }
            ConnectionEvent::AddressChange(_) | ConnectionEvent::ListenUpgradeError(_) => {}
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
    Busy(PendingSend, u64),
}
