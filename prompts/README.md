# Director prompts

Every chat provider (Grok, Claude, Codex, and any added later) loads **all** `*.md` files in this folder, in filename order.

Add a new brief as `02-whatever.md`. Restart `oc-api`. No code change.

Override the folder with `OPENCUT_PROMPTS_DIR` if the process is not started from the repo root.
