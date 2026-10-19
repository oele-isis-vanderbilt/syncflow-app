//! Locates the GStreamer runtime this process loaded and points GStreamer's
//! plugin lookup at it.
//!
//! - **macOS:** the app links GStreamer through `@rpath`, so the same binary
//!   may load the vendored framework (`third_party/gstreamer`, in dev) or the
//!   one bundled in `Contents/Frameworks`.
//! - **Windows:** the installer puts GStreamer's DLLs next to the app exe, with
//!   plugins in `lib\gstreamer-1.0` beside them. GStreamer's default lookup
//!   (`<dll dir>\..\lib\gstreamer-1.0`) misses that layout. In dev, the DLLs
//!   come from the vendored `third_party\gstreamer\sdk\bin` or a system install,
//!   which use the standard layout.
//!
//! Without this, GStreamer can pick up plugins from a system install (e.g.
//! Homebrew, or another GStreamer on Windows) built against a different
//! version. Variables that are already set are left alone, so they can still
//! be overridden from the shell for debugging.

use std::path::{Path, PathBuf};

/// Where the loaded GStreamer runtime keeps its plugins and scanner.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
struct Runtime {
    /// Holds `lib/gstreamer-1.0` (and `lib/gio/modules`, if present).
    root: PathBuf,
    scanner: PathBuf,
}

/// Configures GStreamer to load plugins only from the runtime this process
/// linked. Returns the runtime root it configured, or `None` when the runtime
/// isn't one we recognise (e.g. a Linux system install), which is left to its
/// own defaults.
///
/// Must run before `gstreamer::init()`, and ideally before other threads
/// start, because it sets environment variables.
pub fn configure() -> Option<PathBuf> {
    let Runtime { root, scanner } = loaded_runtime()?;

    set_default(
        "GST_PLUGIN_SYSTEM_PATH",
        &root.join("lib").join("gstreamer-1.0"),
    );
    set_default("GST_PLUGIN_SCANNER", &scanner);
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

#[cfg(any(target_os = "macos", windows))]
fn registry_file() -> String {
    format!("registry.{}.bin", std::env::consts::ARCH)
}

#[cfg(target_os = "macos")]
fn loaded_runtime() -> Option<Runtime> {
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
    in_framework.then(|| Runtime {
        root: root.to_path_buf(),
        scanner: root
            .join("libexec")
            .join("gstreamer-1.0")
            .join("gst-plugin-scanner"),
    })
}

#[cfg(target_os = "macos")]
fn registry_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("livekit-gstreamer")
            .join(registry_file()),
    )
}

#[cfg(windows)]
fn loaded_runtime() -> Option<Runtime> {
    let dll = loaded_gstreamer_dll()?;
    let dll_dir = dll.parent()?;

    // Bundled: <install>\gstreamer-1.0-0.dll, <install>\lib\gstreamer-1.0.
    if dll_dir.join("lib").join("gstreamer-1.0").is_dir() {
        return Some(Runtime {
            root: dll_dir.to_path_buf(),
            scanner: dll_dir.join("gst-plugin-scanner.exe"),
        });
    }
    // Standard: <root>\bin\gstreamer-1.0-0.dll, <root>\lib\gstreamer-1.0.
    let root = dll_dir.parent()?;
    root.join("lib")
        .join("gstreamer-1.0")
        .is_dir()
        .then(|| Runtime {
            root: root.to_path_buf(),
            scanner: root
                .join("libexec")
                .join("gstreamer-1.0")
                .join("gst-plugin-scanner.exe"),
        })
}

/// Path of the DLL that provides `gst_init`, i.e. the gstreamer-1.0-0.dll the
/// loader picked.
#[cfg(windows)]
fn loaded_gstreamer_dll() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    let mut module = std::ptr::null_mut();
    let addr = gstreamer::ffi::gst_init as *const u16;
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if unsafe { GetModuleHandleExW(flags, addr, &mut module) } == 0 {
        return None;
    }

    let mut buf = vec![0u16; 260];
    loop {
        let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) };
        if len == 0 {
            return None;
        }
        // A full buffer means the path was truncated.
        if (len as usize) < buf.len() {
            buf.truncate(len as usize);
            return Some(PathBuf::from(OsString::from_wide(&buf)));
        }
        buf.resize(buf.len() * 2, 0);
    }
}

#[cfg(windows)]
fn registry_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("livekit-gstreamer")
            .join(registry_file()),
    )
}

#[cfg(not(any(target_os = "macos", windows)))]
fn loaded_runtime() -> Option<Runtime> {
    None
}

#[cfg(not(any(target_os = "macos", windows)))]
fn registry_path() -> Option<PathBuf> {
    None
}
