//! Host target triple mapping (same as `install.sh`).

use crate::error::{Error, Result};

/// Detect the release target triple for this host.
pub fn detect_target() -> Result<&'static str> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let arch = match arch {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => {
            return Err(Error::UnsupportedTarget(format!(
                "unsupported arch {other} (need aarch64 or x86_64)"
            )));
        }
    };

    let os = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        other => {
            return Err(Error::UnsupportedTarget(format!(
                "unsupported OS {other} (need macOS or Linux)"
            )));
        }
    };

    // Prefer native arm64 over Rosetta (install.sh checks sysctl); on aarch64
    // builds we already are native.
    Ok(match (arch, os) {
        ("aarch64", "apple-darwin") => "aarch64-apple-darwin",
        ("x86_64", "apple-darwin") => {
            if running_under_rosetta() {
                "aarch64-apple-darwin"
            } else {
                "x86_64-apple-darwin"
            }
        }
        ("aarch64", "unknown-linux-musl") => "aarch64-unknown-linux-musl",
        ("x86_64", "unknown-linux-musl") => "x86_64-unknown-linux-musl",
        _ => unreachable!(),
    })
}

fn running_under_rosetta() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        use std::process::Command;
        let Ok(out) = Command::new("sysctl")
            .args(["-in", "sysctl.proc_translated"])
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).trim() == "1"
    }
    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    {
        false
    }
}
