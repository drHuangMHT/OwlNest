use std::sync::Mutex;

use owlnest::{
    net::p2p::{identity::IdentityUnion, protocols},
    *,
};
use tracing::Level;

#[tokio::main]
async fn main() {
    setup_logging();
    setup_peer();
    let _ = tokio::signal::ctrl_c().await;
}

fn setup_peer() {
    let local_ident = IdentityUnion::generate();
    let swarm_config = net::p2p::SwarmConfig {
        local_ident: local_ident.clone(),
        kad: protocols::kad::Config::default(),
        identify: protocols::identify::Config::new(
            "/owlnest/0.0.1".into(),
            local_ident.get_pubkey(),
        ),
        mdns: protocols::mdns::Config::default(),
        messaging: protocols::messaging::Config::default(),
        tethering: protocols::tethering::Config::default(),
        relay_server: protocols::relay_server::Config::default(),
    };
    let mgr = net::p2p::swarm::Builder::new(swarm_config).build(8);
    utils::cli::setup_interactive_shell(local_ident.clone(), mgr.clone());
}

fn setup_logging() {
    let time = chrono::Local::now().to_rfc3339();
    let log_file_handle = match std::fs::create_dir("./logs") {
        Ok(_) => std::fs::File::create(format!("./logs/{}.log", time)).unwrap(),
        Err(e) => {
            let error = format!("{:?}", e);
            if error.contains("AlreadyExists") {
                std::fs::File::create(format!("./logs/{}.log", time)).unwrap()
            } else {
                std::fs::File::create(format!("{}.log", time)).unwrap()
            }
        }
    };
    tracing_subscriber::fmt::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(Mutex::new(log_file_handle))
        .init();
}
