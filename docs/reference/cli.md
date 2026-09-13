[Documentation](../README.md) · [Keyboard](keyboard.md)

# Command-line reference

Generated from the help text in [src/main.rs](../../src/main.rs).
Run `flere --help` for the version you have installed.

```text
Flere — a native terminal workbench

flere [--state DIR]                       Start/attach UI; first-use intro, then a shell
flere [--state DIR] start                 Start supervisor only (never start saved work)
flere [--state DIR] attach                Attach UI to running supervisor
flere [--state DIR] intro                 Preview the animation; creates no state
flere ssh HOST [options]                  Connect through the local SSH companion
flere [--state DIR] add-project --cwd DIR [--name NAME] [--no-shell]
flere [--state DIR] new [--name NAME] [--cwd DIR] [--no-shell]
flere [--state DIR] tab WORKSPACE_ID       Explicitly start another shell in a workspace
flere [--state DIR] open-file WORKSPACE_ID PATH
flere [--state DIR] worktree NAME REPOSITORY BRANCH [BASE] [--no-shell]
flere [--state DIR] agent WORKSPACE_ID codex|claude|copilot [EXACT_UUID]
flere [--state DIR] refresh               Upgrade this supervisor, preserving sessions
flere [--state DIR] refresh-status        Read upgrade outcome
flere --build-info                       JSON identity of this executable build
flere [--state DIR] build-status          Compare this build with the supervisor
flere [--state DIR] coordinate WORKSPACE_ID OPERATION [JSON]
flere [--state DIR] agent-call OP [JSON]  Scoped MCP fallback using this native run
flere [--state DIR] list                  JSON workspace/session identities
flere [--state DIR] capture --session ID --run TOKEN [--lines 80]
flere [--state DIR] send --session ID --run TOKEN --text TEXT
flere [--state DIR] send --session ID --run TOKEN --key Enter|Escape|Ctrl-C|Tab|Backspace
flere [--state DIR] focus WORKSPACE_ID [TAB_ID]
flere [--state DIR] close --session ID --run TOKEN --terminate
flere [--state DIR] stop --terminate       Hang up Flere shells; retain workspaces
flere [--state DIR] serve                  Foreground supervisor for diagnostics

Ctrl+Space: navigation; h/l: panes; j/k: cards/tabs/files; Space: actions.
G: add project; n/N: new worktree; t: shell tab; b/f: history; x: close tab; q: detach.
s: copy the full Flere view as an image to your local clipboard.
Files: Enter opens directories/editor tabs; - or Backspace goes up.
Space actions supports j/k selection, direct shortcuts and Enter. R refreshes.
Terminal: wheel or Shift+Page Up browses history; Shift+Home jumps oldest; Esc/End returns live.
Detaching leaves shells alive. Opening the UI restores saved tabs; supervisor-only startup starts none.
Stopped terminal: S starts an agent, W retries saved tabs, Enter reopens, t opens a shell, Space opens actions.
Socket: STATE/control.sock. Default state: ~/.local/state/flere.
```

Use an explicit `--state` when operating on a separate instance. Read IDs and
run tokens from `list`; do not reuse a token after its process has been replaced.
