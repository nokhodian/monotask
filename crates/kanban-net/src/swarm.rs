use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use libp2p::{
    identity, noise, tcp, yamux, PeerId, Swarm, SwarmBuilder,
    swarm::SwarmEvent,
    futures::StreamExt,
};
use kanban_storage::Storage;
use crate::{NetCommand, NetConfig, NetError, NetEvent};
use crate::behaviour::{ComposedBehaviour, ComposedBehaviourEvent};

pub(crate) async fn run(
    config: NetConfig,
    storage: Arc<Mutex<Storage>>,
    identity_bytes: [u8; 32],
    mut cmd_rx: mpsc::Receiver<NetCommand>,
    event_tx: mpsc::Sender<NetEvent>,
) {
    if let Err(e) = run_inner(config, storage, identity_bytes, &mut cmd_rx, &event_tx).await {
        tracing::error!("net task failed: {e}");
    }
}

fn build_swarm(identity_bytes: [u8; 32]) -> Result<Swarm<ComposedBehaviour>, NetError> {
    // Bridge: 32-byte seed → libp2p Ed25519 keypair (two separate dalek crates — bytes only)
    let mut key_bytes = identity_bytes;
    let secret = libp2p::identity::ed25519::SecretKey::try_from_bytes(&mut key_bytes)
        .map_err(|e| NetError::Libp2p(e.to_string()))?;
    let ed_kp = libp2p::identity::ed25519::Keypair::from(secret);
    let keypair = identity::Keypair::from(ed_kp);

    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Libp2p(e.to_string()))?
        .with_quic()
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Libp2p(e.to_string()))?
        .with_behaviour(|key, relay_behaviour| {
            ComposedBehaviour::new(key, relay_behaviour)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    e.to_string().into()
                })
        })
        .map_err(|e| NetError::Libp2p(format!("{e:?}")))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    Ok(swarm)
}

async fn run_inner(
    config: NetConfig,
    storage: Arc<Mutex<Storage>>,
    identity_bytes: [u8; 32],
    cmd_rx: &mut mpsc::Receiver<NetCommand>,
    event_tx: &mpsc::Sender<NetEvent>,
) -> Result<(), NetError> {
    use libp2p::Multiaddr;
    use crate::discovery::{announce_spaces, bootstrap_peers};

    let mut swarm = build_swarm(identity_bytes)?;

    let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", config.listen_port)
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| NetError::Libp2p(e.to_string()))?;
    swarm.listen_on(listen_addr)
        .map_err(|e| NetError::Libp2p(e.to_string()))?;

    for (peer_id, addr) in bootstrap_peers() {
        swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
    }
    swarm.behaviour_mut().kademlia.bootstrap().ok();

    let mut pubkey_cache: HashMap<PeerId, libp2p::identity::PublicKey> = HashMap::new();
    let mut my_spaces: Vec<String> = Vec::new();
    let mut reannounce = tokio::time::interval(Duration::from_secs(20 * 3600));
    let mut sync_states: HashMap<String, automerge::sync::State> = HashMap::new();

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    NetCommand::Stop => {
                        tracing::info!("net: stopping");
                        return Ok(());
                    }
                    NetCommand::AnnounceSpaces { space_ids } => {
                        my_spaces = space_ids.clone();
                        announce_spaces(&mut swarm.behaviour_mut().kademlia, &space_ids);
                    }
                    NetCommand::TriggerSync { board_id } => {
                        tracing::debug!("net: trigger sync board={board_id}");
                    }
                }
            }

            event = swarm.next() => {
                let Some(event) = event else { break };
                handle_swarm_event(
                    event,
                    &mut swarm,
                    &storage,
                    &mut pubkey_cache,
                    &my_spaces,
                    identity_bytes,
                    event_tx,
                    &mut sync_states,
                ).await;
            }

            _ = reannounce.tick() => {
                if !my_spaces.is_empty() {
                    announce_spaces(&mut swarm.behaviour_mut().kademlia, &my_spaces);
                }
            }
        }
    }
    Ok(())
}

async fn handle_swarm_event(
    event: SwarmEvent<ComposedBehaviourEvent>,
    swarm: &mut Swarm<ComposedBehaviour>,
    storage: &Arc<Mutex<Storage>>,
    pubkey_cache: &mut HashMap<PeerId, libp2p::identity::PublicKey>,
    my_spaces: &[String],
    identity_bytes: [u8; 32],
    event_tx: &mpsc::Sender<NetEvent>,
    sync_states: &mut HashMap<String, automerge::sync::State>,
) {
    use libp2p::{identify, mdns, kad, request_response};
    match event {
        SwarmEvent::Behaviour(ComposedBehaviourEvent::Identify(
            identify::Event::Received { peer_id, info, .. }
        )) => {
            pubkey_cache.insert(peer_id, info.public_key);
            for addr in info.listen_addrs {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
            }
        }

        SwarmEvent::Behaviour(ComposedBehaviourEvent::Mdns(
            mdns::Event::Discovered(peers)
        )) => {
            for (peer_id, addr) in peers {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                let _ = swarm.dial(peer_id);
                tracing::debug!("net: mDNS discovered {peer_id}");
            }
        }

        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            let _ = event_tx.send(NetEvent::PeerConnected { peer_id: peer_id.to_string() }).await;
            initiate_hello(swarm, storage, my_spaces, identity_bytes, peer_id);
        }

        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            let _ = event_tx.send(NetEvent::PeerDisconnected { peer_id: peer_id.to_string() }).await;
        }

        SwarmEvent::Behaviour(ComposedBehaviourEvent::Sync(
            request_response::Event::Message {
                peer,
                message: request_response::Message::Request { request, channel, .. },
            }
        )) => {
            let response = handle_sync_request(
                request, peer, storage, pubkey_cache, my_spaces, sync_states,
            );
            let _ = swarm.behaviour_mut().sync.send_response(channel, response);
        }

        SwarmEvent::Behaviour(ComposedBehaviourEvent::Sync(
            request_response::Event::Message {
                peer,
                message: request_response::Message::Response { response, .. },
            }
        )) => {
            handle_sync_response(response, peer, storage, event_tx, sync_states).await;
        }

        SwarmEvent::Behaviour(ComposedBehaviourEvent::Kademlia(
            kad::Event::OutboundQueryProgressed {
                result: kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FoundProviders {
                    providers, ..
                })),
                ..
            }
        )) => {
            for provider in providers {
                if provider != *swarm.local_peer_id() {
                    tracing::debug!("net: DHT found provider {provider}, dialing");
                    let _ = swarm.dial(provider);
                }
            }
        }

        _ => {}
    }
}

fn initiate_hello(
    swarm: &mut Swarm<ComposedBehaviour>,
    storage: &Arc<Mutex<Storage>>,
    my_spaces: &[String],
    identity_bytes: [u8; 32],
    peer_id: PeerId,
) {
    use crate::sync_protocol::SyncRequest;
    use kanban_crypto::Identity;

    for space_id in my_spaces {
        let board_ids = {
            let guard = storage.lock().unwrap();
            kanban_storage::space::get_space_boards(guard.conn(), space_id)
                .unwrap_or_default()
        };

        let identity = Identity::from_secret_bytes(&identity_bytes);
        let signature = identity.sign(space_id.as_bytes());

        swarm.behaviour_mut().sync.send_request(
            &peer_id,
            SyncRequest::Hello {
                space_id: space_id.clone(),
                board_ids,
                signature,
            },
        );
    }
}

fn handle_sync_request(
    request: crate::sync_protocol::SyncRequest,
    peer: PeerId,
    storage: &Arc<Mutex<Storage>>,
    pubkey_cache: &HashMap<PeerId, libp2p::identity::PublicKey>,
    _my_spaces: &[String],
    sync_states: &mut HashMap<String, automerge::sync::State>,
) -> crate::sync_protocol::SyncResponse {
    use crate::sync_protocol::{SyncRequest, SyncResponse};
    use kanban_crypto::Identity;

    match request {
        SyncRequest::Hello { space_id, board_ids: _their_board_ids, signature } => {
            let pubkey = match pubkey_cache.get(&peer) {
                Some(pk) => pk,
                None => return SyncResponse::Rejected { reason: "no pubkey cached".into() },
            };
            let ed_pk = match pubkey.clone().try_into_ed25519() {
                Ok(pk) => pk,
                Err(_) => return SyncResponse::Rejected { reason: "not ed25519".into() },
            };
            let pubkey_32: [u8; 32] = ed_pk.to_bytes();
            if Identity::verify(&pubkey_32, space_id.as_bytes(), &signature).is_err() {
                return SyncResponse::Rejected { reason: "bad signature".into() };
            }

            let pubkey_hex = hex::encode(pubkey_32);
            let is_member = {
                let guard = storage.lock().unwrap();
                kanban_storage::space::is_active_member(guard.conn(), &space_id, &pubkey_hex)
                    .unwrap_or(false)
            };
            if !is_member {
                return SyncResponse::Rejected { reason: "not a member".into() };
            }

            let my_board_ids = {
                let guard = storage.lock().unwrap();
                kanban_storage::space::get_space_boards(guard.conn(), &space_id)
                    .unwrap_or_default()
            };
            SyncResponse::HelloAck { board_ids: my_board_ids }
        }

        SyncRequest::BoardSync { board_id, sync_message } => {
            let sync_state = sync_states.entry(board_id.clone()).or_insert_with(automerge::sync::State::new);
            match process_incoming_sync(&board_id, &sync_message, storage, sync_state) {
                Ok(reply_msg) => SyncResponse::BoardSync { board_id, sync_message: reply_msg },
                Err(e) => SyncResponse::Rejected { reason: e.to_string() },
            }
        }
    }
}

fn process_incoming_sync(
    board_id: &str,
    sync_message: &[u8],
    storage: &Arc<Mutex<Storage>>,
    sync_state: &mut automerge::sync::State,
) -> Result<Option<Vec<u8>>, crate::NetError> {
    use automerge::{AutoCommit, sync as am_sync};
    use am_sync::SyncDoc;

    let msg = am_sync::Message::decode(sync_message)
        .map_err(|e| crate::NetError::Sync(e.to_string()))?;

    let mut doc = {
        let guard = storage.lock().unwrap();
        match guard.load_board(board_id) {
            Ok(d) => d,
            Err(_) => {
                let mut d = AutoCommit::new();
                kanban_core::init_doc(&mut d).map_err(|e| crate::NetError::Sync(e.to_string()))?;
                d
            }
        }
    };

    doc.sync().receive_sync_message(sync_state, msg)
        .map_err(|e| crate::NetError::Sync(e.to_string()))?;

    {
        let mut guard = storage.lock().unwrap();
        guard.save_board(board_id, &mut doc)
            .map_err(crate::NetError::Storage)?;
    }

    let reply = doc.sync().generate_sync_message(sync_state);

    Ok(reply.map(|m| m.encode()))
}

async fn handle_sync_response(
    response: crate::sync_protocol::SyncResponse,
    peer: PeerId,
    storage: &Arc<Mutex<Storage>>,
    event_tx: &mpsc::Sender<NetEvent>,
    sync_states: &mut HashMap<String, automerge::sync::State>,
) {
    use crate::sync_protocol::SyncResponse;
    match response {
        SyncResponse::HelloAck { board_ids: their_board_ids } => {
            tracing::debug!(
                "net: HelloAck from {} boards={}",
                peer, their_board_ids.len()
            );
        }
        SyncResponse::BoardSync { board_id, sync_message: Some(msg) } => {
            let sync_state = sync_states.entry(board_id.clone()).or_insert_with(automerge::sync::State::new);
            if let Err(e) = process_incoming_sync(&board_id, &msg, storage, sync_state) {
                let _ = event_tx.send(NetEvent::SyncError { board_id, error: e.to_string() }).await;
            } else {
                let _ = event_tx.send(NetEvent::BoardSynced { board_id, peer_id: peer.to_string() }).await;
            }
        }
        SyncResponse::BoardSync { board_id: _, sync_message: None } => {
            tracing::debug!("net: peer {} converged", peer);
        }
        SyncResponse::Rejected { reason } => {
            tracing::warn!("net: rejected by {}: {reason}", peer);
        }
    }
}
