# Director prompts

Every chat provider (Grok, Claude, Codex, and any added later) loads **all** `*.md` files in this folder, in filename order.

Add a new brief as `02-whatever.md`. Restart `oc-api`. No code change.

`add_motion` is the motion graphic. The director picks a design id. One call renders the clip and places it. `apps/oc-api/motion/render.mjs` draws the picture from that id. No skill file is loaded.

`edit_skill` is one decision page: a spine, a cut, a rhythm, a revision, or color. One call returns that page and changes nothing. Those files live under `skills/edit/`. They are not loaded with the briefs above.

Override the folder with `OPENCUT_PROMPTS_DIR` if the process is not started from the repo root.
