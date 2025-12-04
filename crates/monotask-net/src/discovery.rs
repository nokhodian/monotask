use libp2p::kad::{self, RecordKey};
use sha2::{Digest, Sha256};

/// Derives the Kademlia record key for a Space.
/// Two peers use the same key → they find each other.
/// Format: SHA-256("monotask/space/{space_id_hyphenated_uuid}")
pub fn space_dht_key(space_id: &str) -> RecordKey {
    let input = format!("monotask/space/{space_id}");
    let hash = Sha256::digest(input.as_bytes());
    RecordKey::new(&hash)
}

/// Bootstrap node multiaddresses (public IPFS nodes).
pub fn bootstrap_peers() -> Vec<(libp2p::PeerId, libp2p::Multiaddr)> {
    [
        ("/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
         "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN"),
        ("/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
         "QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb"),
    ]
    .iter()
    .filter_map(|(addr_str, peer_str)| {
        let addr: libp2p::Multiaddr = addr_str.parse().ok()?;
        let peer_id: libp2p::PeerId = peer_str.parse().ok()?;
        Some((peer_id, addr))
    })
    .collect()
}

/// Announce all Spaces to the Kademlia DHT.
pub fn announce_spaces(kademlia: &mut kad::Behaviour<kad::store::MemoryStore>, space_ids: &[String]) {
    for space_id in space_ids {
        let key = space_dht_key(space_id);
        kademlia.start_providing(key).ok();
    }
}

/// Query the DHT for peers in a Space.
pub fn query_space_peers(kademlia: &mut kad::Behaviour<kad::store::MemoryStore>, space_id: &str) {
    let key = space_dht_key(space_id);
    kademlia.get_providers(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dht_key_is_deterministic() {
        let k1 = space_dht_key("550e8400-e29b-41d4-a716-446655440000");
        let k2 = space_dht_key("550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(k1, k2);
    }

    #[test]
    fn different_spaces_produce_different_keys() {
        let k1 = space_dht_key("space-a");
        let k2 = space_dht_key("space-b");
        assert_ne!(k1, k2);
    }
}
