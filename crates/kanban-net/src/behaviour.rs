use libp2p::{
    autonat, dcutr, identify, kad, mdns, relay, request_response,
    swarm::NetworkBehaviour, swarm::StreamProtocol,
};
use crate::sync_protocol::{MonotaskCodec, PROTOCOL_NAME};

/// All libp2p behaviours composed into one.
/// Order matters: `identify` must be listed first — Kademlia and DCUtR depend on it.
#[derive(NetworkBehaviour)]
pub struct ComposedBehaviour {
    pub identify:          identify::Behaviour,
    pub mdns:              mdns::tokio::Behaviour,
    pub kademlia:          kad::Behaviour<kad::store::MemoryStore>,
    pub relay_client:      relay::client::Behaviour,
    pub dcutr:             dcutr::Behaviour,
    pub autonat:           autonat::Behaviour,
    pub sync:              request_response::Behaviour<MonotaskCodec>,
}

impl ComposedBehaviour {
    pub fn new(
        local_key: &libp2p::identity::Keypair,
        relay_behaviour: relay::client::Behaviour,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let local_peer_id = local_key.public().to_peer_id();

        let identify = identify::Behaviour::new(
            identify::Config::new("/monotask/1.0.0".into(), local_key.public())
                .with_push_listen_addr_updates(true),
        );

        let mdns = mdns::tokio::Behaviour::new(
            mdns::Config::default(),
            local_peer_id,
        )?;

        let mut kademlia = kad::Behaviour::new(
            local_peer_id,
            kad::store::MemoryStore::new(local_peer_id),
        );
        kademlia.set_mode(Some(kad::Mode::Server));

        let dcutr = dcutr::Behaviour::new(local_peer_id);
        let autonat = autonat::Behaviour::new(local_peer_id, autonat::Config::default());

        let protocol = StreamProtocol::try_from_owned(PROTOCOL_NAME.to_string())?;
        let sync = request_response::Behaviour::new(
            vec![(protocol, request_response::ProtocolSupport::Full)],
            request_response::Config::default(),
        );

        Ok(Self {
            identify,
            mdns,
            kademlia,
            relay_client: relay_behaviour,
            dcutr,
            autonat,
            sync,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn behaviour_module_exists() { assert!(true); }
}
