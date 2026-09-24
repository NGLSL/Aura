//! Link Microsoft Detours for DetourCreateProcessWithDllExW (ticket 04).

use std::env;
use std::path::PathBuf;

fn detours_root() -> PathBuf {
    if let Ok(root) = env::var("DETOURS_ROOT") {
        return PathBuf::from(root);
    }
    for candidate in ["D:/Tools/Detours", "third_party/Detours"] {
        let p = PathBuf::from(candidate);
        if p.join("include").join("detours.h").exists() {
            return p;
        }
    }
    PathBuf::from("D:/Tools/Detours")
}

fn main() {
    println!("cargo:rerun-if-env-changed=DETOURS_ROOT");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let root = detours_root();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let libdir = if arch == "x86" {
        root.join("lib.X86")
    } else {
        root.join("lib.X64")
    };
    let lib = libdir.join("detours.lib");
    if !lib.exists() {
        // Let the link fail with a clear path if Detours was not built.
        println!(
            "cargo:warning=detours.lib not found at {} — run nmake in Detours/src",
            lib.display()
        );
    }
    println!("cargo:rustc-link-search={}", libdir.display());
    println!("cargo:rustc-link-lib=static=detours");
    println!("cargo:rustc-link-lib=advapi32");
    println!("cargo:rustc-link-lib=shell32");
    println!("cargo:rustc-link-lib=ole32");
}
