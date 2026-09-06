//! Embed an `Info.plist` into the `klipa-helper` Mach-O binary.
//!
//! `SMAppService` only registers a daemon whose helper executable carries
//! a bundle identifier. For an app target Xcode injects this via the
//! "Create Info.plist Section in Binary" build setting; for a plain
//! executable it means an `Info.plist` embedded in the binary's
//! `__TEXT,__info_plist` section. A stock Rust build emits no such
//! section, so `SMAppService.daemon(...).register()` silently fails and no
//! daemon is ever created (the app then falls back to the admin-prompt
//! path). Here we generate a minimal `Info.plist` at build time and ask
//! the linker to create the section. It also makes `codesign` stamp the
//! binary with `dev.peterdsp.klipa.helper` instead of the filename, so the
//! signed identifier matches the daemon's launchd `Label`.
//!
//! macOS only; a no-op on every other target.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Only the macOS daemon needs (or can use) the embedded plist section.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set for build scripts"));
    // Keep the embedded bundle version in step with the crate version.
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());

    // Minimal but sufficient: SMAppService needs the bundle identifier;
    // the version keys keep the daemon's version legible in diagnostics.
    let info_plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>dev.peterdsp.klipa.helper</string>
  <key>CFBundleName</key><string>klipa-helper</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
</dict>
</plist>
"#
    );

    let info_path = out_dir.join("helper-info.plist");
    fs::write(&info_path, info_plist).expect("write embedded Info.plist");

    // `-sectcreate __TEXT __info_plist <file>` (passed through the clang
    // driver to ld) creates the section from our generated file. Applies
    // to the binary target, and to each arch of a universal build since
    // the script runs per target.
    println!(
        "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        info_path.display()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
