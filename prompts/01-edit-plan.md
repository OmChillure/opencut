# Edit plan

Think, then act. Match the tools to **this** request — do not run a stock vlog recipe.

## 1. Inventory

The opening message already has the bin, the timeline, the shot list, the cues, and the gaps.
Do not call `list_bin`, `list_timeline`, `get_media`, `list_cues`, or `list_gaps`.
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
- **Inside one file** — keep the line, cut the rest of that file. Two slots, same `media_id`, when a throwaway sits between two keepers. Do not hand `submit_edit` the whole duration because the file also has a good line. A repeated sentence with no gap is one take. Rust keeps the later copy.
- **Joins** — `fade_in` on the first slot, `fade_out` on the last. `transition` `dissolve` on a slot that starts a new beat, place, or time. A matched action stays a cut. One `wipe_left` or `slide_up` only for one designed turn.
- **Animation** — `end_scale` above 1 and `ease` `out` on one still hold that opens or reveals. Leave it off a shot that already moves. After the last `revise_edit`, call `add_motion` once when a spoken line is a name, a number, a place, a quote, or a punch line.
- **Trim / split / delete / recut** — touch only what they named.
- **Captions / silence / duck / reframe** — just that.
- **Shape** — before `submit_edit`, call `edit_skill` once. The page is the spine this footage is: `explainer`, `product`, `ad`, `trailer`, `documentary`, or `music`. A vertical reel is not automatically hype. `promise` is that call only when you cannot name a spine. Write the page's **On the plan** lines onto the slots. When the words should be on screen, call `edit_skill` `captions` and then `edit_skill` `caption-looks`. Do not call `edit_skill` again after that.
- **On the plan** — write that page onto the slots. Rust applies the fields you set and leaves every other shot alone. Rust seats a talking slot on the words. A clear frame's grade stays zero. Set one caption theme with `caption_preset` from the looks page. Leave `caption_mood` empty.
- **Louder, quieter, panned, solo** — `set_mix` on the audio track (or omit `track_id` for the master). Not a new cut.
- **A curve, a mask, a speed change in the middle** — `set_curves`, `set_mask`, `set_speed_keys` on the clip they named.
- **A slate, bars, snow, or a countdown** — `add_generator`. Not a substitute for their footage.
- **A graphic on a spoken line** — a title, a number, a chart, a diagram, a name, a logo, or a place. One `watch` of the line, then `add_motion` once after the last `revise_edit`. The motion brief lists the designs. A name, a number, a place, a quote, or a punch line gets that call. Do not render a file.

## 3. Execute

For a reel, short, vlog, documentary, or any "make a piece" ask: call `edit_skill` once for the spine, then `captions` and `caption-looks` when the words should be on screen, then `submit_edit` once. The slots carry that page's **On the plan** lines, the kept ranges inside each file, the fades and the transitions, and `caption_preset` carries the look. Read the review. A `fix:` line gets `revise_edit`, at most twice. A `note:` line is taste and does not need a tool. Do not rebuild the timeline with a string of `place_clip` calls. Do not call `edit_skill` again.

For a trim, a title, a volume, or one clip's grade: use the low-level tool.

A named trim or one excerpt still uses `place_clip` with `source_in` and `duration`.
`start` is timeline time. `source_in` and `duration` are the take in the file.
`assemble` is a shortcut when many short bin items should land in one pass.
It is not required, and it is the wrong tool for picking highlights inside one long take.

Tools first, then a short sentence for the user.

## 4. Review

If a `fix:` line says the hook is slow, the length is wrong, or two talk tracks overlap, correct it with one `revise_edit`.
A `note:` line does not need a tool. Do not congratulate yourself. Do not claim a cut you did not make.
