# Git snapshot retention

New Git comparison and patch documents live in the private
`STATE/git-diffs-v1` directory. Comparison sides remain read-only and capped at
512 KiB each; whole-commit patches are capped at 1 MiB. Identical documents in
the same state reuse their paths and existing editor tabs.

Flere automatically removes a managed entry after **30 days without a
reference**. The timer starts when cleanup first confirms that the entry has
lost its last reference, rather than at creation. Reopening resets that timer.
An entire comparison pair is retained whenever either file is referenced by a
saved editor tab. This includes detached UIs, stopped workspaces, pending
restoration and failed restoration. Same-PID refresh preserves the running
comparison; cold startup still reopens the saved editor file, rather than
reconstructing the original two-sided editor invocation.

Each UI worker reserves its generated entry through result handling. Before an
editor opens, the supervisor creates an independent durable reservation and
releases it only after the tab layout is successfully checkpointed. A save
failure can leave the editor running, so an error reply does not release that
supervisor reservation. Entry locks order these reservations against cleanup;
cleanup reads the current saved layout while holding the entry lock. It does
not use an earlier snapshot of references to authorize deletion.
An exited or closing editor can remain visible while a descendant keeps its
PTY open. Before removing such an editor from the saved layout, Flere pins
the retained session independently until it is removed after EOF.

One background worker visits at most 16 directory entries per batch, requested
at most once a minute, and continues across batches. The terminal event loop does no cache scanning or
hashing. Cleanup verifies private ownership, state path/device/inode identity,
the entry manifest and complete file set, and each file's size, inode and
SHA-256 before unlinking generated files. It never recursively deletes a
directory. A backward clock adjustment starts a fresh grace period.

These cases deliberately remain retained:

- Legacy shared documents in `$XDG_CACHE_HOME/flere/git-diffs` or
  `~/.cache/flere/git-diffs`; their complete ownership is unknown.
- An entry opened from a different state, which receives a conservative
  permanent foreign-reference pin without scanning that other state's files.
- Reservations surviving a crash, interrupted refresh or uncertain result.
  A missing or changed process identity does not prove that a file is unused.
- Copied/moved state ownership, changed content, symlinks, hard links, extra or
  missing files, unsupported manifests, and unreadable, incomplete or older
  saved layouts. Cleanup requires the current version 7 tab checkpoint.

Consequently this is an expiration policy, not an absolute disk quota. Protected
or uncertain data can exceed any normal working-set size.

The private `STATE/git-diffs-v1/status.json` records the last batch's reclaimed
entries and bytes, referenced entries, outstanding reservations, confirmed-live
versus unconfirmed reservation owners, foreign pins, and retained errors. Counts
cover that batch, not an inventory of the whole cache. Ownership failures that
prevent a safe status write are reported in the `git-cache-retention` diagnostic
event. No legacy files are silently included in reclaimed totals.

Retention tests use disposable private home-cache directories. They cover
reference release timing, complete pairs, concurrent open locks, failed saves,
independent states, ownership/content changes, refresh and cold editor restore.
