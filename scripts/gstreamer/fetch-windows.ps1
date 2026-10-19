# Fetches the official GStreamer Windows (MSVC x86_64) binaries into the repo,
# leaving nothing installed on the machine:
#
#   third_party/gstreamer/windows/msvc_x86_64   runtime + development files
#   third_party/gstreamer/windows/bundle        what ships next to the app exe
#   third_party/gstreamer/sdk -> windows/msvc_x86_64   (directory junction)
#
# Builds use sdk/ via pkg-config (.cargo/config.toml). The Tauri bundle
# (tauri.windows.conf.json) copies bundle/ into the install folder, where
# livekit_gstreamer::runtime finds the plugins at startup.
#
# The 1.28 installer is an Inno Setup exe with no extract-only mode, so this
# installs it per-user into a scratch folder, copies the files out, and runs
# its uninstaller.
#
# Usage: scripts/gstreamer/fetch-windows.ps1 [-Force]
[CmdletBinding()]
param([switch]$Force)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Pinned version. Keep in step with fetch-macos.sh.
$GstVersion = '1.28.8'
$Installer = "gstreamer-1.0-msvc-x86_64-$GstVersion.exe"
$InstallerSha256 = 'e4c41ee6ee9c6c32a45d198463ec08d79bac3bd5fa58adc93d13fe4a08abc9d9'
$BaseUrl = "https://gstreamer.freedesktop.org/data/pkg/windows/$GstVersion/msvc"

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'this script only runs on Windows'
}

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$GstDir = Join-Path $RepoRoot 'third_party\gstreamer'
$CacheDir = Join-Path $GstDir ".cache\$GstVersion"
$DestDir = Join-Path $GstDir 'windows'
$SdkDir = Join-Path $DestDir 'msvc_x86_64'
$BundleDir = Join-Path $DestDir 'bundle'
$Stamp = Join-Path $DestDir '.version'

# pkg-config packages the gstreamer-rs crates link against. Only these and
# their Requires/Requires.private closure are exposed to cargo, so other -sys
# crates don't pick up GStreamer's copies of their libraries.
$CargoPcRoots = @('gstreamer-1.0', 'gstreamer-base-1.0', 'gstreamer-app-1.0', 'gstreamer-video-1.0', 'glib-2.0', 'gobject-2.0', 'gio-2.0')

# Creates the OS-neutral entry points used by .cargo/config.toml:
#   third_party/gstreamer/sdk -> windows/msvc_x86_64
#   sdk/lib/pkgconfig-cargo/  copies of the CARGO_PC_ROOTS closure
# The copies have ${pcfiledir} replaced by the folder's absolute path with
# forward slashes: some pkg-config builds (e.g. the one in a system GStreamer
# install, often first on PATH) strip the backslashes from ${pcfiledir},
# which breaks the -L path cargo passes to the linker.
function Link-Sdk {
    $pcDir = Join-Path $SdkDir 'lib\pkgconfig'
    $cargoPcDir = Join-Path $SdkDir 'lib\pkgconfig-cargo'
    $pcFileDir = (Join-Path $GstDir 'sdk\lib\pkgconfig-cargo') -replace '\\', '/'
    $utf8 = New-Object System.Text.UTF8Encoding($false)

    if (Test-Path $cargoPcDir) { Remove-Item -Recurse -Force $cargoPcDir }
    New-Item -ItemType Directory -Force $cargoPcDir | Out-Null

    $queue = [System.Collections.Generic.Queue[string]]::new()
    $CargoPcRoots | ForEach-Object { $queue.Enqueue($_) }
    $seen = @{}
    while ($queue.Count -gt 0) {
        $pc = $queue.Dequeue()
        if ($seen.ContainsKey($pc)) { continue }
        $seen[$pc] = $true
        $file = Join-Path $pcDir "$pc.pc"
        if (-not (Test-Path $file)) { throw "$pc.pc not found in $pcDir" }
        $text = [IO.File]::ReadAllText($file).Replace('${pcfiledir}', $pcFileDir)
        [IO.File]::WriteAllText((Join-Path $cargoPcDir "$pc.pc"), $text, $utf8)
        foreach ($line in Get-Content $file) {
            if ($line -match '^Requires(\.private)?:(.*)$') {
                $deps = $Matches[2] -replace '[<>=]+\s*[0-9.]+', '' -replace ',', ' '
                $deps -split '\s+' | Where-Object { $_ } | ForEach-Object { $queue.Enqueue($_) }
            }
        }
    }

    # A junction needs no admin rights or developer mode, unlike a symlink.
    $sdkLink = Join-Path $GstDir 'sdk'
    if (Test-Path $sdkLink) { (Get-Item $sdkLink).Delete() }
    New-Item -ItemType Junction -Path $sdkLink -Target $SdkDir | Out-Null
}

if (-not $Force -and (Test-Path $Stamp) -and ((Get-Content $Stamp -Raw).Trim() -eq $GstVersion)) {
    Link-Sdk
    Write-Host "GStreamer $GstVersion already present at $DestDir"
    exit 0
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -Algorithm SHA256 $Path).Hash.ToLowerInvariant()
}

New-Item -ItemType Directory -Force $CacheDir | Out-Null
$installerPath = Join-Path $CacheDir $Installer
if (-not (Test-Path $installerPath) -or (Get-Sha256 $installerPath) -ne $InstallerSha256) {
    Write-Host "Downloading $Installer"
    $part = "$installerPath.part"
    for ($attempt = 1; ; $attempt++) {
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Installer" -OutFile $part
            break
        } catch {
            if ($attempt -ge 3) { throw }
            Write-Host "Download failed ($($_.Exception.Message)), retrying"
            Start-Sleep -Seconds (5 * $attempt)
        }
    }
    Move-Item -Force $part $installerPath
}
if ((Get-Sha256 $installerPath) -ne $InstallerSha256) {
    throw "checksum mismatch for $Installer"
}

# A per-user GStreamer install shares the installer's app id, so ours would be
# taken as an upgrade of it, and our uninstall would unregister it.
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$existing = @(Get-ChildItem $uninstallKey -ErrorAction SilentlyContinue |
    Where-Object { "$($_.GetValue('DisplayName'))" -like 'GStreamer*' })
if ($existing) {
    throw "a per-user GStreamer install is registered ($($existing[0].PSChildName)); uninstall it first, or this script would take it over"
}

$work = Join-Path $GstDir ".cache\work-$PID"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force $work | Out-Null
$installDir = Join-Path $work 'install'
try {
    # /TYPE=devel is runtime + headers + pkg-config, without debug symbols.
    # /TASKS= skips the optional environment variable and registry entries.
    Write-Host "Installing $Installer into $installDir"
    $installArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/CURRENTUSER', '/TYPE=devel', '/TASKS=', '/NOICONS', "/DIR=`"$installDir`"", "/LOG=`"$work\install.log`"")
    $proc = Start-Process -FilePath $installerPath -ArgumentList $installArgs -Wait -PassThru
    if ($proc.ExitCode -ne 0) { throw "installer exited with code $($proc.ExitCode) (log: $work\install.log)" }

    $dll = Get-ChildItem -Path $installDir -Recurse -Filter 'gstreamer-1.0-0.dll' | Where-Object { $_.Directory.Name -eq 'bin' } | Select-Object -First 1
    if (-not $dll) { throw "gstreamer-1.0-0.dll not found under $installDir" }
    $root = $dll.Directory.Parent.FullName

    $stage = Join-Path $work 'msvc_x86_64'
    Write-Host "Copying $root"
    # robocopy exit codes below 8 mean success.
    robocopy $root $stage /E /NFL /NDL /NJH /NJS /NP /XF '*.pdb' 'unins*' | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with code $LASTEXITCODE" }
    $global:LASTEXITCODE = 0
} finally {
    $uninstaller = Get-ChildItem -Path $installDir -Recurse -Filter 'unins*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($uninstaller) {
        Write-Host 'Uninstalling the scratch install'
        Start-Process -FilePath $uninstaller.FullName -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait | Out-Null
    }
}

# Bundle layout: the DLLs sit next to syncflow-publisher.exe, so Windows'
# loader finds them without PATH changes. gst-plugin-scanner and GLib's spawn
# helpers sit there too so they find the same DLLs.
Write-Host 'Staging bundle'
$bundleStage = Join-Path $work 'bundle'
$bin = Join-Path $stage 'bin'
New-Item -ItemType Directory -Force $bundleStage | Out-Null
Copy-Item (Join-Path $bin '*.dll') $bundleStage
Copy-Item (Join-Path $bin 'gspawn-win64-helper*.exe') $bundleStage
Copy-Item (Join-Path $stage 'libexec\gstreamer-1.0\gst-plugin-scanner.exe') $bundleStage
$plugins = Join-Path $bundleStage 'lib\gstreamer-1.0'
New-Item -ItemType Directory -Force $plugins | Out-Null
# gstpython needs a Python install the app doesn't ship; it would only log a
# load failure on every registry scan.
Copy-Item (Join-Path $stage 'lib\gstreamer-1.0\*.dll') $plugins -Exclude 'gstpython.dll'
# libproxy (used by GIO's giolibproxy module) loads pxbackend-1.0.dll, which
# GStreamer ships in lib\libproxy, outside any DLL search path.
Copy-Item (Join-Path $stage 'lib\libproxy\*.dll') $bundleStage -ErrorAction SilentlyContinue
$gioModules = Join-Path $stage 'lib\gio\modules'
if (Test-Path $gioModules) {
    $dest = Join-Path $bundleStage 'lib\gio\modules'
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item (Join-Path $gioModules '*.dll') $dest
}

# The app links the CRT statically, but GStreamer's DLLs need the MSVC
# runtime. Ship it app-locally unless GStreamer already provides it.
$crt = @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')
$missing = $crt | Where-Object { -not (Test-Path (Join-Path $bundleStage $_)) }
if ($missing) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw "the MSVC runtime ($($missing -join ', ')) is not in GStreamer's bin and Visual Studio was not found" }
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $crtDir = Get-ChildItem -Path (Join-Path $vs 'VC\Redist\MSVC') -Directory -Filter '*.*' |
        Sort-Object { [version]($_.Name -replace '[^0-9.]', '') } -Descending -ErrorAction SilentlyContinue |
        ForEach-Object { Get-ChildItem -Path (Join-Path $_.FullName 'x64') -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue } |
        Select-Object -First 1
    if (-not $crtDir) { throw 'the MSVC redistributable folder was not found' }
    foreach ($name in $missing) {
        Copy-Item (Join-Path $crtDir.FullName $name) $bundleStage
    }
    Write-Host "Added the MSVC runtime from $($crtDir.FullName)"
}

if (Test-Path $DestDir) {
    $oldLink = Join-Path $GstDir 'sdk'
    if (Test-Path $oldLink) { (Get-Item $oldLink).Delete() }
    Remove-Item -Recurse -Force $DestDir
}
New-Item -ItemType Directory -Force $DestDir | Out-Null
Move-Item $stage $SdkDir
Move-Item $bundleStage $BundleDir
Set-Content -Path $Stamp -Value $GstVersion -Encoding ascii
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue

Link-Sdk

$size = (Get-ChildItem $BundleDir -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("GStreamer $GstVersion ready at $DestDir (bundle: {0:N0} MB)" -f $size)
Write-Host 'For cargo test and the examples, put the DLLs on PATH:'
Write-Host "  `$env:PATH = `"$GstDir\sdk\bin;`$env:PATH`""
