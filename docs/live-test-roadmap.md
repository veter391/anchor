# Anchor — live call test (Zoom with a friend)

Only **you** run Anchor. Your friend is just the other side of the Zoom call —
their voice reaches Anchor as system audio. No setup on their end.

## A. Setup (once, ~10 min)

1. Double-click `Anchor_0.6.0_x64-setup.exe` → it installs (no admin). On the
   SmartScreen warning: **More info → Run anyway** (build is unsigned, expected).
2. First launch → accept the consent screen → wait for the model download
   (~1 GB, progress bar). This is one-time.
3. **Optional — smarter "unexpected question" cards.** Settings → Engine →
   *API* → OpenRouter → paste your OpenRouter key → pick a fast model
   (**Gemini Flash Lite** is a great choice). Skip this and it uses the free
   built-in local model — both work.
4. Cards tab → paste a few notes / bullet lists you'd want in front of you.
   Six keywords per card, not sentences.

## B. Audio check (before dialing — 1 min)

5. Sessions → New session → open it → **Check audio**.
6. Say something → the **You (microphone)** dot goes green. Have your friend
   talk on Zoom → the **Other side (system audio)** dot goes green.
   - Both green = good. One stays "silent" = fix that device before the call.

## C. Call #1 — WITH headphones

7. **Go live**, then have a real ~5-min conversation across a few of your card
   topics.
8. Watch: the right card jumps as each topic comes up; bullets get ticked as you
   cover them; live text scrolls.
9. Mid-call, press **Ctrl+Shift+Space** → the orange "unexpected question" card
   (3 anchors) should pop up. Note if it appears.
10. **End call** → the coverage report shows which anchors you hit / missed.

## D. Call #2 — WITHOUT headphones (the echo test)

11. Same again on **open speakers** (no headphones), short call.
12. Watch one thing specifically: while your **friend** is talking, does **your**
    side wrongly tick off bullets (as if you'd said them)? Yes/No — tell me.
    (That's the acoustic-echo case, findings H1/H2.)

## E. Device switch (30 sec)

13. Mid-call, connect a **Bluetooth headset** (or unplug/replug wired ones).
14. Watch: does audio keep working, or does a **dead-channel warning** appear?
    Does it recover on its own? Tell me what happened.

## F. Send me back

15. The file `%LOCALAPPDATA%\AnchorData\audio-diag.txt` (paste its contents).
    Paste `%LOCALAPPDATA%` into Explorer's address bar → `AnchorData` folder.
16. A line per step on what you saw — especially **8, 9, 12, 14**.

That's the whole test. Steps C–E are the real "battlefield" validation; F is what
lets me fix anything that showed up. No numbers to record by hand — the app and
the diag file carry them.
