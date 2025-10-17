// use std::sync::Arc;

// use rustkvm::hardware::native::process::NativeSupervisor;
// use rustkvm::hardware::native::socket as native_socket;
use rustkvm::hardware::{display, usb};
use rustkvm::mdns::{Mdns, MdnsListenOptions, MdnsOptions};
use rustkvm::{cloud, config, tls, video, web, webrtc};
use tokio::signal;
use tracing::{Level, error, info, warn};

static MDNS: once_cell::sync::OnceCell<Mdns> = once_cell::sync::OnceCell::new();

/// Initialize mDNS service
async fn init_mdns() -> anyhow::Result<()> {
    let hostname = "rustkvm".to_string(); // Should be retrieved from network configuration
    let fqdn = format!("{}.local", hostname);

    let mdns = Mdns::new(MdnsOptions {
        local_names: vec![hostname, fqdn],
        listen_options: MdnsListenOptions { ipv4: true, ipv6: true },
    })?;

    // Do not start immediately, wait for network state to be set
    MDNS.set(mdns).map_err(|_| anyhow::anyhow!("Failed to initialize mDNS"))?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set GStreamer environment variables early
    unsafe {
        std::env::set_var("GST_VIDEO_CONVERT_USE_RGA", "1");
        std::env::set_var("GST_VIDEO_FLIP_USE_RGA", "1");
    }

    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_ansi(true)
        .without_time()
        .with_level(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        // .compact()
        .pretty()
        .init();

    if let Err(e) = rustkvm::time_sync::sync_time_once().await {
        warn!("Time sync failed (non-fatal if clock already correct): {}", e);
    }

    config::init_config().await?;
    tls::init().await?;
    webrtc::init_webrtc_api().await?;

    // Initialize ctrl socket server at /var/run/rustkvm_ctrl.sock
    // native_socket::init_ctrl_socket()?;

    // Initialize USB manager and start USB state/LED event pipeline
    let _usb = usb::init_usb();

    // Initialize display (lvgl rotation, static contents, backlight tickers)
    display::init_display().await?;

    video::init_video_state_updater().await?;

    // Initialize native video pipeline (C encoder -> Rust WebRTC)
    info!("Initializing native video pipeline...");
    if let Err(e) = video::start_native_video(Some(1.0)).await {
        warn!("Failed to start native video pipeline: {}", e);
        // Continue without video - system can still function
    } else {
        info!("Native video pipeline started successfully");

        // Spawn video monitoring task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Ok(stats) = video::get_video_stats().await {
                    info!("Video stats: {:?}", stats);
                }

                let metrics = video::video_metrics_snapshot();
                if !metrics.trim().is_empty() {
                    info!("Prometheus metrics:\n{}", metrics);
                }
            }
        });
    }

    // Start and supervise native binary (lazy: only if binary exists)
    // TODO(native-resources): Auto-extract embedded native binary and write sha256
    // embedded native binary and writing sha256 version file when we introduce
    // resource packaging.
    // let native_sup = Arc::new(NativeSupervisor::new("/userdata/rustkvm/bin/rustkvm_native"));
    // if let Err(e) = native_sup.start_now().await {
    //     tracing::warn!("failed to start native binary: {}", e);
    // }
    // {
    //     let sup = Arc::clone(&native_sup);
    //     tokio::spawn(async move {
    //         sup.supervise().await;
    //     });
    // }

    init_mdns().await?;

    // let local_ip = util::local_ip();
    // let http_addr: SocketAddr = "0.0.0.0:8000".parse()?;
    // let https_addr: SocketAddr = "0.0.0.0:8443".parse()?;
    // info!("Starting http server on http://localhost:8000");
    // info!("Starting https server on https://localhost:8443");

    // let tls_config = tls::init_rustls_config(local_ip).await?;

    let web_handle = web::init().await?;

    // Start cloud connection loop
    tokio::spawn(async {
        let cloud_manager = cloud::manager::get_cloud_manager();
        if let Err(e) = cloud_manager.start_connection_loop().await {
            error!("Cloud connection loop failed: {}", e);
        }
    });

    // Start both HTTP and HTTPS servers concurrently
    // let http_server = axum_server::bind(http_addr).serve(app.clone().into_make_service());
    // let https_server =
    //     axum_server::bind_rustls(https_addr, tls_config).serve(app.into_make_service());

    // tokio::try_join!(http_server, https_server)?;

    info!("RustKVM system initialized successfully. Waiting for shutdown signal...");

    // Wait for shutdown signal
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
        .expect("Failed to create SIGTERM signal handler");

    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, shutting down...");
        }
        _ = web_handle => {
            info!("Web server stopped");
        }
    }

    // Graceful shutdown
    info!("Starting graceful shutdown...");
    video::shutdown_video_pipeline().await;

    info!("Shutdown complete");
    Ok(())
}
