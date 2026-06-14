use rkvm_net::mdns::{Mdns, MdnsListenOptions, MdnsOptions};
use rustkvm::cli::Cli;
use rustkvm::hardware::native::socket as native_ctrl;
use rustkvm::hardware::usb::storage as virtual_media;
use rustkvm::hardware::{display, hw, jiggler, tuning, usb};
use rustkvm::{cloud, config, failsafe, mqtt, network, observability, tls, video, web, webrtc};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{Level, error, info, warn};

static MDNS: once_cell::sync::OnceCell<Mdns> = once_cell::sync::OnceCell::new();

async fn init_mdns(hostname: &str) -> anyhow::Result<()> {
    let fqdn = format!("{}.local", hostname);

    let mdns = Mdns::new(MdnsOptions {
        local_names: vec![hostname.to_string(), fqdn],
        listen_options: MdnsListenOptions { ipv4: true, ipv6: true },
    })?;

    MDNS.set(mdns).map_err(|_| anyhow::anyhow!("Failed to initialize mDNS"))?;

    Ok(())
}

fn init_tracing(cli: &Cli) {
    use tracing_subscriber::filter::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let log_level = cli.logging.log_level.parse::<Level>().unwrap_or(Level::INFO);
    let default_directive =
        format!("rustkvm={lvl},rkvm_core={lvl},rkvm_net={lvl},info", lvl = log_level);
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&default_directive))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
    rustkvm::observability::install_log_reload_handle(reload_handle);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_ansi(!cli.logging.log_no_color)
        .without_time()
        .with_level(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true);

    if cli.logging.log_json {
        tracing_subscriber::registry().with(filter).with(fmt_layer.json()).init();
    } else {
        tracing_subscriber::registry().with(filter).with(fmt_layer.pretty()).init();
    }
}

fn main() -> anyhow::Result<()> {
    // SAFETY: this runs before the tokio runtime is built below, i.e. while the
    // process is still single-threaded, so there is no concurrent reader of the
    // environment and `set_var` cannot race. GStreamer reads these on its worker
    // threads later, long after they are set. (Done before `parse_args`/`dotenv`,
    // neither of which spawns threads, to keep the single-threaded guarantee.)
    unsafe {
        std::env::set_var("GST_VIDEO_CONVERT_USE_RGA", "1");
        std::env::set_var("GST_VIDEO_FLIP_USE_RGA", "1");
    }

    let cli = Cli::parse_args();
    dotenvy::dotenv().ok();
    init_tracing(&cli);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build tokio runtime: {e}"))?
        .block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> anyhow::Result<()> {
    cli.validate()?;

    cli.print_summary();

    if cli.dry_run {
        info!("Dry-run mode: configuration valid, exiting");
        return Ok(());
    }

    failsafe::check_failsafe_reason();
    if failsafe::is_active() {
        if let Some(state) = failsafe::get_state() {
            warn!(reason = %state.reason, "failsafe mode activated");
        }
    }

    tuning::apply_rk3588_tuning().await;

    if let Err(e) = rkvm_core::time_sync::sync_time_once().await {
        warn!("Time sync failed (non-fatal if clock already correct): {}", e);
    }
    rkvm_core::time_sync::spawn_periodic_resync(tokio::time::Duration::from_secs(3600));

    config::init_config().await?;

    if let Err(e) = network::init_network(&cli.network.mdns_hostname).await {
        warn!("Failed to apply system hostname: {}", e);
    }
    network::spawn_state_monitor(tokio::time::Duration::from_secs(30));

    let path_warnings = config::validate_hardware_paths().await;
    for w in &path_warnings {
        warn!("Hardware warning: {}", w);
    }
    if !path_warnings.is_empty() {
        info!("{} hardware path issues detected (non-fatal)", path_warnings.len());
    }
    let wd_cancel = CancellationToken::new();
    let wd_task = {
        let token = wd_cancel.clone();
        tokio::spawn(async move {
            if let Err(e) = hw::run_watchdog(token).await {
                warn!("Watchdog task exited with error: {}", e);
            }
        })
    };

    tls::init().await?;
    observability::init_prometheus();
    if let Err(e) = native_ctrl::init_ctrl_socket() {
        warn!("Failed to initialize ctrl socket: {}", e);
    }
    webrtc::init_webrtc_api().await?;

    let _usb = usb::init_usb();

    display::init_display().await?;

    video::init_video_state_updater().await?;

    info!("Initializing native video pipeline with CLI configuration...");
    if let Err(e) = video::start_native_video_with_cli(&cli).await {
        warn!("Failed to start native video pipeline: {}", e);
    } else {
        info!("Native video pipeline started successfully");

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if let Ok(stats) = video::get_video_stats().await {
                    info!(
                        ready = stats.ready,
                        width = stats.width,
                        height = stats.height,
                        fps = stats.fps,
                        has_sink = stats.has_sink,
                        "video stats"
                    );
                }
                let metrics = video::video_metrics_snapshot();
                if !metrics.trim().is_empty() {
                    tracing::debug!("Prometheus metrics:\n{}", metrics);
                }
            }
        });
    }

    if let Err(e) = virtual_media::set_initial_virtual_media_state().await {
        warn!("Failed to set initial virtual media state: {}", e);
    }

    if let Err(e) = virtual_media::ensure_images_folder().await {
        warn!("Failed to ensure images folder: {}", e);
    }

    jiggler::run_jiggler_cron_tab().await;

    tokio::spawn(mqtt::start_mqtt());

    if !cli.network.mdns_disable {
        init_mdns(&cli.network.mdns_hostname).await?;
    }

    let web_handle = web::init().await?;

    if !cli.network.cloud_disable {
        tokio::spawn(async {
            let cloud_manager = cloud::manager::get_cloud_manager();
            if let Err(e) = cloud_manager.start_connection_loop().await {
                error!("Cloud connection loop failed: {}", e);
            }
        });
    }

    info!("RustKVM system initialized successfully. Waiting for shutdown signal...");

    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
        .map_err(|e| anyhow::anyhow!("failed to create SIGTERM signal handler: {e}"))?;

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

    info!("Starting graceful shutdown...");

    wd_cancel.cancel();
    let _ = wd_task.await;
    if let Err(e) = hw::disarm_watchdog() {
        warn!("Failed to disarm watchdog (extra safety attempt): {}", e);
    }

    video::shutdown_video_pipeline().await;

    info!("Shutdown complete");

    // The ctrl-socket accept thread and GStreamer's glib thread pool are not tokio
    // tasks, so they are never joined; dropping the multi-thread runtime would also
    // block until any in-flight `spawn_blocking` work finishes. The watchdog is
    // already disarmed and all critical state is flushed, so terminate immediately
    // rather than risk hanging teardown — a half-dead lingering process previously
    // tripped the hardware watchdog on the next launch.
    std::process::exit(0)
}
