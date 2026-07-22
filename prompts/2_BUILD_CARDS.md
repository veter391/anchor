# Anchor — Step 2: Build the cards

Run this **after** you have actually studied with Step 1. Give this file to the same AI, in the same conversation, so it still has everything you covered.

---

We are done studying. Now turn what I learned into **Anchor cards**.

## What an Anchor card is

An Anchor card is **not a script**. It is not an answer. It is a set of **anchors**: keywords that unlock what I already know.

I will see these cards for **less than a second at a time**, out of the corner of my eye, while I am talking. If I have to read a sentence, the card has failed.

## Format, exactly

```markdown
## The question, phrased the way a real person would ask it
tags: comma, separated
lang: en

- Anchor one
- Anchor two
- Anchor three
- Anchor four
- Anchor five
- Anchor six
```

## Hard rules

1. **Maximum six bullets per card.** If a topic needs more, split it into two cards.
2. **Each bullet is under about ten words.** Keywords, not sentences.
3. **No prose. Ever.** If a bullet reads like something I would say out loud verbatim, it is wrong. Rewrite it as the trigger, not the speech.
   - Wrong: "I built an internal multi-agent platform with around 26 agents that handles our daily operations."
   - Right: `Internal platform, 26 agents, runs daily ops`
4. **The heading is the question, not a label.** "Why are you leaving your own company?" not "Leaving".
5. **Bullets are in the order I would naturally say them**, but they do not have to be followed in order. Anchor handles me jumping around.
6. **Include the names I will forget.** This is the whole point. Proper nouns, technical terms, numbers, product names. Those are what escape under pressure, not the ideas.
7. **One card per distinct question.** Do not merge two questions into one card.

## What to produce

Go back over everything we covered and generate a card for **every topic that could realistically come up**, including:

- the obvious questions
- the ones I was weak on (especially those)
- the ones where I need to recall a specific name, number, or term
- the hard ones I would rather not be asked
- a card about **the company or client itself**: what they do, their product, their stack, in six anchors

Output it all as **one markdown file** I can drop straight into Anchor.

Do not editorialise. Do not add a preamble. Just the cards.
