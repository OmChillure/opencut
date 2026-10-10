Adapted from `color-grading-finishing` (MIT). Copyright (c) 2026 generative-media-skills contributors.

# Color

Separate the job before you touch a number.

- Correction: the shot is usable. Balance exposure, a cast, contrast, and saturation so the cuts belong together.
- A look: the mood, after the shots already match. Warmth, density, how deep the blacks are, how soft the highlights are.
- Finishing: the master is already handled. Do not grade again at export, and do not tonemap.

Do not call a look a correction. Do not hide a mismatch behind a look. "Cinematic" is not a LUT. Translate it: deeper contrast, softer highlights, warmer shadows, lower saturation, or a green cast. Pick the attributes this footage supports. There is no universal cinematic grade.

On this plan a grade is `exposure`, `contrast`, `saturation`, `temperature`, `lift`, `gamma`, `gain`, and `lut`. 0 is unchanged. `lift` is the shadows, `gamma` the mids, `gain` the highlights, each about -1 to 1. `lut` is a look, not a camera transform: `none`, `film`, `cool`, `warm`, `teal_orange`, or `mono`. Do not stack looks. `mono` only when they asked for black and white.

Leave a slot `grade` empty unless `see` or `watch` showed you that frame and it needs a change. A clear frame stays at zero. A gray picture from a phone HDR file is not a cast. Do not lift exposure to paint it. Leave that grade empty.

Shots in the same light share one correction. Pick the hero shot of the scene, match the others to it, and match brightness before hue. A sunset may be warmer than an office. That is not a mismatch. A jump in exposure or white balance at a cut, inside one scene, is.

Write the smallest change that removes what you saw. If one number fixes the hero and ruins the face or the product, back it off. Protect skin, a product color, a logo, and a screen. OpenCut has no qualifier for a single hue, so keep the grade small instead of painting the whole frame to save one object.

These files are already display-referred pictures. Do not treat `lut` as a log-to-video conversion. `film`, `cool`, `warm`, and `teal_orange` are looks. Use one only when the look is the point, and only on frames you saw.

The piece `grade` covers a shot you could not see. It does not invent a look for a shot you did see. Set the slot, not the piece, when you saw the frame.

Order, when a frame you saw actually needs work:

1. Balance that frame: exposure, temperature, then contrast and saturation.
2. Copy that correction to the other shots in the same light.
3. Add a look only after they match, and only if the piece asks for one.
4. Stop. Do not keep tuning a frame you have not seen.
