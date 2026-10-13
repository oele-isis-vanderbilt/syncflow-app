#!/usr/bin/env bash
# Fetches the official GStreamer macOS binaries and unpacks them, without
# installing anything system-wide, into:
#
#   third_party/gstreamer/macos/GStreamer.framework
#   third_party/gstreamer/sdk -> macos/GStreamer.framework/Versions/1.0
#
# Both builds (via pkg-config + rpath) and runtime (livekit_gstreamer::runtime)
# use this folder, and the Tauri bundle copies it into Contents/Frameworks.
#
# Usage: scripts/gstreamer/fetch-macos.sh [--force]
set -euo pipefail

# Pinned version. gstreamer-rs 0.24 supports up to the 1.28 API (feature v1_28).
GST_VERSION="1.28.8"
RUNTIME_PKG="gstreamer-1.0-${GST_VERSION}-universal.pkg"
RUNTIME_SHA256="d371ab04a7632713626836c2fc234e863cc98484c74c473eba99b0fbd9b3a3fe"
DEVEL_PKG="gstreamer-1.0-devel-${GST_VERSION}-universal.pkg"
DEVEL_SHA256="e6d380bc17be7eccf13e0154567d4718fc9321da822408c7ca0cd808b3d4a3f7"
BASE_URL="https://gstreamer.freedesktop.org/data/pkg/osx/${GST_VERSION}"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: this script only runs on macOS" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GST_DIR="${REPO_ROOT}/third_party/gstreamer"
CACHE_DIR="${GST_DIR}/.cache/${GST_VERSION}"
DEST_DIR="${GST_DIR}/macos"
FRAMEWORK="${DEST_DIR}/GStreamer.framework"
STAMP="${DEST_DIR}/.version"

# pkg-config packages the gstreamer-rs crates link against. Only these and
# their Requires/Requires.private closure are exposed to cargo, so other -sys
# crates (bzip2-sys, openssl-sys, ...) don't pick up the framework's copies.
CARGO_PC_ROOTS="gstreamer-1.0 gstreamer-base-1.0 gstreamer-app-1.0 gstreamer-video-1.0 glib-2.0 gobject-2.0 gio-2.0"

# Creates the OS-neutral entry points used by .cargo/config.toml:
#   third_party/gstreamer/sdk -> macos/GStreamer.framework/Versions/1.0
#   sdk/lib/pkgconfig-cargo/  symlinks to the CARGO_PC_ROOTS closure
# The folder sits next to lib/pkgconfig so ${pcfiledir}/../.. still resolves
# to the framework root.
link_sdk() {
    local sdk_rel="macos/GStreamer.framework/Versions/1.0"
    local pc_dir="${GST_DIR}/${sdk_rel}/lib/pkgconfig"
    local cargo_pc_dir="${GST_DIR}/${sdk_rel}/lib/pkgconfig-cargo"

    rm -rf "${cargo_pc_dir}"
    mkdir -p "${cargo_pc_dir}"
    local queue="${CARGO_PC_ROOTS}" seen=" " pc requires
    while [[ -n "${queue// /}" ]]; do
        set -- ${queue}
        pc="$1"
        shift
        queue="$*"
        [[ "${seen}" == *" ${pc} "* ]] && continue
        seen="${seen}${pc} "
        if [[ ! -f "${pc_dir}/${pc}.pc" ]]; then
            echo "error: ${pc}.pc not found in ${pc_dir}" >&2
            exit 1
        fi
        ln -s "../pkgconfig/${pc}.pc" "${cargo_pc_dir}/${pc}.pc"
        requires="$({ grep -E '^Requires(\.private)?:' "${pc_dir}/${pc}.pc" || true; } |
            sed -E 's/^Requires(\.private)?://; s/[<>=]+ *[0-9.]+//g; s/,/ /g' | tr '\n' ' ')"
        queue="${queue} ${requires}"
    done

    ln -sfn "${sdk_rel}" "${GST_DIR}/sdk"
}

if [[ "${1:-}" != "--force" && -f "${STAMP}" && "$(cat "${STAMP}")" == "${GST_VERSION}" ]]; then
    link_sdk
    echo "GStreamer ${GST_VERSION} already present at ${FRAMEWORK}"
    exit 0
fi

download() {
    local pkg="$1" sha="$2"
    local path="${CACHE_DIR}/${pkg}"
    if [[ ! -f "${path}" ]] || ! echo "${sha}  ${path}" | shasum -a 256 -c --status; then
        echo "Downloading ${pkg}"
        curl -fL --retry 3 -o "${path}.part" "${BASE_URL}/${pkg}"
        mv "${path}.part" "${path}"
    fi
    if ! echo "${sha}  ${path}" | shasum -a 256 -c --status; then
        echo "error: checksum mismatch for ${pkg}" >&2
        exit 1
    fi
}

mkdir -p "${CACHE_DIR}"
download "${RUNTIME_PKG}" "${RUNTIME_SHA256}"
download "${DEVEL_PKG}" "${DEVEL_SHA256}"

WORK="$(mktemp -d "${GST_DIR}/.cache/work.XXXXXX")"
trap 'rm -rf "${WORK}"' EXIT

# The runtime pkg contains one component pkg per plugin set (core, codecs,
# codecs-gpl, ...), each with a payload rooted at Versions/1.0, plus
# osx-framework, which holds the framework skeleton (symlinks, Info.plist).
echo "Unpacking runtime"
pkgutil --expand-full "${CACHE_DIR}/${RUNTIME_PKG}" "${WORK}/runtime"
STAGE="${WORK}/GStreamer.framework"
ditto "${WORK}/runtime/osx-framework-${GST_VERSION}-universal.pkg/Payload" "${STAGE}"
for component in "${WORK}"/runtime/*.pkg; do
    [[ "${component}" == *osx-framework-* ]] && continue
    ditto "${component}/Payload" "${STAGE}/Versions/1.0"
done

# The devel pkg is mostly static libraries we don't link. Keep only the
# pkg-config files; they're relative to ${pcfiledir}, so they work in place.
echo "Unpacking pkg-config files"
pkgutil --expand-full "${CACHE_DIR}/${DEVEL_PKG}" "${WORK}/devel"
mkdir -p "${STAGE}/Versions/1.0/lib/pkgconfig"
for component in "${WORK}"/devel/*.pkg; do
    if [[ -d "${component}/Payload/lib/pkgconfig" ]]; then
        ditto "${component}/Payload/lib/pkgconfig" "${STAGE}/Versions/1.0/lib/pkgconfig"
    fi
done
rm -rf "${WORK}/devel"

# The app may be built for arm64 or x86_64 from the same framework.
echo "Checking architectures"
thin=0
while IFS= read -r file; do
    file -b "${file}" | grep -q 'Mach-O' || continue
    archs="$(lipo -archs "${file}" 2>/dev/null || true)"
    if [[ " ${archs} " != *" arm64 "* || " ${archs} " != *" x86_64 "* ]]; then
        echo "error: ${file#"${STAGE}/"} is not universal (${archs:-unknown})" >&2
        thin=1
    fi
done < <(find "${STAGE}/Versions/1.0/lib" "${STAGE}/Versions/1.0/libexec" -type f \( -name '*.dylib' -o -perm -u+x \))
if [[ "${thin}" -ne 0 ]]; then
    exit 1
fi

rm -rf "${DEST_DIR}"
mkdir -p "${DEST_DIR}"
mv "${STAGE}" "${FRAMEWORK}"
echo "${GST_VERSION}" > "${STAMP}"

link_sdk

# Remove .gitignore

rm -rf ${FRAMEWORK}/.gitignore
rm -rf ${FRAMEWORK}/Headers

echo "GStreamer ${GST_VERSION} ready at ${FRAMEWORK}"
