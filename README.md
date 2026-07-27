# Anchor

**Prepared, not prompted.**

Anchor is a live notes overlay for calls. You prepare your own cards; Anchor puts the right one in front of you the moment the topic comes up, and tracks which points you have already covered.

**No stealth mode. No generated answers to read aloud. Nothing leaves your machine.** It is useless until you have done the work — that is the point.

---

## What it is for

Client calls. Team calls. Architecture reviews. Investor conversations. Interviews — on either side of the table. Any online meeting where you know the material but the names escape you under pressure.

The problem is narrow and real: **you know the thing, but the term will not come.** Your notes are in a forty-page document, finding the right page live is impossible, your eyes go hunting, and you present worse than you are.

Anchor holds six anchors in front of your eyes, large, at the moment you need them — reacting from the first words spoken, yours or theirs.

## How it works

1. **You study.** Anchor ships a study prompt ([prompts/1_LEARN.md](prompts/1_LEARN.md)). Take it to whatever AI you learn with and actually learn the material. Anchor will not do this for you.
2. **You build cards.** Six anchors per card, keywords not sentences ([prompts/2_BUILD_CARDS.md](prompts/2_BUILD_CARDS.md)). If a bullet reads like prose, Anchor warns you: you have accidentally written a script, and a script will fail you.
3. **You take the call** ([prompts/3_LOAD.md](prompts/3_LOAD.md)). Anchor listens to both sides locally — your microphone is you, the system audio is them, so it never confuses speakers. It jumps to the right card and highlights the next anchor you have not covered. An unexpected question gets a card assembled live, grounded in your own material first — and anything drawn from beyond your material is explicitly labelled. You always know what is yours.
4. **You review.** Full transcript (text only — audio is never written to disk) and a coverage report: which anchors you hit, which you missed. The misses are your next study list.

## What it is not

- It does not write sentences for you to read aloud. Bullets and keywords only, and machine-assembled bullets are always visibly marked.
- It has no stealth mode, no "undetectable" mode, no process disguise. There is a screen-share exclusion toggle, **off by default**, for presenting your screen without your private notes on the projector — and a **Show notes** button to deliberately share your cards with the other side.
- It does not send your notes or your audio anywhere. Local first; the only network calls are the optional bring-your-own-key LLM step and the page fetch you explicitly request.
- It is not a substitute for knowing your subject.

## How it differs from things that look similar

- **Talking-point trackers** (e.g. Talking Points Tracker) check off one linear outline for a solo speaker. Anchor navigates a corpus of many cards, driven by what the *other side* raises on a two-sided call.
- **Meeting agenda tools** (e.g. Colibri) tick a flat shared agenda in the cloud. Anchor tracks bullet-level coverage of *your own prepared material*, locally.
- **Sales battle cards** (e.g. Clari Copilot) fire on keywords and push the vendor's playbook. Anchor matches meaning and surfaces only what you wrote.
- **Interview "copilots"** generate answers and hide from screen capture. Anchor does neither — this repository is something you can show the person across the table.

## Status

Early — in design and pre-alpha development. The stack: Tauri 2 with a Rust core, dual-channel local audio capture, streaming on-device speech recognition, hybrid semantic + keyword retrieval in SQLite, everything local. English, Spanish, Russian, Ukrainian and German are the launch languages, and cards may be written in a different language than the call.

## Licence

[AGPL-3.0](LICENSE). Use it, read it, fork it — but derivatives stay open.

---

*Anchor does not know anything until you teach it. That is not a limitation. That is the product.*
