# Contributing to Anchor

Thanks for your interest. Anchor is a local-first, privacy-respecting notes
overlay for calls — the bar for anything that merges is **quality and honesty
first**: no half-finished features, no insecure patterns, no fabricated claims.

## Ground rules

- **Be honest in code and copy.** "Done" means built, tested, and exercised — not
  "looks right from the diff." Never present planned behaviour as shipped.
- **Keep it local-first.** The three network egress points (a bring-your-own-key
  LLM call, a user-initiated page fetch, and first-run model downloads) are the
  only ones — see [SECURITY.md](SECURITY.md). Don't add telemetry or phone-home.
- **No prose the user reads aloud, no stealth features.** Those are product
  non-goals, not features waiting to be added.
- **Accessibility and semantics on every UI change** — semantic HTML, focus
  states, keyboard nav, contrast.

## Platform

Windows 11 is the first-class target: audio capture uses WASAPI directly.
macOS and Linux are on the roadmap but not built yet.

## Prerequisites

- **Rust** (stable, `x86_64-pc-windows-msvc`) and **Node.js** with **pnpm**.
- **Visual Studio 2022 Build Tools** with the C++ workload (the prebuilt
  `sherpa-onnx` static libs need the VS2022 STL).
- **CMake** and **Ninja** (to build the embedded `llama.cpp` and `speexdsp`).
- **LLVM/Clang** (`libclang`) for `bindgen`.
- Build the embedded ggml **without OpenMP** so it doesn't clash with the ONNX
  Runtime's OpenMP in one process: set `CMAKE_ARGS=-DGGML_OPENMP=OFF`.

A convenience wrapper that sets this environment is kept out of the repo because
its paths are machine-specific; a minimal version looks like:

```bat
:: put your own paths here
call "…\VC\Auxiliary\Build\vcvars64.bat"
set "LIBCLANG_PATH=…\LLVM\bin"
set "CMAKE_GENERATOR=Ninja"
set "CMAKE_ARGS=-DGGML_OPENMP=OFF"
%*
```

## Getting started

```bash
pnpm install
pnpm tauri dev        # run the dev build (needs the environment above)
```

## Before you open a PR — the gate

Everything must be green, with zero warnings introduced by your change:

```bash
cargo clippy --all-targets -- -D warnings   # Rust: no warnings
cargo test                                  # Rust unit + integration tests
pnpm exec tsc --noEmit                       # TypeScript: no errors
pnpm exec eslint src                         # frontend lint: clean
```

Then actually run the app and exercise the change — audio, overlay, or the
dashboard, whichever you touched. UI changes are checked at the common
breakpoints and in both light and dark themes.

## Commit and PR style

- Small, focused commits with honest, descriptive English messages.
- One logical change per PR; describe what you changed and how you verified it.
- Match the surrounding code's style, naming, and comment density.

## Reporting bugs and ideas

Open an issue with clear steps to reproduce (for bugs) or the problem you're
trying to solve (for features). Security issues go through
[SECURITY.md](SECURITY.md), **not** a public issue.

By contributing you agree your work is licensed under the project's
[AGPL-3.0](LICENSE).
