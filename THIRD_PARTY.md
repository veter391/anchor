# Third-Party Notices

Anchor stands on open models and libraries. This file records the notable ones
and their licenses. The **models are not bundled** — they are downloaded on
first run (with an integrity check) into the app's own data folder, so their
weight-redistribution terms don't apply to this repository. You accept each
model's terms at download time.

## Models (downloaded at first run)

| Model | Role | License |
|-------|------|---------|
| NVIDIA Nemotron-3.5-ASR-Streaming-0.6B | Streaming speech-to-text (multilingual) | OpenMDW-1.1 |
| NVIDIA Nemotron speech-streaming EN-0.6B | Streaming speech-to-text (English, faster) | OpenMDW-1.1 |
| NVIDIA Parakeet-TDT-0.6b-v3 | Offline speech-to-text fallback (25 European languages) | CC-BY-4.0 |
| Google EmbeddingGemma-300m | Text embeddings (retrieval) | Gemma Terms of Use |
| Qwen3-1.7B | Optional local assembly model (default) | Apache-2.0 |
| Phi-4-mini | Optional local assembly model | MIT |
| NuExtract-2.0-2B | Optional local assembly model | MIT |

- **NVIDIA** models: OpenMDW-1.1 (Linux Foundation) is permissive with a notice
  retention on weight redistribution; Parakeet is CC-BY-4.0 (attribution).
- **EmbeddingGemma** is under the Gemma Terms of Use — its weights are never
  shipped inside this project; they are fetched under your own acceptance on
  first run. `multilingual-e5-small` (MIT) is a fallback for weak hardware.

## Libraries

The Rust and JavaScript dependencies are open source under permissive licenses
(mostly **MIT** and **Apache-2.0**). The complete, authoritative list with exact
versions is in `src-tauri/Cargo.lock` and `pnpm-lock.yaml`. A few load-bearing
native ones:

| Library | Purpose | License |
|---------|---------|---------|
| sherpa-onnx | Speech-recognition runtime (ONNX) | Apache-2.0 |
| ONNX Runtime | Inference backend for ASR + embeddings | MIT |
| fastembed | Local text embeddings | Apache-2.0 |
| llama.cpp / llama-cpp-2 | Embedded local LLM | MIT |
| speexdsp (via aec-rs) | Acoustic echo cancellation | BSD (Xiph) / MIT |
| wasapi | Windows audio capture | MIT / Apache-2.0 |
| SQLite (via rusqlite) + sqlite-vec | Local storage + vector search | Public Domain / Apache-2.0 / MIT |
| Tauri | Desktop shell | MIT / Apache-2.0 |
| React | Dashboard UI | MIT |

If you spot a license that needs a fuller notice here, please open an issue.
