# Edit plan

Think, then act. Match the tools to **this** request — do not run a stock vlog recipe.

## 1. Inventory

Call `list_bin` and `list_timeline` first. For a long file, call `get_media`.
Read the shot list (`start-end`, scale, subject, camera, motion, `q`, `speech` / `silence` / `filler`, words).
A row without a scale is only the coarse look (`wide`, `close`, `action`). `list_cues` is the raw transcript if you still need a line the shot list cut off.

From those tools (not filenames):

- who is speaking, and which lines are worth keeping (with source times)
- which ranges are the person, the place, or a screen, and which are wide or close
- which clips have almost no speech (B-roll / stills)
- which file is music
- what is already on the timeline (`source_in` is the in-point in the source)

## 2. Plan (keep this in your head; do not dump a long essay)

Decide the shape from the ask. You choose the in-points. Do not walk the transcript in order and keep each sentence.

- **Several recordings of one moment** — cut between angles on the word. The other file is not the next sentence.
- **Reel / short / tiktok / highlight / a named duration** — hook, spine, cover, end. A long take may become 30–60s. Vertical if they said reel or tiktok.
- **Edit this video / the whole import / cover the source** — keep the piece. Excerpts in order. Drop ums, dead air, and retakes. At least half of the real speech stays (on a silent film, at least half of the source). Not a 30–60s reel.
- **One long source and they asked for a reel or a short** — several excerpts, not one short bite and not the whole file.
- **Trim / split / delete / recut** — touch only what they named.
- **Captions / silence / duck / reframe** — just that.
- **On the plan** — grade, grain, a fade, a push, a cover, music volume, captions, and letterbox only where this footage needs them. Rust applies those fields and leaves every other shot alone. Rust also seats a talking slot on the words. A shot the model has watched keeps the grade from that frame. A clear frame's grade is zero, so the camera color stays. Leave a slot `grade` empty unless you have `see`n the frame and it needs a correction. Shots in the same light share that correction. Mix the joins: omit `transition` for a hard cut, use `dissolve` when time or place changes, and a wipe or a slide only on one graphic or reel join. `end_scale` is one slow push on a hold whose shot-list motion is `none`, with `ease` `in_out`. A shot that is already moving does not also zoom. The motion brief decides the rest. Set `captions: true` and one caption theme for the whole video: `caption_mood` `clean`, `kinetic`, or `bold`, or one `caption_look` phrase such as `bottom sans fade`. The same face and effect is used on every line. Rust only moves a line to stay off a close mouth and inside the frame.
- **Louder, quieter, panned, solo** — `set_mix` on the audio track (or omit `track_id` for the master). Not a new cut.
- **A curve, a mask, a speed change in the middle** — `set_curves`, `set_mask`, `set_speed_keys` on the clip they named.
- **A slate, bars, snow, or a countdown** — `add_generator`. Not a substitute for their footage.
- **A graphic on a spoken line** — a title, a number, a chart, a diagram, a name, a logo, or a place. See the line, pick a `design`, and call `add_motion` once after the last `revise_edit`. That call returns the design page, renders the clip, and places it. Do not render a file. The next chart uses a different pattern from `chart-bars`, `chart-line`, `chart-pie`, and `chart-race`.

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
