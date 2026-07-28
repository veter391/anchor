<#
.SYNOPSIS
  Stage the self-contained native runtime DLL set that must ship beside
  anchor.exe (speech runtime + Microsoft Visual C++ runtime).

.DESCRIPTION
  This is the SINGLE source of truth for "which DLLs ship with Anchor" — the
  dependency-closure-verified set. Both scripts/build-installer.ps1 (local) and
  .github/workflows/release.yml (CI) call it, so the two never drift.

  It is environment-agnostic: it assumes a release build already exists at
  -ReleaseDir (it does NOT build) and that vswhere can find the VS redist. It
  does not depend on build-env.bat, so it works under CI's msvc-dev-cmd too.

  The models are NOT staged; they download on first run (docs/models.md).

.PARAMETER ReleaseDir
  Where the release anchor.exe + sherpa/onnx DLLs live (e.g. src-tauri/target/release).

.PARAMETER OutDir
  Where to stage the DLL set (cleaned + recreated).

.NOTES
  Windows / x64 only. The VC++ runtime files are redistributable under the
  Visual Studio licence.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$ReleaseDir,
  [Parameter(Mandatory = $true)][string]$OutDir
)

$ErrorActionPreference = 'Stop'
function Fail($msg) { Write-Error $msg; exit 1 }

if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

# 1. Speech runtime, emitted beside the exe by the sherpa-onnx 'shared' build.
$runtimeDlls = @(
  'onnxruntime.dll',
  'onnxruntime_providers_shared.dll',
  'sherpa-onnx-c-api.dll',
  'sherpa-onnx-cxx-api.dll'
)
foreach ($d in $runtimeDlls) {
  $src = Join-Path $ReleaseDir $d
  if (-not (Test-Path $src)) { Fail "Missing runtime DLL: $src (was the release built with the sherpa 'shared' feature?)" }
  Copy-Item $src $OutDir
}

# 2. VC++ redistributable (CRT + OpenMP), located via vswhere.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (-not (Test-Path $vswhere)) { Fail 'vswhere.exe not found -- install Visual Studio 2022 Build Tools.' }
$vsRoot = & $vswhere -latest -products '*' -property installationPath
if (-not $vsRoot) { Fail 'No Visual Studio installation found by vswhere.' }
$redistRoot = Join-Path $vsRoot 'VC/Redist/MSVC'
$crtDir = Get-ChildItem $redistRoot -Directory -ErrorAction SilentlyContinue |
  Sort-Object Name -Descending |
  ForEach-Object { Join-Path $_.FullName 'x64/Microsoft.VC143.CRT' } |
  Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $crtDir) { Fail "VC143 x64 CRT redist not found under $redistRoot." }
$ompDir = Join-Path (Split-Path $crtDir) 'Microsoft.VC143.OpenMP'
Copy-Item (Join-Path $crtDir '*.dll') $OutDir          # msvcp140*, vcruntime140*, concrt140, vccorlib140
Copy-Item (Join-Path $ompDir 'vcomp140.dll') $OutDir   # OpenMP runtime (used by the static ONNX Runtime)

$count = (Get-ChildItem $OutDir -File).Count
Write-Host "staged $count DLLs into $OutDir" -ForegroundColor Green
