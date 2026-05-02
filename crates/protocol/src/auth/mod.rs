use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
#[type_path = "protocol::auth"]
pub struct NostrPublicKey(pub [u8; 32]);

impl NostrPublicKey {
    pub fn client_id_prefix(self) -> u64 {
        u64::from_le_bytes(
            self.0[0..8]
                .try_into()
                .expect("NostrPublicKey has 32 bytes"),
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Message)]
pub struct IdentityChallenge {
    pub nonce: [u8; 32],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Message)]
pub struct IdentityProof {
    pub pubkey: NostrPublicKey,
    pub signed_event_json: String,
}

pub struct AuthChannel;

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[type_path = "protocol::auth"]
pub struct PlayerIdentity(pub NostrPublicKey);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_prefix_uses_first_eight_bytes_little_endian() {
        let public_key = NostrPublicKey([
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff,
        ]);

        assert_eq!(public_key.client_id_prefix(), 0x0102_0304_0506_0708);
    }
}
