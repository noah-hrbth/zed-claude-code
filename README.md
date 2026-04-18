# zcc — Zed ↔ Claude Code

Send editor selections from [Zed](https://zed.dev) to [Claude Code](https://claude.com/claude-code) with one keystroke.

When Claude Code runs in a Zed integrated terminal and is attached via `/ide`, press **Cmd+L** in any editor buffer. The selected lines appear in Claude Code as an `@`-mention, and the terminal auto-focuses.

Works for every project and every Zed window after a one-time install.

## Install

```sh
brew install noah-hrbth/zcc/zcc
zcc install
```

`zcc install` merges two entries into your global Zed config:

- `~/.config/zed/tasks.json` — a silent task that forwards the clipboard to a per-worktree daemon.
- `~/.config/zed/keymap.json` — binds `Cmd+L` (Editor) to copy the current selection and spawn the task.

Originals are backed up as `*.bak` next to each file.

## Use

In any project:

```text
# inside a Zed integrated terminal
claude
/ide
```

Zed is offered — accept it. Then select code in any editor buffer and press **Cmd+L**. That's all.

## Uninstall

```sh
zcc uninstall
brew uninstall zcc
```

`zcc uninstall` removes the entries it added; any other Zed config you have is untouched.

## Diagnose

```sh
zcc doctor
```

Reports whether the binary is on PATH, whether the Zed config has our entries, and tails the log.

Logs: `~/Library/Logs/zcc/zcc.log` (JSON lines, rotates at 5 MB).

## Architecture

```
Zed editor — Cmd+L
    └─► workspace::SendKeystrokes "cmd-f18 cmd-f19 cmd-f20"
            ├─► cmd-f18 ─► editor::Copy                    (selection → clipboard)
            ├─► cmd-f19 ─► task::Spawn "zcc-send"
            │                 ▼
            │             /bin/sh -c 'pbpaste | zcc send ...'
            │                 ▼
            │             unix socket $TMPDIR/zcc-<hash>.sock
            │                 ▼
            │             zcc daemon (per worktree, double-forked on demand)
            │                 ▼
            │             ws://127.0.0.1:<port>/   ◄── Claude Code CLI (/ide)
            │             ~/.claude/ide/<port>.lock   ──► at_mentioned JSON-RPC
            │
            └─► cmd-f20 ─► terminal_panel::ToggleFocus    (focus Claude terminal)
```

F18/F19/F20 are unused private-use keys, used as a rendezvous so a single Cmd+L can fan out to three actions (Zed's keymap has no multi-action array syntax).

**Why the clipboard?** `task::Spawn` from a Zed keybinding does not re-evaluate `$ZED_SELECTED_TEXT` per invocation ([zed-industries/zed#40118](https://github.com/zed-industries/zed/issues/40118)). The only reliable way to get the live selection is to force a copy first. The trade-off is that Cmd+L overwrites your system clipboard.

## Known limitations (v1)

- **macOS only.** Linux / Windows later.
- **Clipboard is overwritten by Cmd+L.** The keybinding synthesizes a Cmd+C to capture the selection; your previous clipboard contents are replaced. Required workaround for Zed issue #40118.
- **Vim visual-mode selections are not captured.** Zed's native clipboard copy only fires for native selections — use `Shift+Arrow` or mouse drag to select, not vim `v`/`V`. Tracked for v2.
- **Brief task-tab flash.** A terminal tab flashes next to the Claude tab on every Cmd+L. Cannot be eliminated — Zed's `task::Spawn` unconditionally creates a terminal pane, with no "silent exec" action in the codebase. Keeping the terminal panel open minimises the visible flash.
- **Run `/ide` once per Claude session.** The WebSocket port is per-worktree and not known when Zed starts, so we don't set `CLAUDE_CODE_SSE_PORT` — Claude discovers the lockfile on `/ide`.
- **Config comments not preserved.** `zcc install` re-emits `tasks.json` / `keymap.json` without your comments. Your original is kept at `*.bak`.

## Development

```sh
cargo build --release
cargo test
```

Smoke-test install against a fake home:

```sh
HOME=/tmp/fake-home ./target/release/zcc install
HOME=/tmp/fake-home ./target/release/zcc doctor
HOME=/tmp/fake-home ./target/release/zcc uninstall
```

## License

MIT
