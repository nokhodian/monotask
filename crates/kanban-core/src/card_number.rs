use std::str::FromStr;
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CardNumber {
    pub prefix: String,
    pub seq: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CardNumberError {
    #[error("invalid card number format; expected <prefix>-<integer>, e.g. 'a7f3-1'")]
    InvalidFormat,
}

impl CardNumber {
    pub fn new(prefix: impl Into<String>, seq: u64) -> Self {
        Self { prefix: prefix.into(), seq }
    }

    pub fn to_display(&self) -> String {
        format!("{}-{}", self.prefix, self.seq)
    }
}

impl FromStr for CardNumber {
    type Err = CardNumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let re = Regex::new(r"^([a-z0-9]{4,8})-(\d+)$").unwrap();
        let caps = re.captures(s).ok_or(CardNumberError::InvalidFormat)?;
        let prefix = caps[1].to_string();
        let seq: u64 = caps[2].parse().map_err(|_| CardNumberError::InvalidFormat)?;
        Ok(CardNumber { prefix, seq })
    }
}

/// Derive the actor's display prefix for a given board.
/// Uses 4 chars normally; extends to 8 if another board member shares the same 4-char prefix.
pub fn actor_prefix(pubkey_bytes: &[u8], all_member_pubkeys: &[Vec<u8>]) -> String {
    let encoded = base32::encode(
        base32::Alphabet::RFC4648 { padding: false },
        pubkey_bytes,
    )
    .to_lowercase();

    let prefix4 = &encoded[..4];
    let collision = all_member_pubkeys
        .iter()
        .any(|other_pk| {
            let other_encoded = base32::encode(
                base32::Alphabet::RFC4648 { padding: false },
                other_pk,
            )
            .to_lowercase();
            other_encoded.starts_with(prefix4)
        });

    if collision {
        encoded[..8].to_string()
    } else {
        prefix4.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_card_number() {
        let n: CardNumber = "a7f3-42".parse().unwrap();
        assert_eq!(n.prefix, "a7f3");
        assert_eq!(n.seq, 42);
    }

    #[test]
    fn roundtrip_display() {
        let n = CardNumber::new("a7f3", 1);
        let s = n.to_display();
        let parsed: CardNumber = s.parse().unwrap();
        assert_eq!(parsed, n);
    }

    #[test]
    fn reject_invalid_format() {
        assert!("42".parse::<CardNumber>().is_err());
        assert!("toolongprefix-1".parse::<CardNumber>().is_err());
        assert!("a7f3-".parse::<CardNumber>().is_err());
    }

    #[test]
    fn actor_prefix_no_collision() {
        let pk = vec![1u8; 32];
        let others = vec![vec![2u8; 32]];
        let p = actor_prefix(&pk, &others);
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn actor_prefix_collision_extends_to_8() {
        // All-zero bytes base32-encode to "AAAA..." so both keys share the same 4-char prefix
        let pk = vec![0u8; 32];
        let other = vec![0u8; 32]; // same prefix
        let p = actor_prefix(&pk, &[other]);
        assert_eq!(p.len(), 8);
    }
}
