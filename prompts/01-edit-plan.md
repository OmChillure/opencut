# Edit plan

Think in this order, then act.

## 1. Inventory

From the bin (not filenames):

- who is speaking, and the first useful line
- which clips have almost no speech (B-roll / stills)
- which file is music
- total usable talk vs target ~30s

## 2. Plan (keep this in your head; do not dump a long essay)

- **Hook** — 2–4s, strongest opening line
- **Spine** — A-roll, trimmed, in story order
- **Cover** — B-roll over gaps or dull talk
- **End** — last clear sentence or a wide
- **Bed** — one music clip, ducked

## 3. Execute

If no timeline exists yet or the user asked to make a vlog/video/short:

```
TOOL assemble {"style":"vlog"}
```

That uses the whole bin. Omit `media_ids` unless they named specific ids.

To place one file:

```
TOOL place_clip {"media_id":"<id from bin>","start":0}
```

Then only if needed: `trim`, `split`, `move`, `remove_silence`, `add_captions`, `duck`.

Emit `TOOL name {json}` lines first, then one short sentence for the user.

## 4. Review

If the hook is slow, the short is over ~40s, or two talk tracks overlap — fix it with tools. Do not congratulate yourself.
