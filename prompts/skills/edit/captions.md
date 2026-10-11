Adapted from `captions-media-accessibility` (MIT). Copyright (c) 2026 generative-media-skills contributors.

# Captions

The words are the transcript. You do not write them. You decide whether they go on screen, and then you pick one preset.

Before the look:

- Keep the spoken words, in order. Do not paraphrase. Drop a line that is only filler.
- One or two lines. Break at a phrase. Rust already splits a long line.
- Time the line to the words. It appears when they are said and leaves when they end.
- Keep the line off a mouth, a product, a chart, and text already in the picture.
- One look for the whole video. Do not give the next line a different face.
- Motion stays small unless the footage is a hit they asked to punch. A pop on every word is not a caption.
- High contrast. A light line needs a dark edge. A dark line needs a light edge.

Then call `edit_skill` with `caption-looks` once. Read the footage. A bright picture, a quiet interview, and a reel that should hit are not the same id. Set `caption_preset` to that one id.

On the plan: `captions: true` only when the words should be on screen. Set `caption_preset`. Leave `caption_mood` empty. Do not set `caption_look` when a preset is set. Type stays in front of the picture. A title, a number, or a chart is `add_motion`, not a caption.
