use std::sync::{Arc, OnceLock};

use anyhow::Result;
use salvo::prelude::*;
use tokio::task::JoinHandle;
use tracing::info;

use crate::config::get_config_manager;
use crate::state::AppState;
use crate::tls;

mod auth;
mod device;
mod pprof;
mod ratelimit;
mod routes;
mod socket;
mod storage;
mod types;
mod webrtc_handlers;

pub(super) mod sio {
    pub use socketioxide::SocketIo;
    pub use socketioxide::extract::{AckSender, Data, SocketRef, State};
}

static GLOBAL_APP_STATE: OnceLock<Arc<AppState>> = OnceLock::new();

pub fn get_global_app_state() -> &'static Arc<AppState> {
    GLOBAL_APP_STATE.get().expect("Global app state not initialized")
}

pub(super) fn global_app_state() -> Option<Arc<AppState>> {
    GLOBAL_APP_STATE.get().cloned()
}

pub async fn init() -> Result<JoinHandle<()>> {
    let app_state = Arc::new(AppState::new());

    GLOBAL_APP_STATE
        .set(app_state.clone())
        .map_err(|_| anyhow::anyhow!("Failed to initialize global AppState"))?;

    let (layer, io) = sio::SocketIo::builder().with_state(app_state.clone()).build_layer();

    io.ns("/", socket::handle_session_connect);

    let app = Router::new()
        .hoop(routes::security_headers_hoop)
        .push(Router::with_path("/robots.txt").get(device::handle_robots_txt))
        .push(routes::init_protected_routes().await?)
        .push(routes::init_public_routes().await?)
        .push(routes::init_developer_routes().await?)
        .push(routes::init_static_routes().await?)
        .hoop(layer.compat());

    let doc = OpenApi::new("rustkvm api", "0.0.1").merge_router(&app);

    let app = app
        .unshift(doc.into_router("/api-doc/openapi.json"))
        .unshift(Scalar::new("/api-doc/openapi.json").into_router("/scalar-ui"));

    let local_ip = rkvm_net::local_ip::local_ip();
    let rustls_config = tls::init_rustls_config(local_ip).await?;

    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    let http_bind_address = if config.local_loopback_only { "localhost:80" } else { "0.0.0.0:80" };

    let https_bind_address =
        if config.local_loopback_only { "localhost:443" } else { "0.0.0.0:443" };

    info!(
        "Starting web server - HTTP: {}, HTTPS: {}, loopback_only: {}",
        http_bind_address, https_bind_address, config.local_loopback_only
    );

    let acceptor = TcpListener::new(https_bind_address)
        .rustls(rustls_config)
        .join(TcpListener::new(http_bind_address))
        .bind()
        .await;

    let handle = tokio::spawn(async move {
        Server::new(acceptor).serve(app).await;
    });

    Ok(handle)
}
