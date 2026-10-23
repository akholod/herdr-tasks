---
name: herdr-tasks
description: >-
  Track work as tasks on the herdr-tasks board shared by the user and agents.
  Use whenever you need to record a task, report progress, change a task's
  status/priority/pin, keep detailed context for later, attach files, list or
  read tasks of the current project, or sync with GitHub issues. Triggers on
  mentions of tasks, the task board, todo/in-progress/testing/done, herdr-tasks,
  t-NNNN ids, or "add this to the board".
---

# herdr-tasks

A cross-project task board for herdr. The user sees it as a panel (dock on the
right, `prefix+t`); you use the `herdr-tasks` CLI. Both read and write the same
files, and the panel shows your changes within a second.

## Rules

- Always go through the CLI. Never edit files under the storage root directly
  (it skips validation, locking and atomic writes).
- Never pass storage paths. The project comes from your working directory:
  its git root, or the directory itself when it is not a git repo. Use
  `--project <PATH>` to act on another project.
- Keep `description` short: it is limited by `description_limit` (default 500
  characters); longer input fails with exit 1 and writes nothing. Put the long
  part in `details.md` instead.
- Use `--json` whenever you parse output.

## Task anatomy

| Part | What goes there | Shown in panel |
|---|---|---|
| title | one line | yes |
| description | short summary, at most `description_limit` chars | yes |
| `details.md` | detailed context for agents: plans, findings, logs, links | hidden (user presses `i`) |
| attachments | copied files (screenshots, logs, videos) | name + size, opens externally |

Statuses: `todo`, `in-progress`, `testing`, `done` (a project may override the
list in its config; the first is the default, the last means done).
Priorities: `P0` (urgent), `P1` (default), `P2`. A pinned task stays on top.

## Commands

| Command | Use |
|---|---|
| `herdr-tasks create TITLE [-d TEXT] [--details TEXT \| --details-file PATH\|-] [--priority P0-2] [--status S] [--tag T]... [--pin]` | new task |
| `herdr-tasks list [--status S] [--priority P] [--tag T] [--all-projects]` | tasks of the project |
| `herdr-tasks show ID [--details]` | one task (`--details` adds details.md) |
| `herdr-tasks update ID [--title T] [-d TEXT] [--details TEXT \| --details-file PATH\|-] [--append-details TEXT] [--priority P] [--status S] [--tag T]... [--untag T]...` | edit |
| `herdr-tasks move ID STATUS` | change status |
| `herdr-tasks pin ID [--off]` | pin / unpin |
| `herdr-tasks attach ID FILE...` | copy files into the task |
| `herdr-tasks sync [--dry-run \| --list-new \| --import N... \| --ignore N... \| --unignore N...]` | GitHub issues sync |
| `herdr-tasks project` | resolved project, storage root, effective config |
| `herdr-tasks skill` | print this file |
| `herdr-tasks tui` | the panel (the user runs it, not you) |

IDs look like `t-0042`; `42` and `t-42` are accepted too.

Examples:

```bash
herdr-tasks create "Fix flaky login test" -d "Times out on CI 1 in 5 runs" --priority P0 --tag ci --json
herdr-tasks update t-0042 --append-details "Root cause: shared fixture port 5432. Fix in tests/login.rs."
printf '%s\n' "$PLAN" | herdr-tasks update t-0042 --details-file -
herdr-tasks move t-0042 in-progress
herdr-tasks attach t-0042 /tmp/ci-log.txt
herdr-tasks move t-0042 testing
herdr-tasks list --status in-progress --json
```

A good workflow: create the task when you start, keep `description` to the
user-facing summary, append findings to `details.md` as you go, move it to
`testing` when it is ready for the user to check, and to `done` only when it is
verified.

## JSON shapes

Task object (`create`, `show`, `update`, `move`, `pin`):

```json
{"id":"t-0042","project":"/home/u/repo","title":"...","status":"todo","status_valid":true,
 "priority":"P1","pinned":false,"tags":["ci"],"created":"...","updated":"...",
 "description":"...","github":{"issue":17,"synced_at":"..."},
 "attachments":[{"name":"log.txt","size":1234}],"path":"...","warnings":[]}
```

`list`: `{"project":{...},"tasks":[Task],"errors":[{"id","path","message"}]}`; a
task with a broken `task.toml` is listed in `errors`, the rest still load.
`attach`: `{"task":"t-0042","attachments":[{"name","size","path"}]}`.

## Exit codes and errors

`0` ok, `1` validation (limit, empty title, unknown status), `2` not found, `3`
config/storage (invalid TOML, broken task, IO), `4` external tool (`gh`, GitHub,
`git`), `64` usage. With `--json`, errors go to stderr as
`{"error":{"code":1,"kind":"validation","message":"..."}}` and stdout is empty.

## GitHub sync

Per project, `github.mode` in the project `config.toml`: `off` (default),
`pull` (issues -> tasks, never writes to GitHub), `push` (title and open/closed
state of linked tasks -> issues), `two-way` (both; when both sides changed, the
remote wins and the task is listed in `conflicts`). Sync runs only on command.
Push never creates issues. `sync --list-new` shows new issues, `--import N`
imports only those, `--ignore N` never offers them again. Access: an
authenticated `gh`, else a token from `GITHUB_TOKEN` / `GH_TOKEN` /
`github.token_file`. On import the full issue body goes to `details.md` once;
later syncs never touch `details.md`.

## Config

Global `<root>/config.toml`, per-project `<root>/projects/<folder>/config.toml`
(project wins). Keys: `description_limit`, `statuses`, `kanban_min_width`,
`open_command`, `[github] mode, repo, import_limit, token_file, api_url`.
`herdr-tasks project --json` prints the paths and the effective config.
