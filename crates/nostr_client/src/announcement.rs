use std::net::SocketAddr;

use nostr_sdk::{Client, EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use crate::ServerIdentity;

pub const NOSTR_KIND_SERVER_ANNOUNCEMENT: u16 = 30078;
pub const SERVER_ANNOUNCEMENT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerAnnouncement {
    pub server_addr: SocketAddr,
    pub cert_digest: String,
    pub display_name: String,
    pub version: u32,
}

pub fn server_announcement_builder(
    announcement: &ServerAnnouncement,
) -> Result<EventBuilder, serde_json::Error> {
    let content = serde_json::to_string(announcement)?;
    Ok(
        EventBuilder::new(Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT.into()), content)
            .tag(Tag::identifier("server")),
    )
}

pub async fn publish_server_announcement(
    client: Client,
    identity: ServerIdentity,
    announcement: ServerAnnouncement,
) -> Result<String, String> {
    let event = server_announcement_builder(&announcement)
        .map_err(|error| format!("serialize announcement: {error}"))?
        .sign_with_keys(&identity.keys)
        .map_err(|error| format!("sign announcement: {error}"))?;
    let output = client
        .send_event(&event)
        .await
        .map_err(|error| format!("publish announcement: {error}"))?;
    if output.success.is_empty() {
        return Err(format!(
            "publish announcement: no relay accepted event {}; failures={:?}",
            event.id, output.failed
        ));
    }
    Ok(event.id.to_string())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use nostr_sdk::{Keys, SecretKey};

    use super::*;

    fn announcement() -> ServerAnnouncement {
        ServerAnnouncement {
            server_addr: SocketAddr::from((IpAddr::V4(Ipv4Addr::LOCALHOST), 5001)),
            cert_digest: "F03BA2AB0904DA5DEA7D3F5952FADED758248DF23306F0EF59296B7D0C25A016"
                .to_string(),
            display_name: "Test Server".to_string(),
            version: SERVER_ANNOUNCEMENT_VERSION,
        }
    }

    #[test]
    fn announcement_builder_signs_kind_content_and_identifier() {
        let keys = Keys::new(SecretKey::generate());
        let event = server_announcement_builder(&announcement())
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(
            event.kind,
            Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT.into())
        );
        assert!(event
            .tags
            .identifier()
            .is_some_and(|value| value == "server"));

        let content: ServerAnnouncement = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content.server_addr, announcement().server_addr);
        assert_eq!(content.cert_digest, announcement().cert_digest);
        assert_eq!(content.display_name, announcement().display_name);
        assert_eq!(content.version, SERVER_ANNOUNCEMENT_VERSION);
    }
}
