# Edit plan

Think, then act. Match the tools to **this** request — do not run a stock vlog recipe.

## 1. Inventory

Call `list_bin` and `list_timeline` first. For a long file, call `get_media`.
Read the shot list (`start-end`, look, `speech` / `silence` / `filler`, words).
`list_cues` is the raw transcript if you still need a line the shot list cut off.

From those tools (not filenames):

- who is speaking, and which lines are worth keeping (with source times)
- which clips have almost no speech (B-roll / stills)
- which file is music
- what is already on the timeline (`source_in` is the in-point in the source)

## 2. Plan (keep this in your head; do not dump a long essay)

Decide the shape from the ask. You choose the in-points. Do not walk the transcript in order and keep each sentence.

- **Several recordings of one moment** — cut between angles on the word. The other file is not the next sentence.
- **Reel / short / tiktok / 30–60s** — hook, spine, cover, end. Vertical if they said reel or tiktok.
- **One long source** — several excerpts, not one short bite and not the whole file.
- **Trim / split / delete / recut** — touch only what they named.
- **Captions / silence / duck / reframe** — just that.
- **On the plan** — grade, grain, a fade, a push, a cover, music volume, captions, and letterbox only where this footage needs them. Rust applies those fields and leaves every other shot alone.
- **Louder, quieter, panned, solo** — `set_mix` on the audio track (or omit `track_id` for the master). Not a new cut.
- **A curve, a mask, a speed change in the middle** — `set_curves`, `set_mask`, `set_speed_keys` on the clip they named.
- **A slate, bars, snow, or a countdown** — `add_generator`. Not a substitute for their footage.
- **A drawing of what they are explaining** — `add_design` on that spoken line, after the last `revise_edit` (a revision clears the timeline). See the start, middle, and end of the line first. `duration` is the whole line, up to 15 seconds. A pattern becomes a full-frame animation of that pattern drawing itself while they talk. A house rises until it fills the frame. `behind` puts the words behind the person.

## 3. Execute

For a reel, short, vlog, documentary, or any "make a piece" ask: call `submit_edit` once. Read the review. If it has `fix:` or `note:` lines, call `revise_edit` at most twice. Do not rebuild the timeline with a string of `place_clip` calls.

For a trim, a title, a volume, or one clip's grade: use the low-level tool.

Tools first, then a short sentence for the user.

Put a **slice** of a source on the timeline:

```
TOOL place_clip {"media_id":"<id>","start":0,"source_in":12.4,"duration":3.8}
```

`start` is timeline time. `source_in` + `duration` are the take in the file.
Repeat for each keep. `clear_timeline` first if you are starting a new cut.

`assemble` is a shortcut when many short bin items should land in one pass.
It is **not** required, and it is the wrong tool for picking highlights inside one long take —
use `place_clip` excerpts for that.

Then only if needed: `trim`, `split`, `slip`, `move`, `remove_silence`, `add_captions`, `duck`, `reframe`.

## 4. Review

If the hook is slow, the length is wrong, or two talk tracks overlap — fix it with tools.
Do not congratulate yourself. Do not claim a cut you did not make.
