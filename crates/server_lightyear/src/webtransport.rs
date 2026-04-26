use async_compat::Compat;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use lightyear::prelude::server::WebTransportServerIo;
use lightyear::webtransport::prelude::Identity;
use std::path::Path;

use crate::connection::ServerNetworkConfig;

pub(crate) fn build_io(config: &ServerNetworkConfig) -> WebTransportServerIo {
    let certificate = load_identity(&config.cert_pem_path, &config.key_pem_path);
    let digest = certificate.certificate_chain().as_slice()[0].hash();
    info!("WebTransport certificate digest: {}", digest);
    WebTransportServerIo { certificate }
}

fn load_identity(cert_pem: &Path, key_pem: &Path) -> Identity {
    let cert = cert_pem.to_path_buf();
    let key = key_pem.to_path_buf();
    IoTaskPool::get()
        .scope(|s| {
            s.spawn(Compat::new(async move {
                Identity::load_pemfiles(&cert, &key)
                    .await
                    .expect("Failed to load WebTransport certificates")
            }));
        })
        .pop()
        .unwrap()
}
