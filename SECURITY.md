# Security & Privacy

Anchor is local-first by design. Understanding what it does and does not send is
part of its security model, so both are documented here.

## Privacy architecture

- **Audio is processed in memory and never written to disk.** Only text (the
  transcript) is stored.
- **Everything is stored locally**, in one SQLite file in the app's own data
  folder. No account, no cloud sync, no telemetry.
- **API keys** (for the optional bring-your-own-key LLM) live in the operating
  system's credential manager, never in config files or the database.

### The only network egress

Nothing leaves your machine except these three, each explicit in the UI:

| # | What | When |
|---|------|------|
| 1 | The optional Level-2 LLM call | Only if you enable a bring-your-own-key provider; the provider is shown in the UI. Off by default (a free, fully local model can be used instead). |
| 2 | A page fetch for pre-flight research | Only when you paste a URL and ask Anchor to summarise it. |
| 3 | First-run model downloads | The speech, embedding, and local-LLM models are downloaded on first use (they are not bundled) with an integrity check, then cached offline. |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

- Preferred: use GitHub's private vulnerability reporting on this repository
  (the **Security** tab → *Report a vulnerability*).
- Or email **<CONTACT EMAIL>**.

Please include steps to reproduce and the impact you observed. We aim to
acknowledge reports promptly and will keep you updated as we work on a fix.
Coordinated disclosure is appreciated.

## Scope

In scope: the desktop application and its handling of audio, text, keys, and the
three egress points above. Out of scope: third-party model or provider services
you choose to connect (their own security is governed by their terms).
