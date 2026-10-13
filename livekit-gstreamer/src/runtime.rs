//! Locates the GStreamer runtime this process loaded and points GStreamer's
//! plugin lookup at it.
//!
//! On macOS the app links GStreamer through `@rpath`, so the same binary may
//! load the vendored framework (`third_party/gstreamer`, in dev) or the one
//! bundled in `Contents/Frameworks`. GStreamer doesn't derive its plugin path
//! from where it was loaded, so without this it can pick up plugins from a
//! system install (e.g. Homebrew) built against a different GStreamer.
//!
//! Variables that are already set are left alone, so they can still be
//! overridden from the shell for debugging.

use std::path::{Path, PathBuf};

/// Configures GStreamer to load plugins only from the runtime this process
/// linked, when that runtime is a relocatable `GStreamer.framework`. Returns
/// the runtime root (`.../GStreamer.framework/Versions/1.0`) it configured, or
/// `None` for a system install, which is left to its own defaults.
///
/// Must run before `gstreamer::init()`, and ideally before other threads
/// start, because it sets environment variables.
pub fn configure() -> Option<PathBuf> {
    let root = loaded_framework_root()?;

    set_default(
        "GST_PLUGIN_SYSTEM_PATH",
        &root.join("lib").join("gstreamer-1.0"),
    );
    set_default(
        "GST_PLUGIN_SCANNER",
        &root
            .join("libexec")
            .join("gstreamer-1.0")
            .join("gst-plugin-scanner"),
    );
    let gio_modules = root.join("lib").join("gio").join("modules");
    if gio_modules.is_dir() {
        set_default("GIO_EXTRA_MODULES", &gio_modules);
    }
    // Keep our registry apart from a system GStreamer's, which caches plugins
    // from a different install under the same default path.
    if let Some(registry) = registry_path() {
        set_default("GST_REGISTRY", &registry);
    }

    Some(root)
}

fn set_default(key: &str, value: &Path) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

#[cfg(target_os = "macos")]
fn loaded_framework_root() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;

    // Ask dyld which file provides gst_init, i.e. the libgstreamer we linked.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    let addr = gstreamer::ffi::gst_init as *const libc::c_void;
    if unsafe { libc::dladdr(addr, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    let lib = unsafe { CStr::from_ptr(info.dli_fname) };
    let lib = std::fs::canonicalize(OsStr::from_bytes(lib.to_bytes())).ok()?;

    // .../GStreamer.framework/Versions/1.0/lib/libgstreamer-1.0.0.dylib
    let root = lib.parent()?.parent()?;
    let in_framework = root
        .components()
        .any(|c| c.as_os_str() == "GStreamer.framework");
    in_framework.then(|| root.to_path_buf())
}

#[cfg(not(target_os = "macos"))]
fn loaded_framework_root() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn registry_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("livekit-gstreamer")
            .join(format!("registry.{}.bin", std::env::consts::ARCH)),
    )
}

#[cfg(not(target_os = "macos"))]
fn registry_path() -> Option<PathBuf> {
    None
}
