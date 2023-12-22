use crate::{generate_handler_method, net::p2p::swarm::EventSender};
use either::Either;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tracing::{error, trace, warn};

mod behaviour;
mod config;
mod error;
mod handler;
mod op;
mod protocol;

pub use behaviour::Behaviour;
pub use config::Config;
pub use error::Error;
pub use protocol::PROTOCOL_NAME;

pub enum InEvent {
    SendFile {
        file: File,
        file_name: String,
        to: PeerId,
        send_id: u64,
        callback: oneshot::Sender<Result<Duration, FileSendReqError>>,
    },
    AcceptFile {
        file_or_folder: Either<File, PathBuf>,
        recv_id: u64,
        callback: oneshot::Sender<Result<Duration, FileRecvError>>,
    },
    ListPendingRecv(oneshot::Sender<Vec<RecvInfo>>),
    ListPendingSend(oneshot::Sender<Vec<PendingSendInfo>>),
    CancelRecv(u64, oneshot::Sender<Result<(), ()>>), // (local_recv_id, callback)
    CancelSend(u64, oneshot::Sender<Result<(), ()>>), // (local_send_id, callback)
}

#[derive(Debug)]
pub enum OutEvent {
    IncomingFile {
        from: PeerId,
        file_name: String,
        local_recv_id: u64,
    },
    RecvProgressed {
        local_recv_id: u64,
        finished: bool,
    },
    OngoingRecvError {
        local_recv_id: u64,
        error: String,
    },
    SendProgressed {
        local_send_id: u64,
        rtt: Option<Duration>,
    },
    ReqSendResult(oneshot::Sender<Result<Duration, SendError>>),
    CancelledSend(u64),
    Error(Error),
    InboundNegotiated(PeerId),
    OutboundNegotiated(PeerId),
    Unsupported(PeerId),
}

use tokio::sync::{mpsc, oneshot};

use crate::with_timeout;
#[allow(unused_macros)]
macro_rules! event_op {
    ($listener:ident,$pattern:pat,{$($ops:tt)+}) => {
        async move{
        loop{
            let ev = crate::handle_listener_result!($listener);
            if let SwarmEvent::Behaviour(BehaviourEvent::Messaging($pattern)) = ev.as_ref() {
                $($ops)+
            } else {
                continue;
            }
        }}
    };
}
use self::{
    behaviour::{PendingSendInfo, RecvInfo},
    error::{FileRecvError, FileSendReqError, SendError},
};
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Handle {
    sender: mpsc::Sender<InEvent>,
    event_tx: EventSender,
    send_counter: Arc<AtomicU64>,
}
impl Handle {
    pub fn new(buffer: usize, event_tx: &EventSender) -> (Self, mpsc::Receiver<InEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (
            Self {
                sender: tx,
                event_tx: event_tx.clone(),
                send_counter: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }

    pub async fn send_file(
        &self,
        to: PeerId,
        path: impl AsRef<Path>,
    ) -> Result<(u64, Duration), FileSendReqError> {
        if path.as_ref().is_dir() {
            // Reject sending directory
            return Err(FileSendReqError::IsDirectory);
        }
        // Get the handle to the file(locking)
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(false)
            .open(path.as_ref())
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => FileSendReqError::FileNotFound,
                std::io::ErrorKind::PermissionDenied => FileSendReqError::PermissionDenied,
                e => FileSendReqError::OtherFsError(e),
            })?;
        let (tx, rx) = oneshot::channel();
        let send_id = self.next_id();
        let ev = InEvent::SendFile {
            file,
            file_name: path
                .as_ref()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            to,
            send_id,
            callback: tx,
        };
        self.sender.send(ev).await.expect("send to succeed");
        match with_timeout!(rx, 10) {
            Ok(v) => v.expect("callback to succeed").map(|v| (send_id, v)),
            Err(_) => {
                warn!("timeout reached for a timed future");
                Err(FileSendReqError::Timeout)
            }
        }
    }
    pub async fn recv_file(
        &self,
        recv_id: u64,
        path_to_write: impl AsRef<Path>,
    ) -> Result<Duration, FileRecvError> {
        let path_to_write = path_to_write.as_ref();
        let folder_or_file = match fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path_to_write)
        {
            Ok(file) => Either::Left(file),
            Err(err) => {
                error!("{:?}", err);
                match err.kind() {
                    std::io::ErrorKind::IsADirectory => Either::Right(path_to_write.to_owned()),
                    e => return Err(FileRecvError::FsError(e)),
                }
            }
        };

        let (tx, rx) = oneshot::channel();
        let ev = InEvent::AcceptFile {
            file_or_folder: folder_or_file,
            recv_id,
            callback: tx,
        };
        self.sender.send(ev).await.expect("send to succeed");
        match with_timeout!(rx, 10) {
            Ok(rtt) => rtt.expect("callback to succeed"),
            Err(_) => {
                warn!("timeout reached for a timed future");
                Err(FileRecvError::Timeout)
            }
        }
    }
    generate_handler_method!(
        ListPendingRecv:list_pending_recv()->Vec<RecvInfo>;
        ListPendingSend:list_pending_send()->Vec<PendingSendInfo>;
        CancelSend:cancel_send(local_send_id:u64)->Result<(),()>;
        CancelRecv:cancel_recv(local_recv_id:u64)->Result<(),()>;
    );
    fn next_id(&self) -> u64 {
        self.send_counter.fetch_add(1, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod test {
    use std::{fs, io::Read, str::FromStr, thread, time::Duration};

    use libp2p::{Multiaddr, PeerId};

    #[allow(unused)]
    use crate::net::p2p::{
        setup_default, setup_logging,
        swarm::{behaviour::BehaviourEvent, Manager, SwarmEvent},
    };
    const SOURCE_FILE: &str = "../Cargo.lock";
    const DEST_FILE: &str = "../test_lock_file";

    #[test]
    fn single_send_recv() {
        let (peer1_m, peer2_m) = setup_peer();
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        assert_eq!(
            peer1_m
                .executor()
                .block_on(peer1_m.blob_transfer().list_pending_send())
                .len(),
            1
        );
        wait_recv(
            &peer2_m,
            peer2_m
                .executor()
                .block_on(peer2_m.blob_transfer().list_pending_recv())[0]
                .local_recv_id,
            DEST_FILE,
        );
        assert!(verify_file(SOURCE_FILE, DEST_FILE));
    }

    #[test]
    fn cancel_single_send() {
        let (peer1_m, peer2_m) = setup_peer();
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        let _ = &peer2_m
            .executor()
            .block_on(peer2_m.blob_transfer().list_pending_recv())[0];
        peer1_m
            .executor()
            .block_on(peer1_m.blob_transfer().cancel_send(0))
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(
            peer2_m
                .executor()
                .block_on(peer2_m.blob_transfer().list_pending_recv())
                .len()
                == 0
        )
    }

    #[test]
    fn cancel_single_send_in_multiple() {
        let (peer1_m, peer2_m) = setup_peer();
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        let _ = peer2_m
            .executor()
            .block_on(peer2_m.blob_transfer().list_pending_recv())[2];
        peer1_m
            .executor()
            .block_on(peer1_m.blob_transfer().cancel_send(2))
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(
            peer2_m
                .executor()
                .block_on(peer2_m.blob_transfer().list_pending_recv())
                .len()
                == 3
        );
        assert!(
            peer2_m
                .executor()
                .block_on(peer2_m.blob_transfer().list_pending_recv())
                .extract_if(|v| v.local_recv_id == 2)
                .count()
                == 0
        ); // Check if the recv_id increments linearly
    }

    #[test]
    fn cancel_single_recv() {
        let (peer1_m, peer2_m) = setup_peer();
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        let recv_id = peer2_m
            .executor()
            .block_on(peer2_m.blob_transfer().list_pending_recv())[0]
            .local_recv_id;
        peer2_m
            .executor()
            .block_on(peer2_m.blob_transfer().cancel_recv(recv_id))
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(
            peer1_m
                .executor()
                .block_on(peer1_m.blob_transfer().list_pending_send())
                .len()
                == 0
        )
    }

    #[test]
    fn cancel_single_recv_in_multiple() {
        let (peer1_m, peer2_m) = setup_peer();
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        send(&peer1_m, peer2_m.identity().get_peer_id(), SOURCE_FILE);
        let _ = peer1_m
            .executor()
            .block_on(peer1_m.blob_transfer().list_pending_send())[2];
        peer2_m
            .executor()
            .block_on(peer2_m.blob_transfer().cancel_recv(2))
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(
            peer1_m
                .executor()
                .block_on(peer1_m.blob_transfer().list_pending_send())
                .len()
                == 3
        );
        assert!(
            peer1_m
                .executor()
                .block_on(peer1_m.blob_transfer().list_pending_send())
                .extract_if(|v| v.local_send_id == 2)
                .count()
                == 0
        ); // Check if the send_id increments linearly
    }

    fn setup_peer() -> (Manager, Manager) {
        let (peer1_m, _) = setup_default();
        let (peer2_m, _) = setup_default();
        // setup_logging();
        peer1_m
            .executor()
            .block_on(
                peer1_m
                    .swarm()
                    .listen(&Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()),
            )
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        let peer1_listen = &peer1_m.swarm().list_listeners_blocking()[0];
        thread::sleep(Duration::from_millis(100));
        peer2_m.swarm().dial_blocking(peer1_listen).unwrap();
        thread::sleep(Duration::from_millis(200));
        (peer1_m, peer2_m)
    }

    fn send(manager: &Manager, to: PeerId, file: &str) {
        manager
            .executor()
            .block_on(manager.blob_transfer().send_file(to, file))
            .unwrap();
        thread::sleep(Duration::from_millis(200));
    }

    fn wait_recv(manager: &Manager, recv_id: u64, file: &str) {
        let manager_clone = manager.clone();
        let handle = manager.executor().spawn(async move {
            let mut listener = manager_clone.event_subscriber().subscribe();
            while let Ok(ev) = listener.recv().await {
                if let SwarmEvent::Behaviour(BehaviourEvent::BlobTransfer(
                    super::OutEvent::RecvProgressed { finished, .. },
                )) = ev.as_ref()
                {
                    if *finished {
                        return;
                    }
                }
            }
        });
        manager
            .executor()
            .block_on(manager.blob_transfer().recv_file(recv_id, file))
            .unwrap();
        manager.executor().block_on(handle).unwrap();
    }

    /// Verify and clean up
    fn verify_file(left: &str, right: &str) -> bool {
        let mut left_file_buf = Vec::new();
        fs::OpenOptions::new()
            .read(true)
            .open(left)
            .unwrap()
            .read_to_end(&mut left_file_buf)
            .unwrap();
        let left_file_hash = xxhash_rust::xxh3::xxh3_128(&left_file_buf);
        drop(left_file_buf);
        let mut right_file_buf = Vec::new();
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(right)
            .unwrap()
            .read_to_end(&mut right_file_buf)
            .unwrap();
        fs::remove_file(DEST_FILE).unwrap();
        let right_file_hash = xxhash_rust::xxh3::xxh3_128(&right_file_buf);
        drop(right_file_buf);
        left_file_hash == right_file_hash
    }
}
