# Motion

This brief decides camera moves and `add_design`. A generic push in another brief yields to this one.

The shot list already has `motion`: `l2r`, `r2l`, `toward`, `away`, or `none`. Read it before you move anything.

## The camera

- Continue the direction. After `l2r`, the next shot is another `l2r` or a `none`. Cut to `r2l` only when the idea is a reversal. The same rule runs the other way.
- `toward` is already a push. `away` is already a pull. `l2r` and `r2l` are already traveling. Leave `end_scale` off those shots. A second move fights the camera that is already there.
- `none` on a hold longer than about two seconds can take one push: `end_scale` about 1.06 to 1.08 and `ease` `in_out`. One hold in a scene, not every hold.
- One treatment on a join. A dissolve, a push, and a speed ramp are three ideas. Pick the one the footage needs.
- `ease` `out` when the move should arrive. `ease` `in` when it should leave. `linear` only when they asked for a constant creep. Omit `ease` and a push eases in and out.

## The graphic

`add_design` is one idea crossing the frame, for the length of that spoken line.

- `prompt` names one subject and one path: where the form sits in the first moment, and where it sits at the end. The change of place has to be obvious. A form that pulses in the center is a failed graphic.
- A chart pattern is that diagram drawing itself in one continuous stroke. A house rises until it fills the frame. Those are one action, not a sequence of scenes, and not a real object that happens to share the name.
- No letters and no numbers in `prompt`. The words go in `text`, and the editor draws that label.
- A short line gets one stroke. Do not ask a two-second clip to tell a three-part story.
- Pick `layout` from the three frames you saw. Empty space on one side: `beside`. The person can spare a corner and the idea is the picture: `behind`. The idea needs the whole frame: `cutaway`.
