use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error")]
    Serialization,
    #[error("unknown message type: {0:#x}")]
    UnknownMessageType(u8),
    #[error("empty message")]
    EmptyMessage,
    #[error("invalid key")]
    InvalidKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("handshake io error")]
    HandshakeIo,
    #[error("unexpected message type during handshake")]
    UnexpectedMessageType,
    #[error("incompatible peer version: {0}")]
    IncompatibleVersion(String),
}

// Placeholder — real implementation in Phase 2 of base plan
pub struct NetworkHandle;

impl NetworkHandle {
    pub fn new() -> Self { Self }
}

impl Default for NetworkHandle {
    fn default() -> Self { Self::new() }
}
