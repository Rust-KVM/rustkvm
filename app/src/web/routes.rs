use anyhow::Result;
use salvo::http::header::{CACHE_CONTROL, HeaderValue};
use salvo::prelude::*;
use salvo::serve_static::{StaticDir, static_embed};

use super::{auth, device, pprof, storage, webrtc_handlers};
use crate::assets::ClientAssets;
use crate::middleware::{auth_middleware, developer_auth_middleware, public_middleware};

const CACHE_IMMUTABLE_MAX_AGE: u32 = 365 * 24 * 60 * 60;
const CACHE_SHORT_MAX_AGE: u32 = 5 * 60;
const CACHABLE_EXTENSIONS: &[&str] =
    &[".jpg", ".jpeg", ".png", ".svg", ".gif", ".webp", ".ico", ".woff2"];

pub(super) async fn init_public_routes() -> Result<Router> {
    let router = Router::new()
        .hoop(public_middleware)
        .push(Router::with_path("/auth/login-local").post(auth::handle_login_local))
        .push(Router::with_path("/device/status").get(device::handle_device_status))
        .push(Router::with_path("/device/setup").post(device::handle_device_setup))
        .push(Router::with_path("/metrics").get(storage::handle_metrics));
    Ok(router)
}

pub(super) async fn init_protected_routes() -> Result<Router> {
    let router = Router::new()
        .hoop(auth_middleware)
        .push(Router::with_path("/webrtc/session").post(webrtc_handlers::handle_webrtc_session))
        .push(
            Router::with_path("/webrtc/signaling/client")
                .get(webrtc_handlers::handle_webrtc_signaling_client),
        )
        .push(Router::with_path("/cloud/register").post(device::handle_cloud_register))
        .push(Router::with_path("/cloud/state").get(device::handle_cloud_status))
        .push(Router::with_path("/device").get(device::handle_device))
        .push(Router::with_path("/auth/logout").post(auth::handle_logout))
        .push(Router::with_path("/auth/password-local").post(auth::create_password_local))
        .push(Router::with_path("/auth/password-local").put(auth::modify_password_local))
        .push(Router::with_path("/auth/local-password").delete(auth::disable_local_password))
        .push(Router::with_path("/storage/upload").post(storage::handle_storage_upload))
        .push(Router::with_path("/device/send-wol/{mac_addr}").post(storage::handle_send_wol))
        .push(Router::with_path("/diagnostics").get(storage::handle_diagnostics_download));
    Ok(router)
}

pub(super) async fn init_developer_routes() -> Result<Router> {
    let router = Router::new()
        .hoop(developer_auth_middleware)
        .push(Router::with_path("/developer/pprof").get(pprof::handle_pprof_index))
        .push(Router::with_path("/developer/pprof/cmdline").get(pprof::handle_pprof_cmdline))
        .push(Router::with_path("/developer/pprof/profile").get(pprof::handle_pprof_profile))
        .push(Router::with_path("/developer/pprof/symbol").get(pprof::handle_pprof_symbol))
        .push(Router::with_path("/developer/pprof/symbol").post(pprof::handle_pprof_create_symbol))
        .push(Router::with_path("/developer/pprof/trace").get(pprof::handle_pprof_trace))
        .push(Router::with_path("/developer/pprof/allocs").get(pprof::handle_pprof_allocs))
        .push(Router::with_path("/developer/pprof/block").get(pprof::handle_pprof_block))
        .push(Router::with_path("/developer/pprof/goroutine").get(pprof::handle_pprof_goroutine))
        .push(Router::with_path("/developer/pprof/heap").get(pprof::handle_pprof_heap))
        .push(Router::with_path("/developer/pprof/mutex").get(pprof::handle_pprof_mutex))
        .push(
            Router::with_path("/developer/pprof/threadcreate")
                .get(pprof::handle_pprof_threadcreate),
        );
    Ok(router)
}

pub(super) async fn init_static_routes() -> Result<Router> {
    let compression = Compression::new().enable_gzip(CompressionLevel::Fastest);

    let router = if let Some(serve_dir) = option_env!("RUSTKVM_SERVE_DIR") {
        Router::new()
            .push(
                Router::with_path("/static/{*path}")
                    .hoop(compression.clone())
                    .hoop(static_cache_control_hoop)
                    .get(StaticDir::new(serve_dir)),
            )
            .push(
                Router::with_path("{*path}")
                    .hoop(compression)
                    .get(StaticDir::new(serve_dir).defaults("index.html")),
            )
    } else {
        Router::new()
            .push(
                Router::with_path("/static/{*path}")
                    .hoop(compression.clone())
                    .hoop(static_cache_control_hoop)
                    .get(static_embed::<ClientAssets>()),
            )
            .push(
                Router::with_path("{*path}")
                    .hoop(compression)
                    .get(static_embed::<ClientAssets>().fallback("index.html")),
            )
    };
    Ok(router)
}

#[handler]
async fn static_cache_control_hoop(req: &mut Request, res: &mut Response) {
    let path = req.uri().path();
    let value: Option<HeaderValue> = if path.starts_with("/static/assets/immutable/") {
        HeaderValue::from_str(&format!("public, max-age={CACHE_IMMUTABLE_MAX_AGE}, immutable")).ok()
    } else if path.starts_with("/static/")
        && CACHABLE_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
    {
        HeaderValue::from_str(&format!("public, max-age={CACHE_SHORT_MAX_AGE}")).ok()
    } else {
        None
    };
    if let Some(v) = value {
        res.headers_mut().insert(CACHE_CONTROL, v);
    }
}

#[handler]
pub(super) async fn security_headers_hoop(res: &mut Response) {
    let h = res.headers_mut();
    h.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    h.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    h.insert("X-XSS-Protection", HeaderValue::from_static("1; mode=block"));
}
