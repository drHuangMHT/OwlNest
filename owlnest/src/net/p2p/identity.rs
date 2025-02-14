use libp2p::{
    identity::{self, Keypair},
    PeerId,
};
use std::path::Path;
use std::{fs, io::Write};

/// Identity of this swarm(peer), including the keypair
/// and the peer ID derived from it.
#[derive(Debug, Clone)]
pub struct IdentityUnion {
    keypair: identity::Keypair,
    peer_id: PeerId,
}

impl IdentityUnion {
    /// Generate a random identity using `ed25519`.  
    /// Note: RSA is not encouraged.
    pub fn generate() -> Self {
        let keypair = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());
        Self { keypair, peer_id }
    }

    /// Get the public key of the keypair.
    pub fn get_pubkey(&self) -> identity::PublicKey {
        self.keypair.public()
    }

    /// Get the clone of the keypair.
    /// NOTE: You should NEVER share this keypair to ANYONE. This is
    /// the only proof that you are actually you.
    pub fn get_keypair(&self) -> identity::Keypair {
        self.keypair.clone()
    }

    /// Return a clone of the `peer_id` field.
    pub fn get_peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Read an identity from exsiting keypair file generated by libp2p.  
    /// Other format will only result in error.
    pub fn from_file_protobuf_encoding<P>(path: P) -> Result<Self, Box<dyn std::error::Error>>
    where
        P: AsRef<Path>,
    {
        let buf = match fs::read(path) {
            Ok(buf) => buf,
            Err(e) => return Err(Box::new(e)),
        };
        let keypair = match Keypair::from_protobuf_encoding(&buf) {
            Ok(keypair) => keypair,
            Err(e) => return Err(Box::new(e)),
        };
        Ok(Self {
            peer_id: PeerId::from_public_key(&keypair.public()),
            keypair,
        })
    }

    /// Export the public key to a file that you can share with others.
    pub fn export_public_key(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        let buf = self.get_pubkey().encode_protobuf();
        Self::export_to_file(path, &buf)
    }

    /// Export the keypair to the given file.  
    /// NOTE: You should NEVER share this file with ANYONE. This is the
    /// only proof that you are actually you.
    pub fn export_keypair(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        Self::export_to_file(
            path,
            &self.keypair.to_protobuf_encoding().expect("Not a RSA key"),
        )
    }
    fn export_to_file<P>(path: P, buf: &[u8]) -> Result<(), std::io::Error>
    where
        P: AsRef<Path>,
    {
        let mut handle = std::fs::File::create(path)?;
        handle.write_all(buf)
    }
}

impl Default for IdentityUnion {
    fn default() -> Self {
        Self::generate()
    }
}

impl From<Keypair> for IdentityUnion {
    fn from(value: Keypair) -> Self {
        let peer_id = PeerId::from(value.public());
        Self {
            keypair: value,
            peer_id,
        }
    }
}
