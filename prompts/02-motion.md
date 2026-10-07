# Motion

This brief decides camera moves and `add_motion`. A generic push in another brief yields to this one.

The shot list already has `motion`: `l2r`, `r2l`, `toward`, `away`, or `none`. Read it before you move anything.

## The camera

- Continue the direction. After `l2r`, the next shot is another `l2r` or a `none`. Cut to `r2l` only when the idea is a reversal. The same rule runs the other way.
- `toward` is already a push. `away` is already a pull. `l2r` and `r2l` are already traveling. Leave `end_scale` off those shots. A second move fights the camera that is already there.
- `none` on a hold longer than about two seconds can take one push: `end_scale` about 1.06 to 1.08 and `ease` `in_out`. One hold in a scene, not every hold.
- One treatment on a join. A dissolve, a push, and a speed ramp are three ideas. Pick the one the footage needs.
- `ease` `out` when the move should arrive. `ease` `in` when it should leave. `linear` only when they asked for a constant creep. Omit `ease` and a push eases in and out.

## The graphic

Every graphic is `add_motion`. See the start, middle, and end of the spoken line, then pick one `design`. That call returns the design page, renders the clip, and places it. Do not render a file. A camera move is not a reason to call it.

A punch line or a title: `kinetic-slam`, `kinetic-typewriter`, `kinetic-words`, `kinetic-wave`, `kinetic-bounce`, `kinetic-punch`, `kinetic-blur`, `kinetic-glitch`, `kinetic-burst`, or `kinetic-editorial`.
A spoken number: `stat-count`, `stat-ring`, or `stat-bars`.
A chart, a diagram, or several numbers: `chart-bars`, `chart-line`, `chart-pie`, or `chart-race`. The next chart uses a different one of those four.
A name or a quote: `lower-bar`, `lower-callout`, `lower-quote`, or `lower-split`.
A brand: `logo-draw` or `logo-lockup`.
A place the line is about: `map-highlight` or `map-route`.

`text` is the words or the number, exactly as spoken. The clip already draws them. `prompt` is the series for a chart, the place for a map, or the mark for a logo. `style` is `bold`, `editorial`, `swiss`, `terminal`, `spotlight`, `clean`, or `neon`. `duration` is the length of that line. The host keeps it between 2 and 8 seconds. `layout` is `cutaway`, `beside`, or `behind`. Empty space on one side: `beside`. The person can spare a corner: `behind`. The idea needs the whole frame: `cutaway`. A lower third sits behind unless you name a layout. `kind` is only a shortcut (`kinetic`, `stat`, `chart`, `lower`, `logo`, `map`) when you have not picked a design. Call `add_motion` after the last `revise_edit`.
