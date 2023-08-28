use super::*;
use crate::net::p2p::swarm;
use swarm::manager::Manager;

/// Top-level handler for `kad` command.
pub fn handle_kad(manager: &Manager, command: Vec<&str>) {
    if command.len() < 2 {
        println!("Missing subcommands. Type \"kad help\" for more information");
        return;
    }
    match command[1] {
        "lookup" => handle_kad_lookup(manager, command),
        "help" => println!("{}", TOP_HELP_MESSAGE),
        _ => println!("Unrecoginzed subcommands. Type \"kad help\" for more information"),
    }
}

/// Handler for `kad lookup` command.
fn handle_kad_lookup(manager: &Manager, command: Vec<&str>) {
    if command.len() < 3 {
        println!("Missing required argument: <peer ID>");
        return;
    }
    let peer_id = match PeerId::from_str(command[2]) {
        Ok(peer_id) => peer_id,
        Err(e) => {
            println!("Error: Failed parsing peer ID `{}`: {}", command[1], e);
            return;
        }
    };
    match manager.kad().blocking_lookup(peer_id){
        Ok(v) => println!("{:?}",v),
        Err(_) => println!("Lookup failed"),
    }
}

/// Top-level help message for `kad` command.
const TOP_HELP_MESSAGE: &str = r#"
Protocol `/ipfs/kad/1.0.0`

Available Subcommands:
    lookup <peer ID>        
                Initiate a lookup for the given peer.
"#;
