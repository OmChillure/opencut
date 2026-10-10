# Edit plan

Think, then act. Match the tools to **this** request — do not run a stock vlog recipe.

## 1. Inventory

The opening message already has the bin, the timeline, the shot list, and the gaps.
Do not call `list_bin`, `list_timeline`, `get_media`, `list_cues`, or `list_gaps` to read them again.
Call one only when a line you need was cut off.
A row without a scale is the one to `watch`.

From that list (not filenames):

- who is speaking, and which lines are worth keeping (with source times)
- which ranges are the person, the place, or a screen, and which are wide or close
- which clips have almost no speech (B-roll / stills)
- which file is music
- what is already on the timeline (`source_in` is the in-point in the source)

## 2. Plan (keep this in your head; do not dump a long essay)

Decide the shape from the ask. You choose the in-points. Do not walk the transcript in order and keep each sentence.

- **Several recordings of one moment** — cut between angles on the word. The other file is not the next sentence.
- **Reel / short / tiktok / highlight / a named duration** — a short, as the mission says. Vertical if they said reel or tiktok.
- **Edit this video / the whole import / cover the source** — keep the piece. Not a highlight reel.
- **One long source and they asked for a reel or a short** — several excerpts, not one short bite and not the whole file.
- **Trim / split / delete / recut** — touch only what they named.
- **Captions / silence / duck / reframe** — just that.
- **Shape** — before `submit_edit`, call `edit_skill` once. The page is the spine this footage is: `explainer`, `product`, `ad`, `trailer`, `documentary`, or `music`. A vertical reel is not automatically hype. `promise` is that call only when you cannot name a spine. Write the page's **On the plan** lines onto the slots. Do not call `edit_skill` again.
- **On the plan** — write that page onto the slots. Rust applies the fields you set and leaves every other shot alone. Rust seats a talking slot on the words. A clear frame's grade stays zero. Set one caption theme with `caption_mood` or one `caption_look`, as the director brief says.
- **Louder, quieter, panned, solo** — `set_mix` on the audio track (or omit `track_id` for the master). Not a new cut.
- **A curve, a mask, a speed change in the middle** — `set_curves`, `set_mask`, `set_speed_keys` on the clip they named.
- **A slate, bars, snow, or a countdown** — `add_generator`. Not a substitute for their footage.
- **A graphic on a spoken line** — a title, a number, a chart, a diagram, a name, a logo, or a place. One `watch` of the line, then `add_motion` once after the last `revise_edit`. The motion brief lists the designs. Do not render a file.

## 3. Execute

For a reel, short, vlog, documentary, or any "make a piece" ask: call `edit_skill` once, then `submit_edit` once. The slots carry that page's **On the plan** lines. Read the review. A `fix:` line gets `revise_edit`, at most twice. A `note:` line is taste and does not need a tool. Do not rebuild the timeline with a string of `place_clip` calls. Do not call `edit_skill` again.

For a trim, a title, a volume, or one clip's grade: use the low-level tool.

A named trim or one excerpt still uses `place_clip` with `source_in` and `duration`.
`start` is timeline time. `source_in` and `duration` are the take in the file.
`assemble` is a shortcut when many short bin items should land in one pass.
It is not required, and it is the wrong tool for picking highlights inside one long take.

Tools first, then a short sentence for the user.

## 4. Review

If a `fix:` line says the hook is slow, the length is wrong, or two talk tracks overlap, correct it with one `revise_edit`.
A `note:` line does not need a tool. Do not congratulate yourself. Do not claim a cut you did not make.
