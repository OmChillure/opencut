# Motion

The shot list already has `motion`: `l2r`, `r2l`, `toward`, `away`, or `none`. Read it before you move anything. `toward` and `away` are already a push and a pull. `l2r` and `r2l` are already traveling. Leave `end_scale` off those. Any other push is the `rhythm` or `shot` page.

## The graphic

Every graphic is `add_motion`. One `watch` of that spoken line replaces three `see` calls. Then pick one `design`. That call renders the clip and places it. Do not write HTML. Do not render a file. A camera move is not a reason to call it.

A punch line or a title: `kinetic-slam`, `kinetic-typewriter`, `kinetic-words`, `kinetic-wave`, `kinetic-bounce`, `kinetic-punch`, `kinetic-blur`, `kinetic-glitch`, `kinetic-burst`, or `kinetic-editorial`.
A spoken number: `stat-count`, `stat-ring`, or `stat-bars`.
A chart, a diagram, or several numbers: `chart-bars`, `chart-line`, `chart-pie`, or `chart-race`. The next chart uses a different one of those four.
A name or a quote: `lower-bar`, `lower-callout`, `lower-quote`, or `lower-split`.
A brand: `logo-draw` or `logo-lockup`.
A place the line is about: `map-highlight` or `map-route`.

`text` is the words or the number, exactly as spoken. The clip already shows those words. `prompt` is the series for a chart, the place for a map, or the mark for a logo. `style` is `bold`, `editorial`, `swiss`, `terminal`, `spotlight`, `clean`, or `neon`. `duration` is the length of that line. The host keeps it between 2 and 8 seconds. `layout` is `cutaway`, `beside`, or `behind`. Empty space on one side: `beside`. The person can spare a corner: `behind`. The idea needs the whole frame: `cutaway`. A lower third sits behind unless you name a layout. `kind` is only a shortcut (`kinetic`, `stat`, `chart`, `lower`, `logo`, `map`) when you have not picked a design. Call `add_motion` after the last `revise_edit`.
