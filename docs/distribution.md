# Distribution

Anchor ships as a **portable folder** — extract it anywhere you can write to
(your Desktop, a USB stick, a projects folder) and run `Anchor.exe`. There is no
installer and nothing is scattered across the machine: the app keeps its
database, downloaded models and cache in a single `data/` folder next to the
executable. Deleting the folder removes Anchor completely.

Portable, rather than an installer into `Program Files`, is deliberate:
`Program Files` is not user-writable, so the "everything in one folder next to
the exe" model could not write its own data there.

> Status: **pre-alpha.** There is no published binary yet — build from source
> (see the main [README](../README.md) and [CONTRIBUTING](../CONTRIBUTING.md)).
> This page documents how the portable build is put together and what to expect
> when a release is available.

## What's in the folder

A portable build is self-contained (verified with a dependency-closure check —
every non-system DLL the app loads is present):

- `Anchor.exe`
- The speech runtime: `sherpa-onnx-c-api.dll`, `onnxruntime.dll`,
  `onnxruntime_providers_shared.dll`, `sherpa-onnx-cxx-api.dll`
- The Microsoft Visual C++ runtime those libraries need: `msvcp140*.dll`,
  `vcruntime140*.dll`, `concrt140.dll`, `vccorlib140.dll`, `vcomp140.dll`
  (bundled so you do **not** need to install the VC++ Redistributable separately)

The web view uses the **WebView2 runtime**. It ships with Windows 11; on an
older Windows 10 machine that doesn't have it, install Microsoft's free
"Evergreen WebView2 Runtime" once (a signed Microsoft download). The speech,
embedding and assembly **models are not bundled** — they download on first use
into `data/`, with an integrity check, then cache offline (see
[models.md](models.md)).

To assemble that folder reproducibly after a release build
(`build-env.bat pnpm tauri build --no-bundle`), run
[`scripts/package-portable.ps1`](../scripts/package-portable.ps1) — it gathers
the executable, the speech-runtime DLLs and the VC++ runtime into one staged
folder (the exact self-contained set, verified with a dependency-closure check).

## First run

1. Extract the folder and run `Anchor.exe`.
2. Open **Settings → Speech model** and download a speech model (once).
3. Follow the three steps in the app (Learn → Build cards → Load & go).

The first embedding also downloads its model (~300 MB) in the background.
Altogether a first run pulls roughly 1 GB (the speech model + the embedding
model; the optional local assistant is extra). Everything lands in `data/`,
which Anchor restricts to your user account on Windows.

## SmartScreen (unsigned builds)

The current builds are **not code-signed**, so Windows SmartScreen may show
"Windows protected your PC" the first time you run `Anchor.exe`. This is the
standard warning for any new, unsigned application — not a sign of a problem.
To run it: click **More info**, then **Run anyway**.

A signed build (which removes the warning) requires a code-signing certificate;
that is planned but not yet in place. Until then, the SmartScreen step above is
the expected path, and building from source is always an option.
