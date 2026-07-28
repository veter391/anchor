<#
.SYNOPSIS
  Stage a self-contained portable Anchor folder from a release build.

.DESCRIPTION
  Anchor ships as a portable extract-and-run folder (see docs/distribution.md).
  A release `Anchor.exe` needs its speech-runtime DLLs AND the Microsoft Visual
  C++ runtime beside it, or it will not start on a machine without the VC++
  Redistributable. This script gathers exactly those files -- verified as the
  complete, self-contained set with a dependency-closure check (2026-07-28) --
  into one folder ready to zip.

  Run the release build first:
      build-env.bat pnpm tauri build --no-bundle
  then this script:
      pwsh -File scripts/package-portable.ps1

  The models are NOT bundled; they download on first run (docs/models.md).

.NOTES
  Windows / x64 only. The VC++ runtime files are redistributable under the
  Visual Studio licence. The result is unsigned -- see docs/distribution.md for
  the SmartScreen note.
#>
[CmdletBinding()]
param(
  [string]$ReleaseDir = "src-tauri/target/release",
  [string]$OutDir = "src-tauri/target/portable/Anchor"
)

$ErrorActionPreference = 'Stop'

function Fail($msg) { Write-Error $msg; exit 1 }

# --- 1. The release binary + its speech-runtime DLLs (emitted next to the exe
#        by the sherpa-onnx `shared` build) -----------------------------------
$exe = Join-Path $ReleaseDir "anchor.exe"
if (-not (Test-Path $exe)) {
  Fail "No release build at '$exe'. Run: build-env.bat pnpm tauri build --no-bundle"
}
$runtimeDlls = @(
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "sherpa-onnx-c-api.dll",
  "sherpa-onnx-cxx-api.dll"
)

# --- 2. Locate the VC++ redistributable (CRT + OpenMP) via vswhere -----------
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio/Installer/vswhere.exe"
if (-not (Test-Path $vswhere)) { Fail "vswhere.exe not found -- install Visual Studio 2022 Build Tools." }
$vsRoot = & $vswhere -latest -products '*' -property installationPath
if (-not $vsRoot) { Fail "No Visual Studio installation found by vswhere." }
$redistRoot = Join-Path $vsRoot "VC/Redist/MSVC"
# Highest version folder that actually has the x64 CRT.
$crtDir = Get-ChildItem $redistRoot -Directory -ErrorAction SilentlyContinue |
  Sort-Object Name -Descending |
  ForEach-Object { Join-Path $_.FullName "x64/Microsoft.VC143.CRT" } |
  Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $crtDir) { Fail "VC143 x64 CRT redist not found under $redistRoot." }
$ompDir = Join-Path (Split-Path $crtDir) "Microsoft.VC143.OpenMP"

# --- 3. Stage -----------------------------------------------------------------
if (Test-Path $OutDir) {
  & icacls $OutDir /reset /t /c /q 2>&1 | Out-Null   # in case a prior run hardened it
  Remove-Item $OutDir -Recurse -Force
}
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

Copy-Item $exe (Join-Path $OutDir "Anchor.exe")
foreach ($d in $runtimeDlls) {
  $src = Join-Path $ReleaseDir $d
  if (-not (Test-Path $src)) { Fail "Missing runtime DLL: $src (was the release built with the sherpa 'shared' feature?)" }
  Copy-Item $src $OutDir
}
Copy-Item (Join-Path $crtDir "*.dll") $OutDir          # msvcp140*, vcruntime140*, concrt140, vccorlib140
Copy-Item (Join-Path $ompDir "vcomp140.dll") $OutDir   # OpenMP runtime (used by the static ONNX Runtime)

$count = (Get-ChildItem $OutDir -File).Count
$sizeMb = [math]::Round((Get-ChildItem $OutDir -File | Measure-Object Length -Sum).Sum / 1MB)
Write-Host "Staged $count files (~$sizeMb MB) into $OutDir" -ForegroundColor Green
Write-Host "Models are not bundled; they download on first run. Zip this folder to distribute." -ForegroundColor Green
Write-Host "Note: unsigned -- first launch shows SmartScreen (More info -> Run anyway). See docs/distribution.md." -ForegroundColor DarkYellow
