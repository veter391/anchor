# Architecture

Anchor is a Tauri 2 desktop app with a Rust core. The core owns everything on
the hot path — audio, speech recognition, the match engine, hit-testing — and
the web view renders. All of it runs locally.

```
Microphone (you) ─┐                          ┌─ Overlay (one card, large type,
                  ├─► capture ─► ASR ─► match │   click-through, under the webcam)
System audio ─────┘   (WASAPI)  (engine) engine└─ Dashboard (sessions, cards,
   (the other side, via loopback)                  transcript, coverage, settings)
```

## Capture — two channels, no diarisation

Two audio streams are captured through WASAPI: your **microphone** is you, and a
**render-endpoint loopback** is the other side. Speaker identity is true by
construction — no diarisation model, and it works with headphones. Both streams
are 16 kHz mono, timestamped from the same system performance counter (QPC) so
they share one timeline.

The capture path is resilient: if the default audio device changes mid-call
(a Bluetooth headset switching profiles, a headphone unplug), the affected
stream is rebuilt in place rather than dying.

### Acoustic echo cancellation

Without headphones, the microphone hears the other side out loud from the
speakers, so the same voice lands on both channels. Because the loopback stream
already holds that far-end audio, Anchor uses it as a reference, time-aligns it
to the microphone, and cancels the echo (speexdsp) before recognition — so your
"me" channel carries your voice, not the other side's.

## Speech recognition

The match engine reacts from the first words, so the primary recognizer is a
streaming model that emits word-level partials. Anchor ships a small set of
genuinely-good on-device models the user can choose between:

- **Multilingual** streaming (English, Spanish, Russian, Ukrainian, German) —
  the default, with per-session language selection or auto-detect.
- **English-only** streaming — the fastest option when every call is English.
- **Compatibility (offline)** — an offline model driven as pseudo-streaming with
  a *LocalAgreement-2* policy (re-decode a growing buffer, emit only the prefix
  two consecutive decodes agree on), for CPUs where streaming struggles.

All three sit behind one interface, so the rest of the pipeline is unaware of
which is live. Models are downloaded on first run, never bundled.

## The match engine

Two rolling windows (~10 s) track the conversation: the **other side's** speech
drives which card is shown, **your** speech drives which points are covered.

- **Retrieval is hybrid**: dense embeddings (a multilingual model, one space for
  both the rolling window and the cards) fused with SQLite FTS5 / BM25 via
  Reciprocal Rank Fusion. The keyword leg is the insurance for proper nouns,
  product names, and numbers — exactly the things that escape under pressure.
  It is genuinely cross-lingual: English cards retrieve from Russian, Spanish,
  Ukrainian, or German speech.
- **Hysteresis, not nearest-neighbour**: a challenger card must beat the current
  one by a margin, hold for a few ticks, and respect a cooldown — so the card
  doesn't flicker on a stray word.
- **Coverage is matching, not a sequence**: each bullet is an independent
  covered/uncovered flag, so you can answer out of order.

The language model is never in this loop; retrieval is local, sub-tick, and
cannot hallucinate.

## Assembly (the unexpected question)

When the best match falls below a confidence threshold, Anchor assembles a card
of 4–6 bullets, grounded first in your own material. This is the only place a
language model runs, and it is optional:

- **Local (free):** an embedded GGUF model runs in-process (no server).
- **API (bring-your-own-key):** any OpenAI-compatible endpoint.

Every assembled bullet is checked against your material; anything drawn from
beyond it is explicitly labelled "model knowledge", never silently presented as
yours.

## Storage

One local SQLite file: cards, bullets (with per-length variants), embeddings
(`sqlite-vec`), an FTS5 index, sessions, transcript (text only), and coverage.
Audio is never written to disk. Everything is written live, so a crash
mid-session still leaves a usable coverage report.

## The overlay

A frameless, always-on-top window placed under the webcam: one card at a time,
large type, no scrollbars, no spinners. It is click-through on empty regions (a
Rust cursor poll toggles cursor pass-through by hit-zone), and can be excluded
from a screen share (off by default; paired with a "Show notes" button — there
is no stealth mode).

## Stack

Tauri 2 + Rust core; React + TypeScript dashboard; `sherpa-onnx` (ONNX Runtime)
for speech, `fastembed` for embeddings, `llama.cpp` for the optional local LLM,
`rusqlite` + `sqlite-vec` + FTS5 for storage, `wasapi` for audio.
