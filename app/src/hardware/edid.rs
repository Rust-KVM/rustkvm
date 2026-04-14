use anyhow::Result;
use tracing::{debug, info};

unsafe extern "C" {
    fn get_edid(edid: *mut u8, max_size: usize) -> i32;
    fn set_edid(edid: *const u8, size: usize) -> i32;
    fn videoc_log_status() -> *const std::os::raw::c_char;
    fn free_videoc_status(status: *mut std::os::raw::c_char);
}

pub fn read_edid() -> Result<Vec<u8>> {
    let mut buffer = vec![0u8; 256];
    let result = unsafe { get_edid(buffer.as_mut_ptr(), 256) };

    if result < 0 {
        anyhow::bail!("Failed to read EDID: {}", std::io::Error::last_os_error());
    }

    let actual_size = result as usize;
    buffer.truncate(actual_size);
    buffer.shrink_to_fit();
    info!("Read EDID: {} bytes", buffer.len());
    Ok(buffer)
}

pub fn write_edid(edid: &[u8]) -> Result<()> {
    if edid.len() != 128 && edid.len() != 256 {
        anyhow::bail!("EDID size must be 128 or 256 bytes, got {}", edid.len());
    }

    let result = unsafe { set_edid(edid.as_ptr(), edid.len()) };
    if result < 0 {
        anyhow::bail!("Failed to set EDID: {}", std::io::Error::last_os_error());
    }

    info!("Set EDID: {} bytes", edid.len());
    Ok(())
}

pub fn get_video_controller_status() -> Result<String> {
    let c_str = unsafe { videoc_log_status() };
    if c_str.is_null() {
        anyhow::bail!("Failed to get video controller status");
    }

    let c_str = unsafe { std::ffi::CStr::from_ptr(c_str) };
    let status = c_str.to_str()?.to_string();

    unsafe { free_videoc_status(c_str.as_ptr() as *mut _) };

    debug!("Video controller status retrieved: {} chars", status.len());
    Ok(status)
}
