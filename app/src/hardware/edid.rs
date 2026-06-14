use anyhow::Result;
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Updater, ioctl, opcode};
use tracing::info;

const V4L_NODE: &str = "/dev/video0";

#[repr(C)]
struct V4l2Edid {
    pad: u32,
    start_block: u32,
    blocks: u32,
    reserved: [u32; 5],
    edid: *mut u8,
}

const VIDIOC_G_EDID: Opcode = opcode::read_write::<V4l2Edid>(b'V', 40);
const VIDIOC_S_EDID: Opcode = opcode::read_write::<V4l2Edid>(b'V', 41);

pub fn read_edid() -> Result<Vec<u8>> {
    let fd = open(V4L_NODE, OFlags::RDWR, Mode::empty())?;
    let mut buf = vec![0u8; 256];
    let mut req =
        V4l2Edid { pad: 0, start_block: 0, blocks: 2, reserved: [0; 5], edid: buf.as_mut_ptr() };
    // SAFETY: VIDIOC_G_EDID opcode matches V4l2Edid layout. `req.edid` points
    // into `buf` (256 bytes, exclusively borrowed for the duration of the call)
    // and the kernel writes at most `blocks * 128` bytes there, updating
    // `req.blocks` to the actual count.
    unsafe {
        ioctl(&fd, Updater::<VIDIOC_G_EDID, V4l2Edid>::new(&mut req))?;
    }
    let n = req.blocks as usize * 128;
    buf.truncate(n);
    info!("Read EDID: {} bytes", buf.len());
    Ok(buf)
}

pub fn write_edid(edid: &[u8]) -> Result<()> {
    if edid.len() != 128 && edid.len() != 256 {
        anyhow::bail!("EDID size must be 128 or 256 bytes, got {}", edid.len());
    }
    let mut data = edid.to_vec();
    fix_edid_checksum(&mut data);

    let fd = open(V4L_NODE, OFlags::RDWR, Mode::empty())?;
    let mut req = V4l2Edid {
        pad: 0,
        start_block: 0,
        blocks: (data.len() / 128) as u32,
        reserved: [0; 5],
        edid: data.as_mut_ptr(),
    };
    // SAFETY: VIDIOC_S_EDID opcode matches V4l2Edid layout. `req.edid` points
    // into `data` (128 or 256 bytes, exclusively borrowed) and the kernel only
    // reads `blocks * 128` bytes from there.
    unsafe {
        ioctl(&fd, Updater::<VIDIOC_S_EDID, V4l2Edid>::new(&mut req))?;
    }
    info!("Set EDID: {} bytes", data.len());
    Ok(())
}

fn fix_edid_checksum(edid: &mut [u8]) {
    for block in edid.chunks_mut(128) {
        let sum = block[..127].iter().copied().fold(0u8, u8::wrapping_add);
        block[127] = 0u8.wrapping_sub(sum);
    }
}
