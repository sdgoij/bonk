# ADR-020: Watch drop folders by polling, not by change notifications

**Status:** Accepted, 2026-10-28

## Context

The PRD asks for local file watchers so an on-premise ERP can push incoming
invoices into the archive without a person in the loop. M3.6 in the plan named
`notify` with a debounce.

Two facts about the environment decide this:

1. Drop folders are often network shares (SMB, NFS, a synced folder). `inotify`
   does not report changes on those mounts at all, `FSEvents` is lossy, and
   `ReadDirectoryChangesW` behaves differently per filesystem. A watcher built on
   notifications silently does nothing on the very setup the feature exists for.
2. The interesting question is not "did something happen" but "has the writer
   finished". ERP exporters write in several steps, and a notification fires on
   the first one.

## Decision

`import::watch` polls the directory and treats a file as readable once it has been
untouched for a quiet period (`--quiet-for`, 5 seconds by default):

- A file is offered when it is at least the quiet period old **and** is new or has
  changed since it was last offered.
- What has been offered is remembered (size and modification time), so a file that
  will never parse is not retried on every poll; editing it offers it again.
- Files already in the folder when the watcher starts are left alone. A drop
  folder can hold years of invoices, and starting a watcher must not import them.
- `--once` reads the folder as it stands, which is the deliberate batch case (cron,
  a migration, a test).
- Subdirectories and editor leftovers (`.tmp`, `.part`, `.crdownload`, `.swp`,
  `~$…`, dotfiles) are ignored.

`bonk watch <dir>` runs the loop; `--every` sets the poll interval. Idempotence is
not the watcher's job: the pipeline already refuses the same bytes and the same
supplier number twice.

## Consequences

- The feature works on the network shares it is aimed at, and no new dependency
  enters the build.
- Latency is the poll interval plus the quiet period — seconds, not milliseconds.
  For an ERP push that is invisible; for an interactive drag-and-drop it is not,
  and `bonk import` remains the immediate path.
- A file whose modification time is in the future (clock skew on the sharing host)
  is not read until the clock catches up. That is reported by absence, not by an
  error.
- The watcher's behaviour is a pure function of the directory listing and a clock
  passed in, so it is tested without sleeping or touching mtimes.

## Alternatives

- **`notify` with a debounce**: the plan's original choice. Rejected because
  notifications do not arrive on network shares, which is where ERP exports
  usually land.
- **`notify` plus polling as a fallback**: two code paths with different failure
  modes for a feature whose latency budget is seconds. The fallback would be the
  one that always runs.
- **Watching the filesystem and trusting the first event**: imports half-written
  files; the debounce is the part that matters, and it needs no notifications.
