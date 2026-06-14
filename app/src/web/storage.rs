use std::io::{Cursor, Write};

use anyhow::Result;
use futures::{StreamExt, TryStreamExt};
use prometheus::{Encoder, TextEncoder};
use salvo::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use tracing::{info, warn};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::types::ApiResponse;
use crate::config::{Config, get_config_manager};
use crate::observability::diagnostics;

const APP_LOG_PATH: &str = "/userdata/rustkvm/log/rustkvm_app.log";
const CRASH_DUMP_DIR: &str = "/userdata/rustkvm/log/crashes";

#[endpoint]
pub(super) async fn handle_storage_upload(
    req: &mut Request,
    _res: &mut Response,
) -> Result<Json<ApiResponse>, StatusError> {
    let upload_id = req.query::<String>("uploadId").unwrap_or_default();
    if upload_id.is_empty() {
        return Err(StatusError::not_found().brief("Upload not found"));
    }

    let (size, already_uploaded) =
        match crate::hardware::usb::storage::get_upload_progress(&upload_id).await {
            Ok((size, already)) => (size, already),
            Err(_) => return Err(StatusError::not_found().brief("Upload not found")),
        };

    let mut total_bytes_written = already_uploaded;
    let mut stream = req.take_body().into_stream();

    while let Some(item) = stream.next().await {
        let frame = match item {
            Ok(f) => f,
            Err(e) => {
                warn!("failed to read request body {}: {}", upload_id, e);
                return Err(
                    StatusError::internal_server_error().brief("Failed to read upload data")
                );
            }
        };

        match frame.into_data() {
            Ok(bytes) => {
                if bytes.is_empty() {
                    continue;
                }
                if let Err(e) =
                    crate::hardware::usb::storage::append_upload_data(&upload_id, bytes.as_ref())
                        .await
                {
                    warn!("failed to write upload chunk {}: {}", upload_id, e);
                    return Err(
                        StatusError::internal_server_error().brief("Failed to write upload data")
                    );
                }
                total_bytes_written += bytes.len() as i64;
            }
            Err(_non_data_frame) => {
                continue;
            }
        }
    }

    match crate::hardware::usb::storage::complete_upload(&upload_id).await {
        Ok(_) => {
            if total_bytes_written == size {
                info!(
                    "Upload {} completed successfully: {}/{}",
                    upload_id, total_bytes_written, size
                );
            } else {
                warn!(
                    "Upload {} ended before complete file received: {}/{}",
                    upload_id, total_bytes_written, size
                );
            }
            Ok(Json(ApiResponse { message: "Upload completed".to_string() }))
        }
        Err(e) => {
            warn!("failed to finalize upload {}: {}", upload_id, e);
            Err(StatusError::internal_server_error().brief("Failed to finalize upload"))
        }
    }
}

#[endpoint]
pub(super) async fn handle_metrics(res: &mut Response) -> Result<(), StatusError> {
    let metric_families = prometheus::default_registry().gather();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| StatusError::internal_server_error().brief(format!("encode metrics: {e}")))?;
    res.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"));
    res.write_body(buffer)
        .map_err(|e| StatusError::internal_server_error().brief(format!("write body: {e}")))?;
    Ok(())
}

#[endpoint]
pub(super) async fn handle_send_wol(
    req: &mut Request,
    res: &mut Response,
) -> Result<&'static str, StatusError> {
    let mac_addr = req
        .param::<String>("mac_addr")
        .ok_or_else(|| StatusError::bad_request().brief("missing mac-addr"))?;
    let broadcast_ip = req.query::<String>("broadcastIP");

    rkvm_net::wol::send_magic_packet(&mac_addr, broadcast_ip.as_deref()).await.map_err(|e| {
        warn!(error = %e, mac = %mac_addr, "WOL send failed");
        StatusError::internal_server_error().brief(format!("WOL failed: {e}"))
    })?;

    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    Ok("WOL sent")
}

#[endpoint]
pub(super) async fn handle_diagnostics_download(res: &mut Response) -> Result<(), StatusError> {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let filename = format!("rustkvm-diagnostics-{timestamp}.zip");
    let body = build_diagnostics_zip()
        .await
        .map_err(|e| StatusError::internal_server_error().brief(format!("build zip: {e}")))?;

    let headers = res.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    let disp = format!("attachment; filename={filename}");
    if let Ok(value) = HeaderValue::from_str(&disp) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    res.write_body(body)
        .map_err(|e| StatusError::internal_server_error().brief(format!("write body: {e}")))?;
    Ok(())
}

async fn build_diagnostics_zip() -> Result<Vec<u8>> {
    let cursor = Cursor::new(Vec::<u8>::new());
    let mut zip = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    if let Ok(bytes) = tokio::fs::read(APP_LOG_PATH).await {
        zip.start_file("app.log", options)?;
        zip.write_all(&bytes)?;
    }

    let snapshot =
        diagnostics::get_system_snapshot().await.unwrap_or_else(|_| serde_json::json!({}));
    let snapshot_text = serde_json::to_string_pretty(&snapshot)?;
    zip.start_file("system-diagnostics.json", options)?;
    zip.write_all(snapshot_text.as_bytes())?;

    if let Ok(mut entries) = tokio::fs::read_dir(CRASH_DUMP_DIR).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("rustkvm-") || !name.ends_with(".log") {
                continue;
            }
            if let Ok(bytes) = tokio::fs::read(entry.path()).await {
                zip.start_file(format!("crashes/{name}"), options)?;
                zip.write_all(&bytes)?;
            }
        }
    }

    let cm = get_config_manager();
    let cfg = cm.get().await;
    if let Ok(redacted) = redacted_config_toml(&cfg) {
        zip.start_file("config.toml", options)?;
        zip.write_all(redacted.as_bytes())?;
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

fn redacted_config_toml(cfg: &Config) -> Result<String> {
    let mut redacted = cfg.clone();
    redacted.cloud_token = None;
    redacted.local_auth_token = None;
    redacted.hashed_password = None;
    redacted.google_identity = None;
    Ok(toml::to_string_pretty(&redacted)?)
}
