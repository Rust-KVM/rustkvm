use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    edid_bridge::build_edid_bridge();
}

/// Build the EDID bridge (edid.c) as a separate unit
mod edid_bridge {
    use super::*;

    pub fn build_edid_bridge() {
        println!("cargo:rerun-if-changed=cshim/edid.c");

        // Use /opt paths directly like Makefile - no fallbacks
        let rk_sdk_base = "/opt/rk3588-buildkit";
        let rk_media_output = format!("{}/aarch64-buildroot-linux-gnu", rk_sdk_base);
        let rk_media_libs = format!("{}/sysroot/usr/lib", rk_media_output);

        let cc = format!("{}/bin/aarch64-buildroot-linux-gnu-gcc", rk_sdk_base);
        let ar = format!("{}/bin/aarch64-buildroot-linux-gnu-ar", rk_sdk_base);

        // println!("cargo:warning=Using RK libs: {}", rk_media_libs);

        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let obj_e = out_dir.join("edid.o");
        let lib_e = out_dir.join("libedid.a");

        let status = Command::new(&cc)
            .args([
                "-c",
                "-fPIC",
                "cshim/edid.c",
                "-o",
                obj_e.to_str().expect("Invalid UTF-8 in obj path"),
                "-O2",
            ])
            .status()
            .expect("Failed to spawn cross-compiler");
        assert!(status.success(), "Cross-compile edid.c failed");

        let status = Command::new(&ar)
            .args([
                "rcs",
                lib_e.to_str().expect("Invalid UTF-8 in lib path"),
                obj_e.to_str().expect("Invalid UTF-8 in obj path"),
            ])
            .status()
            .expect("Failed to spawn archiver");
        assert!(status.success(), "Archive creation failed");

        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=edid");
        println!("cargo:rustc-link-search=native={}", rk_media_libs);
    }
}
