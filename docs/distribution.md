# Distribution

Anchor ships two ways from the same release build:

1. **Installer (`Anchor_<version>_x64-setup.exe`)** — the normal way. Double-click
   it and Anchor installs like any desktop app: a Start-menu entry, an
   uninstaller, and a launch shortcut. No admin rights and no separate
   Visual C++ Redistributable are needed.
2. **Portable folder** — extract it anywhere you can write to (Desktop, a USB
   stick, a projects folder) and run `Anchor.exe`. Nothing is registered with
   the system; deleting the folder removes Anchor completely.

Both are self-contained (verified with a dependency-closure check — every
non-system DLL the app loads is bundled) and store everything Anchor creates in
one place next to the executable.

> Status: **pre-alpha.** There is no published binary yet — build from source
> (see the main [README](../README.md) and [CONTRIBUTING](../CONTRIBUTING.md)).
> This page documents how the builds are put together and what to expect when a
> release is available.

## Installer

The installer is a per-user install (Tauri's NSIS bundle in `currentUser`
mode): it installs into `%LOCALAPPDATA%\Anchor` — a folder your own account can
write to — so it needs **no administrator prompt**. That location matters:
Anchor keeps its database, downloaded models and cache in a `data/` folder next
to the executable, and `%LOCALAPPDATA%` is writable, whereas `Program Files`
is not.

What lands next to `anchor.exe` after install:

- `anchor.exe`
- The speech runtime: `sherpa-onnx-c-api.dll`, `onnxruntime.dll`,
  `onnxruntime_providers_shared.dll`, `sherpa-onnx-cxx-api.dll`
- The Microsoft Visual C++ runtime those libraries need: `msvcp140*.dll`,
  `vcruntime140*.dll`, `concrt140.dll`, `vccorlib140.dll`, `vcomp140.dll`
  (bundled so you do **not** need the VC++ Redistributable installed separately)

Those DLLs are carried inside the installer as a bundled resource and copied
beside the exe by the installer's post-install hook
([`src-tauri/installer.nsh`](../src-tauri/installer.nsh)); uninstalling removes
them. Only the DLLs are extra — the models are not bundled (see
[First run](#first-run)).

To build the installer reproducibly, run
[`scripts/build-installer.ps1`](../scripts/build-installer.ps1) from the repo
root:

```powershell
pwsh -File scripts/build-installer.ps1
```

It does a release build, stages the exact self-contained DLL set into
`src-tauri/installer-libs/` (that folder is gitignored — the DLLs are
build-generated and live under a machine-specific VS install, so they are staged
fresh, never committed), and runs the Tauri bundler. The finished installer
lands in `src-tauri/target/release/bundle/nsis/`.

## Portable folder

A portable build contains exactly the same files as the installed folder —
`Anchor.exe` plus the speech-runtime and VC++ DLLs above — but with nothing
registered on the machine. Assemble it reproducibly after a release build
(`build-env.bat pnpm tauri build --no-bundle`) with
[`scripts/package-portable.ps1`](../scripts/package-portable.ps1), then zip the
staged folder to distribute.

## WebView2

Both builds use the **WebView2 runtime** for their UI. It ships with Windows 11;
on an older Windows 10 machine that doesn't have it, install Microsoft's free
"Evergreen WebView2 Runtime" once (a signed Microsoft download).

## First run

1. Install (or extract the portable folder) and launch Anchor.
2. Accept the one-time consent screen.
3. Anchor downloads its default speech model automatically (a progress bar shows
   it) — you do not pick or click anything.
4. Follow the three steps in the app (Learn → Build cards → Load & go).

The first embedding also downloads its model (~300 MB) in the background.
Altogether a first run pulls roughly 1 GB (the speech model + the embedding
model; the optional local assistant is extra), each with an integrity check,
then caches offline (see [models.md](models.md)). Everything lands in `data/`,
which Anchor restricts to your user account on Windows.

## SmartScreen (unsigned builds)

The current builds are **not code-signed**, so Windows SmartScreen may show
"Windows protected your PC" the first time you run the installer or the portable
`Anchor.exe`. This is the standard warning for any new, unsigned application —
not a sign of a problem. To proceed: click **More info**, then **Run anyway**.

A signed build (which removes the warning) requires a code-signing certificate;
that is planned but not yet in place. Until then, the SmartScreen step above is
the expected path, and building from source is always an option.
