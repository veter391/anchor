<#
.SYNOPSIS
  Build the Anchor Windows installer (NSIS .exe) with a reproducible, staged
  set of native runtime DLLs.

.DESCRIPTION
  Anchor installs per-user (no admin) via an NSIS setup .exe -- see
  docs/distribution.md. The installed anchor.exe must find its speech-runtime
  DLLs (sherpa-onnx + ONNX Runtime) and the Microsoft Visual C++ runtime NEXT
  TO it, exactly like the portable build. Those files are build-generated and
  live under a machine-specific VS install, so they are gitignored and staged
  fresh here rather than committed.

  This script:
    1. runs a release build (so the exe + the sherpa DLLs exist);
    2. stages the exact self-contained DLL set into src-tauri/installer-libs/
       (bundled as a Tauri resource; the NSIS POSTINSTALL hook in
       src-tauri/installer.nsh then copies them beside the exe on the user's
       machine and removes the staging folder);
    3. runs the Tauri bundler to produce the NSIS installer.

  The DLL set mirrors scripts/package-portable.ps1 (the portable distribution)
  -- both are the same dependency-closure-verified set; keep them in sync.

  Usage (from the repo root):
      pwsh -File scripts/build-installer.ps1

  The models are NOT bundled; they download on first run (docs/models.md).

.NOTES
  Windows / x64 only. Requires the build environment (build-env.bat wraps
  vcvars64 + Ninja + LIBCLANG_PATH). The VC++ runtime files are redistributable
  under the Visual Studio licence. The result is unsigned -- see
  docs/distribution.md for the SmartScreen note.
#>
[CmdletBinding()]
param(
  [string]$ReleaseDir = "src-tauri/target/release",
  [string]$LibsDir = "src-tauri/installer-libs",
  [switch]$SkipBuild   # reuse an existing release build (faster iteration)
)

$ErrorActionPreference = 'Stop'

function Fail($msg) { Write-Error $msg; exit 1 }

# The build environment wrapper lives at the repo root. Resolve the root from
# this script's location so the script works regardless of the caller's CWD.
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$buildEnv = Join-Path $root "build-env.bat"
if (-not (Test-Path $buildEnv)) { Fail "build-env.bat not found at repo root ($root)." }

# --- 1. Release build (emits anchor.exe + the sherpa/onnx DLLs) --------------
if (-not $SkipBuild) {
  Write-Host "[1/3] Release build (no bundle) ..." -ForegroundColor Cyan
  & cmd /c "`"$buildEnv`" pnpm tauri build --no-bundle"
  if ($LASTEXITCODE -ne 0) { Fail "Release build failed (exit $LASTEXITCODE)." }
}
$exe = Join-Path $ReleaseDir "anchor.exe"
if (-not (Test-Path $exe)) { Fail "No release build at '$exe'. Remove -SkipBuild to build it." }

# --- 2. Stage the self-contained DLL set into installer-libs/ ----------------
# Delegated to the single-source stager (shared with the CI release workflow).
Write-Host "[2/3] Staging runtime DLLs into $LibsDir ..." -ForegroundColor Cyan
& (Join-Path $PSScriptRoot "stage-installer-libs.ps1") -ReleaseDir $ReleaseDir -OutDir $LibsDir
if ($LASTEXITCODE -ne 0) { Fail "Staging the runtime DLLs failed (exit $LASTEXITCODE)." }

# --- 3. Bundle the NSIS installer --------------------------------------------
# The installer-libs resource lives ONLY in tauri.installer.conf.json (merged in
# here), never in the base config — otherwise every plain `cargo build`/clippy/CI
# would fail its build.rs resource-glob check on a checkout without the staged,
# build-generated DLLs.
Write-Host "[3/3] Bundling the NSIS installer ..." -ForegroundColor Cyan
& cmd /c "`"$buildEnv`" pnpm tauri build --config src-tauri/tauri.installer.conf.json"
if ($LASTEXITCODE -ne 0) { Fail "Tauri bundle failed (exit $LASTEXITCODE)." }

$nsis = Join-Path $ReleaseDir "bundle/nsis"
$setup = Get-ChildItem $nsis -Filter "*-setup.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($setup) {
  $sizeMb = [math]::Round($setup.Length / 1MB, 1)
  Write-Host "Installer ready: $($setup.FullName) (~$sizeMb MB)" -ForegroundColor Green
} else {
  Write-Host "Bundle finished; check $nsis" -ForegroundColor Green
}
Write-Host "Models are not bundled; they download on first run. Unsigned -- first launch shows SmartScreen. See docs/distribution.md." -ForegroundColor DarkYellow
