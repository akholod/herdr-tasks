# herdr-tasks

A cross-project task board for [herdr](https://herdr.dev). One Rust binary:
`herdr-tasks tui` is the panel (a dock on the right, about a third of the tab),
every other subcommand is the CLI that agents use. Tasks are plain files, so the
panel, the CLI and GitHub sync all see the same data; the panel picks up changes
within a second.

- Project = any herdr space. Every open space appears in the Project grouping,
  even before it has tasks.
- One list grouped by priority, status or project; collapsible groups; a pinned
  group on top; kanban columns for the status grouping when the pane is wide.
- Task = title + short description (hard limit) + hidden `details.md` for agent
  context + attachments.
- Optional GitHub issues sync per project (`off`, `pull`, `push`, `two-way`),
  only on command.

## Install

Requires a Rust toolchain (1.89+) and herdr 0.9+.

```bash
bash scripts/install.sh          # dry run: prints what it would do
bash scripts/install.sh --yes    # build, link, skill, ~/.local/bin symlink, prefix+t
```

`install.sh` runs `cargo build --release`, `herdr plugin link .`, copies
`skill/SKILL.md` to `~/.claude/skills/herdr-tasks/` (and `~/.agents/skills/`
when that exists), links the binary into `~/.local/bin`, appends the keybinding
below to `~/.config/herdr/config.toml` (after a `.bak.<timestamp>` backup, once)
and reloads herdr. Options: `--key KEY`, `--no-build`, `--no-link`.

```toml
[[keys.command]]
key = "prefix+t"
type = "plugin_action"
command = "herdr-tasks.open-dock"
```

The `open-board` action opens the wide board as an overlay:
`herdr plugin pane open --plugin herdr-tasks --entrypoint board --placement overlay`
(or `--placement tab`).

## Panel keys

| Key | Action |
|---|---|
| `j`/`k`, `gg`/`G`, `Ctrl+d`/`Ctrl+u` | move |
| `Tab` / `Shift+Tab` | grouping: priority → status → project |
| `h`/`l`, `Space` on a header | collapse / expand group |
| `C` | collapse all / expand all |
| `Enter` | open detail; in the detail, open the attachment |
| `Esc` | close detail (never quits) |
| `s` / `S` | next / previous status |
| `p` | cycle priority P0 → P1 → P2 |
| `t` | pin / unpin |
| `i` | show / hide `details.md` |
| `J`/`K`, `o` | attachment cursor, open attachment |
| `n` | new task (title prompt) in the project under the cursor |
| `R` | GitHub sync of the current project (opens the new-issue review) |
| `r` | reload |
| `y` | copy the task id (OSC 52) |
| `?` | help (pane size, storage root) |
| `q`, `Ctrl+c` | quit |

Mouse: click grouping tabs, group headers, kanban column headers, task rows,
the `[status] [P1] [pin] [details]` chips and attachment rows; the wheel
scrolls the list or the detail under the cursor. Mouse capture blocks native
text selection: hold Shift while dragging to select.

## CLI

```bash
herdr-tasks create "Fix login" -d "Times out on CI" --priority P0 --tag ci --json
herdr-tasks update t-0001 --append-details "root cause: shared port"
herdr-tasks move t-0001 testing
herdr-tasks attach t-0001 shot.png
herdr-tasks list --json
herdr-tasks sync --dry-run
```

The project is the git root of the current directory (or the directory itself
outside git); `--project PATH` overrides. Exit codes: `0` ok, `1` validation,
`2` not found, `3` config/storage, `4` external tool, `64` usage. With
`--json`, errors are `{"error":{"code","kind","message"}}` on stderr. See
`skill/SKILL.md` (`herdr-tasks skill`) for the full reference.

## Storage

Root: `$HERDR_TASKS_HOME`, else `$HERDR_PLUGIN_CONFIG_DIR`, else
`herdr plugin config-dir herdr-tasks`, else
`~/.config/herdr/plugins/config/herdr-tasks`. `herdr-tasks project --json`
prints the resolved `root` and `root_source`.

```
<root>/
  config.toml                    # global config (optional)
  projects.toml                  # "/path/to/project" = "<slug>-<hash8>"
  projects/<slug>-<hash8>/
    config.toml                  # project config (optional, wins over global)
    sync-state.toml              # ignored_issues = [12, 15]
    .lock                        # advisory write lock
    tasks/t-0001/
      task.toml  description.md  details.md  attachments/
```

Writes are atomic (temp file + rename) under a per-project lock.

## Config

```toml
description_limit = 500
statuses = ["todo", "in-progress", "testing", "done"]  # first = default, last = done
kanban_min_width = 100
open_command = []            # empty = xdg-open (Linux) / open (macOS)

[github]
mode = "off"                 # off | pull | push | two-way
repo = ""                    # empty = from `git remote get-url origin`
import_limit = 200
token_file = ""              # REST fallback token file (chmod 600)
api_url = "https://api.github.com"
```

## GitHub sync

`herdr-tasks sync` (or `R` in the panel) with `github.mode`:

- `pull`: open issues become tasks (body → description, truncated to the limit;
  full body → `details.md` on first import only); title, labels and open/closed
  state of linked tasks follow GitHub. Never writes to GitHub.
- `push`: title and open/closed state of linked tasks go to GitHub. Issues are
  never created.
- `two-way`: both. When both sides changed since the last sync the remote wins
  and the task is listed in `conflicts`.

After a fetch the panel lists new issues for review; unchecked ones are
remembered in `sync-state.toml` and not offered again. The CLI equivalents are
`sync --list-new`, `--import N...`, `--ignore N...`, `--unignore N...`.
Access uses `gh` when it is installed and authenticated, else the REST API with
`GITHUB_TOKEN`, `GH_TOKEN` or `github.token_file`.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test
bash scripts/check-dock.sh       # inside herdr, with the dock open
```

TUI snapshots live in `tests/snapshots/` (`cargo insta review`, or
`INSTA_UPDATE=always cargo test` and review the diff).
