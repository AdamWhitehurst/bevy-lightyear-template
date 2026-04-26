use lightyear::webtransport::client::WebTransportClientIo;

use crate::connection::ClientNetworkConfig;

pub(crate) fn build_io(config: &ClientNetworkConfig) -> WebTransportClientIo {
    WebTransportClientIo {
        certificate_digest: config.certificate_digest.clone(),
    }
}
