fn main() {
    gstreamer_rpath();
    tauri_build::build()
}

/// GStreamer.framework libraries are linked through @rpath. The bundled
/// framework (Contents/Frameworks) comes first; the libdir pkg-config resolved
/// (the vendored framework in dev) is the fallback for `tauri dev`.
fn gstreamer_rpath() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!(
        "cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks/GStreamer.framework/Versions/1.0/lib"
    );
    match pkg_config::get_variable("gstreamer-1.0", "libdir") {
        Ok(libdir) => {
            let libdir = std::fs::canonicalize(&libdir).unwrap_or_else(|_| libdir.into());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", libdir.display());
        }
        Err(e) => println!("cargo:warning=could not resolve the GStreamer libdir: {e}"),
    }
}
