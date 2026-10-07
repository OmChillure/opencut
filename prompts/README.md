# Director prompts

Every chat provider (Grok, Claude, Codex, and any added later) loads **all** `*.md` files in this folder, in filename order.

Add a new brief as `02-whatever.md`. Restart `oc-api`. No code change.

`add_motion` is the motion graphic. The director picks a design. One call returns that design page, renders the clip, and places it. The files under `skills/` are those design pages. The director does not run them.

Override the folder with `OPENCUT_PROMPTS_DIR` if the process is not started from the repo root.
