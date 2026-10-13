fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    // GStreamer.framework libraries are linked through @rpath. Give the
    // examples and tests an rpath to the libdir pkg-config resolved, which is
    // the vendored framework when scripts/gstreamer/fetch-macos.sh has run.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    match pkg_config::get_variable("gstreamer-1.0", "libdir") {
        Ok(libdir) => {
            let libdir = std::fs::canonicalize(&libdir).unwrap_or_else(|_| libdir.into());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", libdir.display());
        }
        Err(e) => println!("cargo:warning=could not resolve the GStreamer libdir: {e}"),
    }
}
