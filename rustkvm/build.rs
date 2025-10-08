use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    cross_compile::build_c_shims();
    native_video_bridge::build_native_video_bridge();
    edid_bridge::build_edid_bridge();
}

/// Cross-compilation utilities for C code integration
mod cross_compile {
    use super::*;

    /// Build all C shims required for the project
    pub fn build_c_shims() {
        println!("cargo:rerun-if-changed=cshim/getauxval.c");
        println!("cargo:rerun-if-env-changed=CROSS_TOOLCHAIN");
        println!("cargo:rerun-if-env-changed=CROSS_CC");
        println!("cargo:rerun-if-env-changed=CROSS_AR");
        println!("cargo:rerun-if-env-changed=CROSS_SYSROOT");

        let toolchain =
            env::var("CROSS_TOOLCHAIN").unwrap_or_else(|_| "/opt/rk3588-buildkit".to_string());
        let cc = env::var("CROSS_CC")
            .unwrap_or_else(|_| format!("{}/bin/aarch64-buildroot-linux-gnu-gcc", toolchain));
        let ar = env::var("CROSS_AR")
            .unwrap_or_else(|_| format!("{}/bin/aarch64-buildroot-linux-gnu-ar", toolchain));
        let sysroot = env::var("CROSS_SYSROOT")
            .unwrap_or_else(|_| format!("{}/aarch64-buildroot-linux-gnu/sysroot", toolchain));

        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let obj_file = out_dir.join("getauxval.o");
        let lib_file = out_dir.join("libgetauxval.a");

        // Compile C source to object file
        let status = Command::new(&cc)
            .args([
                "-c",
                "-fPIC",
                "cshim/getauxval.c",
                "-o",
                obj_file.to_str().expect("Invalid UTF-8 in obj path"),
                &format!("--sysroot={}", sysroot),
                "-O2",
            ])
            .status()
            .expect("Failed to spawn cross-compiler");
        assert!(status.success(), "Cross-compile getauxval.c failed");

        // Create static library
        let status = Command::new(&ar)
            .args([
                "rcs",
                lib_file.to_str().expect("Invalid UTF-8 in lib path"),
                obj_file.to_str().expect("Invalid UTF-8 in obj path"),
            ])
            .status()
            .expect("Failed to spawn archiver");
        assert!(status.success(), "Archive creation failed");

        println!("cargo::rustc-link-arg=-L{}", out_dir.display());
        println!("cargo::rustc-link-arg=-lgetauxval");
        println!("cargo::rustc-link-arg=--sysroot={}", sysroot);
    }
}

/// Build the native video bridge (get_video_track.c) as a separate unit
mod native_video_bridge {
    use super::*;

    pub fn build_native_video_bridge() {
        println!("cargo:rerun-if-changed=cshim/get_video_track.c");

        // Use /opt paths directly like Makefile - no fallbacks
        let rk_sdk_base = "/opt/rk3588-buildkit";
        let rk_media_output = format!("{}/aarch64-buildroot-linux-gnu", rk_sdk_base);
        let rk_media_include = format!("{}/sysroot/usr/include", rk_media_output);
        let rk_media_include_drm = format!("{}/libdrm", rk_media_include);
        let rk_media_libs = format!("{}/sysroot/usr/lib", rk_media_output);

        let cc = format!("{}/bin/aarch64-buildroot-linux-gnu-gcc", rk_sdk_base);
        let ar = format!("{}/bin/aarch64-buildroot-linux-gnu-ar", rk_sdk_base);

        println!(
            "cargo:warning=Using RK includes: {} and {}",
            rk_media_include, rk_media_include_drm
        );
        println!("cargo:warning=Using RK libs: {}", rk_media_libs);

        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let obj_v = out_dir.join("get_video_track.o");
        let lib_v = out_dir.join("libget_video_track.a");

        let status = Command::new(&cc)
            .args([
                "-c",
                "-fPIC",
                "-DRUSTKVM_AS_LIB=1",
                "-DUSE_ROCKCHIP_MPP",
                &format!("-I{}", rk_media_include),
                &format!("-I{}", rk_media_include_drm),
                "cshim/get_video_track.c",
                "-o",
                obj_v.to_str().expect("Invalid UTF-8 in obj path"),
                "-O2",
            ])
            .status()
            .expect("Failed to spawn cross-compiler");
        assert!(status.success(), "Cross-compile get_video_track.c failed");

        let status = Command::new(&ar)
            .args([
                "rcs",
                lib_v.to_str().expect("Invalid UTF-8 in lib path"),
                obj_v.to_str().expect("Invalid UTF-8 in obj path"),
            ])
            .status()
            .expect("Failed to spawn archiver");
        assert!(status.success(), "Archive creation failed");

        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=get_video_track");
        println!("cargo:rustc-link-search=native={}", rk_media_libs);
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=rockit");
        println!("cargo:rustc-link-lib=rockchip_mpp");
        println!("cargo:rustc-link-lib=rga");
        println!("cargo:rustc-link-lib=m");
    }
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
