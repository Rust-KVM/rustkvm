use once_cell::sync::{Lazy, OnceCell};
use prometheus::{Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, Opts};
use tracing::warn;

static APP_INFO_REGISTERED: OnceCell<()> = OnceCell::new();

pub static VIDEO_FRAMES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::with_opts(Opts::new(
        "rustkvm_video_frames_total",
        "Number of video frames pulled from GStreamer and forwarded to WebRTC",
    ))
    .expect("rustkvm_video_frames_total: invalid metric definition");
    if let Err(e) = prometheus::default_registry().register(Box::new(c.clone())) {
        warn!("failed to register rustkvm_video_frames_total: {e}");
    }
    c
});

pub static AUDIO_FRAMES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::with_opts(Opts::new(
        "rustkvm_audio_frames_total",
        "Number of audio frames forwarded to WebRTC",
    ))
    .expect("rustkvm_audio_frames_total: invalid metric definition");
    if let Err(e) = prometheus::default_registry().register(Box::new(c.clone())) {
        warn!("failed to register rustkvm_audio_frames_total: {e}");
    }
    c
});

pub static RPC_CALLS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("rustkvm_rpc_calls_total", "JSON-RPC method invocations by name"),
        &["method"],
    )
    .expect("rustkvm_rpc_calls_total: invalid metric definition");
    if let Err(e) = prometheus::default_registry().register(Box::new(c.clone())) {
        warn!("failed to register rustkvm_rpc_calls_total: {e}");
    }
    c
});

pub static RPC_LATENCY_SECONDS: Lazy<Histogram> = Lazy::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new("rustkvm_rpc_latency_seconds", "JSON-RPC handler latency")
            .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
    )
    .expect("rustkvm_rpc_latency_seconds: invalid metric definition");
    if let Err(e) = prometheus::default_registry().register(Box::new(h.clone())) {
        warn!("failed to register rustkvm_rpc_latency_seconds: {e}");
    }
    h
});

pub fn init_prometheus() {
    APP_INFO_REGISTERED.get_or_init(|| {
        let info = crate::version::VersionInfo::current();
        let gauge = match IntGauge::with_opts(
            Opts::new("rustkvm_app_info", "RustKVM application build info")
                .const_label("version", info.version)
                .const_label("revision", info.revision)
                .const_label("branch", info.branch)
                .const_label("build_date", info.build_date)
                .const_label("rust_version", info.rust_version)
                .const_label("platform", info.platform),
        ) {
            Ok(g) => g,
            Err(e) => {
                warn!("failed to construct rustkvm_app_info gauge: {e}");
                return;
            }
        };
        gauge.set(1);
        if let Err(e) = prometheus::default_registry().register(Box::new(gauge)) {
            warn!("failed to register rustkvm_app_info: {e}");
        }
    });
}
