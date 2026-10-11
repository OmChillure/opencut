Adapted from the embedded-captions catalog in heygen-com/hyperframes (Apache-2.0).

# Caption looks

Pick one id. Rust paints that color, edge, font, place, and effect on every line, in front of the picture. It does not put type behind the person, and it does not render HTML. Do not mix two ids. Leave `caption_mood` empty.

Match the footage, not a style word:

- Bright picture: `ink` or `inkwater` (black type, light edge).
- Interview, testimony, a calm talking piece: `documentary` or `anchor`.
- Warm and quiet: `cream`, `brush`, or `editorial`.
- A reel, sport, or a line they asked to hit: `loud`, `ordnance`, `stomp`, or `thunder`.
- Dark night or music: `neon` or `neonsign`.
- A screen, a console, or a tape: `terminal`, `glitch`, `vhs`, or `hologram`.
- Unsure: `anchor`.

Each id paints as follows. Place moves off a close mouth.

- `cream` — lower, serif, fade, cream, dark edge. Warm monologue.
- `ink` — bottom, serif, still, black, light edge. Bright picture.
- `editorial` — lower, serif, fade, bone, dark edge. Fashion or a lyric line.
- `keynote` — lower, sans, fade, white, dark edge. A launch or an explainer.
- `documentary` — bottom, sans, still, bone, dark edge. Interview or journalism.
- `loud` — lower, display, pop, white, dark edge. A reel that should hit.
- `neon` — lower, display, fade, cyan, dark edge. Dark nightlife.
- `glitch` — lower, mono, fade, mint, dark edge. A corrupted feed, still readable.
- `chrome` — lower, display, fade, bone, dark edge. Product, in front.
- `velocity` — lower, display, pop, bone, dark edge. Fast, still one line.
- `anchor` — bottom, sans, fade, white, dark edge. The readable default.
- `arcade` — lower, display, pop, mint, dark edge. A game.
- `aurora` — lower, serif, fade, mint, dark edge. Soft color.
- `biolume` — lower, sans, fade, mint, dark edge. A glow on a dark shot.
- `brush` — lower, serif, fade, cream, dark edge. A written line.
- `chalkboard` — bottom, serif, still, cream, dark edge. Teaching.
- `dossier` — bottom, serif, fade, bone, dark edge. A file or a record.
- `graffiti` — lower, display, pop, rose, dark edge. Street, one line.
- `hologram` — lower, sans, fade, cyan, dark edge. A screen.
- `inkwater` — bottom, serif, fade, black, light edge. Dark type on a light shot.
- `laser` — lower, mono, still, mint, dark edge. A thin technical line.
- `lastpage` — bottom, serif, still, bone, dark edge. A closing line.
- `neonsign` — lower, display, still, cyan, dark edge. A sign that sits.
- `ordnance` — lower, display, pop, amber, dark edge. One hit, not every word.
- `papercut` — bottom, serif, still, white, dark edge. Flat paper.
- `popup` — lower, display, pop, rose, dark edge. A short sticker line.
- `ransom` — lower, display, still, white, dark edge. Cut-out energy, in front.
- `scoreboard` — top, display, still, amber, dark edge. A number band.
- `spectrum` — lower, sans, fade, violet, dark edge. A color wash.
- `stardust` — lower, serif, fade, violet, dark edge. A quiet poem line.
- `stomp` — middle, display, pop, white, dark edge. A wide shot. A face drops it to lower.
- `terminal` — bottom, mono, typewriter, mint, dark edge. A console.
- `thunder` — lower, display, pop, white, dark edge. One impact.
- `transit` — bottom, sans, still, bone, dark edge. Wayfinding.
- `vhs` — bottom, mono, still, cream, dark edge. Tape.

On the plan: `captions: true` and `caption_preset` set to that id. Do not call `edit_skill` again.
