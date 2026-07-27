# Models

Anchor runs entirely on-device. Nothing is bundled in the download — models are
fetched on first use (with an integrity check) into the app's own data folder,
then cached for offline use. Licenses are in [THIRD_PARTY.md](../THIRD_PARTY.md).

## Speech recognition (pick one in Settings)

| Choice | Model | Languages | Best for |
|--------|-------|-----------|----------|
| **Automatic** (default) | Multilingual streaming | EN · ES · RU · UK · DE | Almost everyone |
| **Multilingual** | Nemotron-3.5-ASR-Streaming | EN · ES · RU · UK · DE | Non-English or mixed calls |
| **English — fastest** | Nemotron speech-streaming EN | EN | English-only, lightest load |
| **Compatibility** | Parakeet-TDT-0.6b-v3 (offline) | 25 European languages | Slower CPUs |

The streaming models react from the first words. The Compatibility model is
offline; Anchor drives it as pseudo-streaming with a *LocalAgreement-2* policy
(re-decode a growing buffer, commit only the prefix two consecutive decodes
agree on) so it still tracks a live conversation. Each option is downloaded only
if you choose it.

You can also set a **language per session**; the multilingual model is steered
to it, or auto-detects when left on "Automatic".

## Retrieval embeddings

**EmbeddingGemma-300m** (multilingual, 256-dim for the hot loop) powers hybrid
retrieval — it's what makes English cards match Russian, Spanish, Ukrainian, or
German speech. `multilingual-e5-small` is a lighter fallback.

## Assembly (optional, for unexpected questions)

Only runs when a question falls outside your prepared cards, and only if you
enable it. Two ways:

- **Local, free, in-process** — a GGUF model (Qwen3-1.7B by default; Phi-4-mini
  and NuExtract-2.0-2B selectable), one-click download and switch.
- **API, bring-your-own-key** — any OpenAI-compatible endpoint; the key lives in
  your OS credential manager.

## Footprint

The speech model and the embedding model are the resident cost during a call;
together the running app sits around ~2 GB of RAM with everything live. The
optional local assembly model is loaded on demand and released after use.
