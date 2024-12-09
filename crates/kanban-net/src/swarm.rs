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
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(24 * 3600)))
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

    // Try the configured port first; fall back to OS-assigned port if it's taken.
    let primary_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", config.listen_port)
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| NetError::Libp2p(e.to_string()))?;
    if swarm.listen_on(primary_addr.clone()).is_err() {
        eprintln!("NET: port {} in use, falling back to OS-assigned port", config.listen_port);
        let fallback: Multiaddr = "/ip4/0.0.0.0/tcp/0".parse()
            .map_err(|e: libp2p::multiaddr::Error| NetError::Libp2p(e.to_string()))?;
        swarm.listen_on(fallback)
            .map_err(|e| NetError::Libp2p(e.to_string()))?;
    }

    for (peer_id, addr) in bootstrap_peers() {
        swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
    }
    swarm.behaviour_mut().kademlia.bootstrap().ok();

    // Dial any manually-specified bootstrap peers immediately (bypasses mDNS).
    for addr_str in &config.bootstrap_peers {
        match addr_str.parse::<Multiaddr>() {
            Ok(addr) => {
                tracing::info!("net: dialing bootstrap peer {addr}");
                let _ = swarm.dial(addr);
            }
            Err(e) => tracing::warn!("net: invalid peer addr '{addr_str}': {e}"),
        }
    }

    // Keep a mutable list of peer addresses for reconnection.
    // This grows as AddPeer commands arrive so dynamically-added peers
    // are also re-dialed by the 30-second reconnect tick.
    let mut bootstrap_peer_addrs: Vec<String> = config.bootstrap_peers.clone();

    let mut pubkey_cache: HashMap<PeerId, libp2p::identity::PublicKey> = HashMap::new();
    let mut connected_peers: std::collections::HashSet<PeerId> = std::collections::HashSet::new();
    let mut my_spaces: Vec<String> = Vec::new();
    let mut reannounce = tokio::time::interval(Duration::from_secs(20 * 3600));
    // Reconnect to saved peers every 10 seconds when not connected.
    let mut reconnect_tick = tokio::time::interval(Duration::from_secs(10));
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
                        // Also query DHT for peers already in these spaces (internet discovery)
                        for space_id in &space_ids {
                            crate::discovery::query_space_peers(
                                &mut swarm.behaviour_mut().kademlia,
                                space_id,
                            );
                        }
                    }
                    NetCommand::TriggerSync { board_id } => {
                        use crate::sync_protocol::SyncRequest;
                        use automerge::sync::SyncDoc;

                        eprintln!("TRIGGER_SYNC[{board_id:.8}]: {} connected peers", connected_peers.len());
                        // If no peers are connected, re-dial saved bootstrap peers so the
                        // next connection event will trigger a full sync automatically.
                        if connected_peers.is_empty() {
                            for addr_str in &bootstrap_peer_addrs {
                                if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                    let _ = swarm.dial(addr);
                                }
                            }
                        }
                        if !connected_peers.is_empty() {
                            // Clear stale initiator states so the peer gets the latest changes.
                            for peer_id in &connected_peers {
                                sync_states.remove(&format!("i/{peer_id}/{board_id}"));
                            }

                            let peers: Vec<PeerId> = connected_peers.iter().copied().collect();
                            let mut doc_opt = {
                                let guard = storage.lock().unwrap();
                                guard.load_board(&board_id).ok()
                            };

                            if let Some(ref mut doc) = doc_opt {
                                for peer_id in &peers {
                                    let sync_key = format!("i/{peer_id}/{board_id}");
                                    let sync_state = sync_states
                                        .entry(sync_key)
                                        .or_insert_with(automerge::sync::State::new);
                                    if let Some(msg) = doc.sync().generate_sync_message(sync_state) {
                                        let bytes = msg.encode();
                                        eprintln!("TRIGGER_SYNC[{board_id:.8}]: → {peer_id} ({} bytes)", bytes.len());
                                        swarm.behaviour_mut().sync.send_request(
                                            peer_id,
                                            SyncRequest::BoardSync {
                                                board_id: board_id.clone(),
                                                sync_message: bytes,
                                            },
                                        );
                                    }
                                }
                            } else {
                                eprintln!("TRIGGER_SYNC[{board_id:.8}]: board not found, skipping");
                            }
                        }
                    }
                    NetCommand::ForceRediscovery => {
                        eprintln!("FORCE_REDISCOVERY: spaces={} connected_peers={}", my_spaces.len(), connected_peers.len());
                        // Clear all initiator sync states so the next Hello round forces a
                        // full re-sync even for boards where the peer was previously "up to date".
                        sync_states.retain(|k, _| !k.starts_with("i/"));
                        eprintln!("FORCE_REDISCOVERY: cleared initiator sync states, re-Hello {} peers", connected_peers.len());
                        if !my_spaces.is_empty() {
                            crate::discovery::announce_spaces(&mut swarm.behaviour_mut().kademlia, &my_spaces);
                            for space_id in &my_spaces {
                                crate::discovery::query_space_peers(&mut swarm.behaviour_mut().kademlia, space_id);
                            }
                        }
                        // Re-Hello all currently connected peers to trigger a fresh sync round
                        for &peer_id in &connected_peers {
                            initiate_hello(&mut swarm, &storage, &my_spaces, identity_bytes, peer_id);
                        }
                    }
                    NetCommand::AddPeer { addr } => {
                        use libp2p::Multiaddr;
                        match addr.parse::<Multiaddr>() {
                            Ok(multiaddr) => {
                                eprintln!("ADDPEER: dialing {multiaddr}");
                                let _ = swarm.dial(multiaddr);
                                // Also register for future reconnect attempts so the
                                // 30-second tick re-dials after a disconnect.
                                if !bootstrap_peer_addrs.contains(&addr) {
                                    bootstrap_peer_addrs.push(addr);
                                }
                            }
                            Err(e) => eprintln!("ADDPEER: invalid addr '{addr}': {e}"),
                        }
                    }
                    NetCommand::GetPeers { reply } => {
                        let peers: Vec<String> = connected_peers
                            .iter()
                            .map(|p| p.to_string())
                            .collect();
                        let _ = reply.send(peers);
                    }
                    NetCommand::GetListenAddrs { reply } => {
                        let addrs: Vec<String> = swarm.listeners()
                            .map(|a| a.to_string())
                            .collect();
                        let _ = reply.send(addrs);
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
                    &mut connected_peers,
                    &my_spaces,
                    identity_bytes,
                    event_tx,
                    &mut sync_states,
                ).await;
            }

            _ = reannounce.tick() => {
                if !my_spaces.is_empty() {
                    announce_spaces(&mut swarm.behaviour_mut().kademlia, &my_spaces);
                    for space_id in &my_spaces {
                        crate::discovery::query_space_peers(
                            &mut swarm.behaviour_mut().kademlia,
                            space_id,
                        );
                    }
                }
            }

            _ = reconnect_tick.tick() => {
                // Re-dial saved peers whenever we have no active connections.
                if connected_peers.is_empty() && !bootstrap_peer_addrs.is_empty() {
                    eprintln!("NET: no connected peers, re-dialing {} saved peer(s)", bootstrap_peer_addrs.len());
                    for addr_str in &bootstrap_peer_addrs {
                        if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                            let _ = swarm.dial(addr);
                        }
                    }
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
    connected_peers: &mut std::collections::HashSet<PeerId>,
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
            // Send Hello only after Identify so the remote peer has our pubkey cached.
            initiate_hello(swarm, storage, my_spaces, identity_bytes, peer_id);
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
            connected_peers.insert(peer_id);
            let _ = event_tx.send(NetEvent::PeerConnected { peer_id: peer_id.to_string() }).await;
            // Hello is sent from Identify::Received so the remote has our pubkey cached by then.
        }

        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            connected_peers.remove(&peer_id);
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
            use crate::sync_protocol::{SyncRequest, SyncResponse};
            use automerge::sync::SyncDoc;
            // HelloAck must be handled here where we have &mut swarm to send follow-up BoardSync requests
            if let SyncResponse::HelloAck { space_id, board_ids: their_board_ids, space_doc_bytes } = response {
                eprintln!("HELLOACK from {peer}: space={space_id:.8} their_boards={}", their_board_ids.len());
                let our_board_ids = {
                    let mut guard = storage.lock().unwrap();
                    // Merge peer's space doc — updates members, boards, name in SQL.
                    if !space_doc_bytes.is_empty() {
                        merge_space_doc(&space_id, &space_doc_bytes, &mut guard);
                    }
                    // Register any boards the peer has but we don't yet know about.
                    for board_id in &their_board_ids {
                        let _ = kanban_storage::space::add_board(guard.conn(), &space_id, board_id);
                    }
                    guard.list_board_ids().unwrap_or_default()
                };
                let all_boards: std::collections::HashSet<String> =
                    our_board_ids.into_iter().chain(their_board_ids).collect();
                eprintln!("HELLOACK: syncing {} boards total", all_boards.len());
                for board_id in all_boards {
                    // Use "i/" prefix to distinguish initiator state from responder state.
                    // Both peers may simultaneously initiate sync for the same board, and
                    // sharing one state object between the two roles corrupts the protocol.
                    let sync_state = sync_states
                        .entry(format!("i/{peer}/{board_id}"))
                        .or_insert_with(automerge::sync::State::new);
                    let msg_bytes = {
                        let guard = storage.lock().unwrap();
                        let mut doc = match guard.load_board(&board_id) {
                            Ok(d) => d,
                            Err(_) => automerge::AutoCommit::new(),
                        };
                        let msg = doc.sync().generate_sync_message(sync_state);
                        msg.map(|m| m.encode())
                    };
                    eprintln!("HELLOACK board {board_id:.8}: sending_sync={}", msg_bytes.is_some());
                    if let Some(bytes) = msg_bytes {
                        swarm.behaviour_mut().sync.send_request(
                            &peer,
                            SyncRequest::BoardSync { board_id, sync_message: bytes },
                        );
                    }
                }
            } else if let SyncResponse::BoardSync { board_id, sync_message } = response {
                // Multi-round sync: process the incoming message, then send follow-up if needed.
                // handle_sync_response lacks &mut swarm, so we handle BoardSync here.
                let sync_key = format!("i/{peer}/{board_id}");
                let sync_state = sync_states.entry(sync_key).or_insert_with(automerge::sync::State::new);
                eprintln!("RESP_BOARDSYNC[{board_id:.8}]: has_msg={}", sync_message.is_some());
                match sync_message {
                    None => {
                        // Peer has converged on this board
                        eprintln!("RESP_BOARDSYNC[{board_id:.8}]: peer converged → BoardSynced");
                        let _ = event_tx.send(NetEvent::BoardSynced {
                            board_id,
                            peer_id: peer.to_string(),
                        }).await;
                    }
                    Some(msg_bytes) => {
                        eprintln!("RESP_BOARDSYNC[{board_id:.8}]: calling process_incoming_sync");
                        match process_incoming_sync(&board_id, &msg_bytes, storage, sync_state) {
                            Err(e) => {
                                eprintln!("RESP_BOARDSYNC[{board_id:.8}]: ERROR: {e}");
                                let _ = event_tx.send(NetEvent::SyncError {
                                    board_id,
                                    error: e.to_string(),
                                }).await;
                            }
                            Ok(Some(reply_bytes)) => {
                                // Send follow-up round (contains our changes the peer is missing)
                                eprintln!("RESP_BOARDSYNC[{board_id:.8}]: sending follow-up round ({} bytes)", reply_bytes.len());
                                swarm.behaviour_mut().sync.send_request(
                                    &peer,
                                    SyncRequest::BoardSync {
                                        board_id: board_id.clone(),
                                        sync_message: reply_bytes,
                                    },
                                );
                            }
                            Ok(None) => {
                                // We've converged — nothing more to send
                                eprintln!("RESP_BOARDSYNC[{board_id:.8}]: converged → BoardSynced");
                                let _ = event_tx.send(NetEvent::BoardSynced {
                                    board_id,
                                    peer_id: peer.to_string(),
                                }).await;
                            }
                        }
                    }
                }
            } else {
                handle_sync_response(response, peer, storage, event_tx, sync_states).await;
            }
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
        let (board_ids, space_doc_bytes) = {
            let guard = storage.lock().unwrap();
            let boards = kanban_storage::space::get_space_boards(guard.conn(), space_id)
                .unwrap_or_default();
            let doc_bytes = kanban_storage::space::load_space_doc(guard.conn(), space_id)
                .unwrap_or_default();
            (boards, doc_bytes)
        };

        let identity = Identity::from_secret_bytes(&identity_bytes);
        let signature = identity.sign(space_id.as_bytes());

        swarm.behaviour_mut().sync.send_request(
            &peer_id,
            SyncRequest::Hello {
                space_id: space_id.clone(),
                board_ids,
                signature,
                space_doc_bytes,
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
        SyncRequest::Hello { space_id, board_ids: _their_board_ids, signature, space_doc_bytes } => {
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

            // Merge peer's space doc (members, boards, name) into ours, then reply with merged.
            let (my_board_ids, my_space_doc_bytes) = {
                let mut guard = storage.lock().unwrap();
                if !space_doc_bytes.is_empty() {
                    merge_space_doc(&space_id, &space_doc_bytes, &mut guard);
                }
                let boards = kanban_storage::space::get_space_boards(guard.conn(), &space_id)
                    .unwrap_or_default();
                let doc_bytes = kanban_storage::space::load_space_doc(guard.conn(), &space_id)
                    .unwrap_or_default();
                (boards, doc_bytes)
            };
            SyncResponse::HelloAck {
                space_id,
                board_ids: my_board_ids,
                space_doc_bytes: my_space_doc_bytes,
            }
        }

        SyncRequest::BoardSync { board_id, sync_message } => {
            // Use "r/" prefix to keep responder state separate from initiator state.
            let sync_state = sync_states.entry(format!("r/{peer}/{board_id}")).or_insert_with(automerge::sync::State::new);
            eprintln!("REQ_BOARDSYNC[{board_id:.8}] from {peer}: msg_len={}", sync_message.len());
            match process_incoming_sync(&board_id, &sync_message, storage, sync_state) {
                Ok(reply_msg) => {
                    eprintln!("REQ_BOARDSYNC[{board_id:.8}]: reply_is_some={}", reply_msg.is_some());
                    SyncResponse::BoardSync { board_id, sync_message: reply_msg }
                }
                Err(e) => {
                    eprintln!("REQ_BOARDSYNC[{board_id:.8}]: ERROR → Rejected: {e}");
                    SyncResponse::Rejected { reason: e.to_string() }
                }
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
    use automerge::{AutoCommit, ReadDoc, sync as am_sync};
    use am_sync::SyncDoc;

    let msg = am_sync::Message::decode(sync_message)
        .map_err(|e| crate::NetError::Sync(e.to_string()))?;

    // Hold the lock across load → apply → save to prevent a concurrent sync
    // from loading a stale snapshot and overwriting a more-recent save.
    let mut guard = storage.lock().unwrap();
    let mut doc = match guard.load_board(board_id) {
        Ok(d) => d,
        // Unknown board: start from a truly empty doc so the peer's full
        // history merges in without conflicting put_object operations.
        Err(_) => AutoCommit::new(),
    };

    let heads_before = doc.get_heads();
    let doc_bytes_before = doc.save().len();

    doc.sync().receive_sync_message(sync_state, msg)
        .map_err(|e| crate::NetError::Sync(e.to_string()))?;

    let heads_after = doc.get_heads();
    let doc_bytes_after = doc.save().len();
    eprintln!(
        "SYNC[{board_id:.8}]: heads_changed={} bytes_before={doc_bytes_before} bytes_after={doc_bytes_after}",
        heads_before != heads_after
    );

    // Always persist after receiving a sync message so the card_number_index
    // stays consistent with the doc state. If heads_changed=false the doc is
    // unchanged but the index may be stale from a prior run, so rebuild it.
    guard.save_board(board_id, &mut doc)
        .map_err(crate::NetError::Storage)?;
    eprintln!("SYNC[{board_id:.8}]: saved board ({doc_bytes_after} bytes, heads_changed={})", heads_before != heads_after);
    drop(guard);

    let reply = doc.sync().generate_sync_message(sync_state);
    eprintln!("SYNC[{board_id:.8}]: reply_is_some={}", reply.is_some());

    Ok(reply.map(|m| m.encode()))
}

/// Merge the peer's Automerge space doc into our local copy, then update the SQL
/// tables (name, space_members, space_boards) to reflect the merged state.
fn merge_space_doc(space_id: &str, peer_doc_bytes: &[u8], guard: &mut Storage) {
    use automerge::AutoCommit;
    use kanban_storage::space as ss;
    use kanban_core::space as cs;

    let our_bytes = match ss::load_space_doc(guard.conn(), space_id) {
        Ok(b) => b,
        Err(_) => return,  // space not in DB, skip
    };

    let mut our_doc = if our_bytes.is_empty() {
        AutoCommit::new()
    } else {
        match AutoCommit::load(&our_bytes) {
            Ok(d) => d,
            Err(e) => { eprintln!("SPACE_SYNC: failed to load our doc: {e}"); return; }
        }
    };

    let mut their_doc = match AutoCommit::load(peer_doc_bytes) {
        Ok(d) => d,
        Err(e) => { eprintln!("SPACE_SYNC: failed to load peer doc: {e}"); return; }
    };

    if let Err(e) = our_doc.merge(&mut their_doc) {
        eprintln!("SPACE_SYNC: merge error: {e}"); return;
    }

    // Save merged doc bytes.
    let merged_bytes = our_doc.save();
    let _ = ss::update_space_doc(guard.conn(), space_id, &merged_bytes);

    // Sync space name.
    if let Some(name) = cs::get_space_name(&our_doc) {
        let _ = ss::rename_space(guard.conn(), space_id, &name);
    }

    // Sync members.
    if let Ok(members) = cs::list_members(&our_doc) {
        for m in members {
            let _ = ss::upsert_member(guard.conn(), space_id, &kanban_core::space::Member {
                pubkey: m.pubkey,
                display_name: m.display_name,
                avatar_blob: m.avatar_blob,
                kicked: m.kicked,
            });
        }
    }

    // Sync boards (add any boards referenced in the space doc but not yet in space_boards).
    if let Ok(board_refs) = cs::list_board_refs(&our_doc) {
        for board_id in board_refs {
            let _ = ss::add_board(guard.conn(), space_id, &board_id);
        }
    }

    eprintln!("SPACE_SYNC[{space_id:.8}]: merged and saved");
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
        SyncResponse::HelloAck { space_id: _, board_ids: their_board_ids, .. } => {
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
            eprintln!("REJECTED by {peer}: {reason}");
            tracing::warn!("net: rejected by {}: {reason}", peer);
        }
    }
}
