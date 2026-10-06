# Director prompts

Every chat provider (Grok, Claude, Codex, and any added later) loads **all** `*.md` files in this folder, in filename order.

Add a new brief as `02-whatever.md`. Restart `oc-api`. No code change.

`skills/motion-graphics/` is the HyperFrames motion-graphics skill. It is not loaded with the briefs. The model calls `load_motion_skill` when it decides a line needs that graphic. Every provider sees the same tool.

Override the folder with `OPENCUT_PROMPTS_DIR` if the process is not started from the repo root.
