Adapted from `editing-montage` (MIT). Copyright (c) 2026 generative-media-skills contributors.

# Documentary

The viewer should follow a claim or a question through people and evidence. Let a sentence finish. Do not punch every hold.

Shape:

- The claim or the question.
- A person, or the thing you actually observed.
- Evidence beats. Each one adds a fact the previous shot did not have.
- A complication or a counterpoint, when the footage has one. Do not invent it.
- A close that synthesizes. Do not explain the picture a second time in a caption.

Cutaways relate to the sentence being said, or they show a true context. A cutaway that only hides a jump, and adds nothing, is filler. Silence stays when it carries a thought.

Hold a face until it can be read. Cut on a change of thought or a breath, not on every sentence end. For action, cut just before or during the movement so the next angle completes it.

On the plan: keep the order of the argument. One file with a keeper and a throwaway is two slots of that `media_id`. `source_in` is the keeper. `duration` includes the last word and a short tail, and does not cover the gap, the sentence you are dropping, or dead air. `fade_in` on the first slot and `fade_out` on the last. `dissolve` when the beat, the time, or the place changes. The next sentence of the same thought stays a cut. `end_scale` above 1 and `ease` `out` on one still hold that opens or reveals. After the last revision, `add_motion` once when a line names a person, a number, a place, or a punch line. No speed ramp unless they asked. Leave `caption_mood` empty. One `caption_preset` comes from the captions page. Grade stays empty on a clear frame.
