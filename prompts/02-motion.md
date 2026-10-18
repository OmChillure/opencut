# Motion

The shot list already has `motion`: `l2r`, `r2l`, `toward`, `away`, or `none`. Read it before you move anything. `toward` and `away` are already a push and a pull. `l2r` and `r2l` are already traveling. Leave `end_scale` off those. Any other push is the `rhythm` or `shot` page.

## The graphic

Every graphic is `add_motion`. One `watch` of that spoken line replaces three `see` calls. Then pick one `design`. That call renders the clip and places it. Do not write HTML. Do not render a file. A piece places one graphic when a spoken line is a name, a number, a place, a quote, or a punch line. Call `add_motion` after the last `revise_edit`. Skipping that line leaves the piece unfinished. A camera move is `end_scale` on the slot, not this call.

A punch line or a title: `kinetic-slam`, `kinetic-typewriter`, `kinetic-words`, `kinetic-wave`, `kinetic-bounce`, `kinetic-punch`, `kinetic-blur`, `kinetic-glitch`, `kinetic-burst`, or `kinetic-editorial`.
A spoken number: `stat-count`, `stat-ring`, or `stat-bars`.
A chart, a diagram, or several numbers: `chart-bars`, `chart-line`, `chart-pie`, or `chart-race`. The next chart uses a different one of those four.
A name or a quote: `lower-bar`, `lower-callout`, `lower-quote`, or `lower-split`.
A brand: `logo-draw` or `logo-lockup`.
A place the line is about: `map-highlight` or `map-route`.

`text` is the number and the words around it, or the name, or the quote, exactly as spoken. Write `10,000 prize pool`, not a bare number, when the line names what the number is. `prompt` is the series for a chart, the place for a map, or the mark for a logo. `style` is `bold`, `editorial`, `swiss`, `terminal`, `spotlight`, `clean`, or `neon`. `duration` is how long that line is spoken. The host keeps the graphic on screen only while that shot is up, between 1.2 and 8 seconds. Leave `layout` empty. On a talking shot the person stays full frame and the type sits in the lower band: one sans, the number with its label, no plate over the face. A full-frame card is only a title with nobody in the shot. Pass `cutaway` only for that title. `kind` is only a shortcut (`kinetic`, `stat`, `chart`, `lower`, `logo`, `map`) when you have not picked a design. Call `add_motion` after the last `revise_edit`. Do not write HTML. Do not render a file.
