use std::io;
use libp2p::request_response;
use libp2p::swarm::StreamProtocol;
use libp2p::futures;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_NAME: &str = "/monotask/board-sync/1.0.0";

const MAX_MSG_SIZE: u32 = 10 * 1024 * 1024; // 10 MB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncRequest {
    /// Prove Space membership and share which boards this peer has.
    Hello {
        space_id: String,
        board_ids: Vec<String>,
        /// Ed25519 signature over `space_id.as_bytes()`.
        signature: Vec<u8>,
        /// Automerge-encoded space doc (members, boards, name). Empty = not available.
        #[serde(default)]
        space_doc_bytes: Vec<u8>,
    },
    /// One round of Automerge sync for a board.
    BoardSync {
        board_id: String,
        /// `automerge::sync::Message::encode()` output.
        sync_message: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncResponse {
    /// Hello accepted; here are the responder's board IDs in this Space.
    HelloAck {
        space_id: String,
        board_ids: Vec<String>,
        /// Automerge-encoded space doc (members, boards, name). Empty = not available.
        #[serde(default)]
        space_doc_bytes: Vec<u8>,
    },
    /// One round of Automerge sync in reply. `None` = this side has converged.
    BoardSync {
        board_id: String,
        sync_message: Option<Vec<u8>>,
    },
    /// Rejected: not in same Space, bad signature, or member is kicked.
    Rejected { reason: String },
}

/// CBOR codec for request_response::Behaviour.
#[derive(Debug, Clone, Default)]
pub struct MonotaskCodec;

#[async_trait::async_trait]
impl request_response::Codec for MonotaskCodec {
    type Protocol = StreamProtocol;
    type Request  = SyncRequest;
    type Response = SyncResponse;

    async fn read_request<T>(&mut self, _: &StreamProtocol, io: &mut T)
        -> io::Result<SyncRequest>
    where T: futures::AsyncRead + Unpin + Send
    {
        use futures::AsyncReadExt;
        let len = read_u32(io).await?;
        if len > MAX_MSG_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message too large: {} bytes (max {})", len, MAX_MSG_SIZE),
            ));
        }
        let mut buf = vec![0u8; len as usize];
        io.read_exact(&mut buf).await?;
        ciborium::from_reader(buf.as_slice())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    async fn read_response<T>(&mut self, _: &StreamProtocol, io: &mut T)
        -> io::Result<SyncResponse>
    where T: futures::AsyncRead + Unpin + Send
    {
        use futures::AsyncReadExt;
        let len = read_u32(io).await?;
        if len > MAX_MSG_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message too large: {} bytes (max {})", len, MAX_MSG_SIZE),
            ));
        }
        let mut buf = vec![0u8; len as usize];
        io.read_exact(&mut buf).await?;
        ciborium::from_reader(buf.as_slice())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    async fn write_request<T>(&mut self, _: &StreamProtocol, io: &mut T, req: SyncRequest)
        -> io::Result<()>
    where T: futures::AsyncWrite + Unpin + Send
    {
        use futures::AsyncWriteExt;
        let mut buf = Vec::new();
        ciborium::into_writer(&req, &mut buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        write_u32(io, buf.len() as u32).await?;
        io.write_all(&buf).await
    }

    async fn write_response<T>(&mut self, _: &StreamProtocol, io: &mut T, res: SyncResponse)
        -> io::Result<()>
    where T: futures::AsyncWrite + Unpin + Send
    {
        use futures::AsyncWriteExt;
        let mut buf = Vec::new();
        ciborium::into_writer(&res, &mut buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        write_u32(io, buf.len() as u32).await?;
        io.write_all(&buf).await
    }
}

async fn read_u32<T: futures::AsyncRead + Unpin>(io: &mut T) -> io::Result<u32> {
    use futures::AsyncReadExt;
    let mut buf = [0u8; 4];
    io.read_exact(&mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}

async fn write_u32<T: futures::AsyncWrite + Unpin>(io: &mut T, v: u32) -> io::Result<()> {
    use futures::AsyncWriteExt;
    io.write_all(&v.to_be_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cbor_roundtrip_request(req: SyncRequest) -> SyncRequest {
        let mut buf = Vec::new();
        ciborium::into_writer(&req, &mut buf).unwrap();
        ciborium::from_reader(buf.as_slice()).unwrap()
    }

    fn cbor_roundtrip_response(res: SyncResponse) -> SyncResponse {
        let mut buf = Vec::new();
        ciborium::into_writer(&res, &mut buf).unwrap();
        ciborium::from_reader(buf.as_slice()).unwrap()
    }

    #[test]
    fn serialize_hello_roundtrip() {
        let req = SyncRequest::Hello {
            space_id: "space-1".into(),
            board_ids: vec!["b1".into(), "b2".into()],
            signature: vec![1, 2, 3],
            space_doc_bytes: vec![],
        };
        let SyncRequest::Hello { space_id, board_ids, signature, .. } = cbor_roundtrip_request(req)
            else { panic!("wrong variant") };
        assert_eq!(space_id, "space-1");
        assert_eq!(board_ids, vec!["b1", "b2"]);
        assert_eq!(signature, vec![1, 2, 3]);
    }

    #[test]
    fn serialize_board_sync_roundtrip() {
        let req = SyncRequest::BoardSync {
            board_id: "b1".into(),
            sync_message: vec![0xDE, 0xAD],
        };
        let SyncRequest::BoardSync { board_id, sync_message } = cbor_roundtrip_request(req)
            else { panic!("wrong variant") };
        assert_eq!(board_id, "b1");
        assert_eq!(sync_message, vec![0xDE, 0xAD]);
    }

    #[test]
    fn serialize_hello_ack_roundtrip() {
        let res = SyncResponse::HelloAck {
            space_id: "s1".into(),
            board_ids: vec!["x".into()],
            space_doc_bytes: vec![],
        };
        let SyncResponse::HelloAck { space_id, board_ids, .. } = cbor_roundtrip_response(res)
            else { panic!() };
        assert_eq!(space_id, "s1");
        assert_eq!(board_ids, vec!["x"]);
    }

    #[test]
    fn serialize_rejected_roundtrip() {
        let res = SyncResponse::Rejected { reason: "kicked".into() };
        let SyncResponse::Rejected { reason } = cbor_roundtrip_response(res)
            else { panic!() };
        assert_eq!(reason, "kicked");
    }
}
