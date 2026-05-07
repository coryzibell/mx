#import "lib.typ": *

#page-header("Sync", "GitHub sync for issues and discussions.")

== Overview

`mx sync` provides bidirectional synchronization between GitHub and local YAML
files. Issues and discussions are pulled from GitHub into a local cache as YAML,
edited locally, and pushed back.

The sync subsystem uses two API layers internally: the GitHub REST API for
issues and the GitHub GraphQL API for discussions. Authentication
is handled automatically through a token stored in `~/.claude.json`.

All YAML files live in a sync cache directory at
`$MX_HOME/cache/sync/<owner>-<repo>/` by default. Each file represents a single
issue or discussion.

== Subcommands

`mx sync` has three subcommands:

- `pull` -- download issues and discussions from GitHub to local YAML
- `push` -- upload local YAML changes back to GitHub
- `issues` -- run a full bidirectional sync (pull then push)

Every subcommand accepts a `--dry-run` flag that previews what would happen
without making any changes.

== Pull

#command("mx sync pull <repo>",
  [Download open issues and discussions from a GitHub repository into local
  YAML files. Issues are fetched via the REST API; discussions via GraphQL.
  Each item becomes a separate YAML file in the output directory.],
  flags: (
    ([`repo`], [positional], [Repository in `owner/repo` format.]),
    ([`-o`, `--output`], [path], [Output directory. Defaults to
    `$MX_HOME/cache/sync/<owner>-<repo>/`.]),
    ([`--dry-run`], [flag], [Preview what would be pulled without writing
    files.]),
  ),
  examples: (
    "mx sync pull coryzibell/mx",
    "mx sync pull coryzibell/mx --output ./local-issues",
    "mx sync pull coryzibell/mx --dry-run",
  ),
)

=== What pull does

+ Fetches all open issues via the REST API, including comments.
+ Fetches all discussions via the GraphQL API, including comments.
+ For each item, checks whether a local YAML file already exists (matched by
  issue number or discussion ID).
+ *New items* get a fresh YAML file. The filename is derived from the number
  and a slugified title: `42-fix-crash-on-empty-input.yaml` for issues,
  `d7-feature-request-dark-mode.yaml` for discussions.
+ *Existing items* are updated with the latest remote state -- but only if the
  local copy has not been modified since the last sync. If local changes are
  detected, the item is skipped to avoid overwriting your edits.

=== Local change detection

Pull uses a `last_synced` snapshot stored in each YAML file's metadata to
detect local modifications. When a file is synced, the snapshot records the
title, body, labels, and timestamp at that moment. On the next pull, the
current local values are compared against the snapshot:

- If they match, the file is safe to overwrite with the remote state.
- If they differ, pull skips the file and prints a message indicating local
  changes were preserved.

This is a safety mechanism. If you have edited a YAML file locally and want
to pull the remote state anyway, you must push your changes first (or
discard them by deleting the file and re-pulling).

=== Pull output

Pull prints a summary showing counts of created, updated, and unchanged
items for both issues and discussions:

```
Issues: 3 created, 5 updated, 2 unchanged
Discussions: 1 created, 0 updated, 4 unchanged
```

== Push

#command("mx sync push <repo>",
  [Upload local YAML changes to GitHub. Creates new issues or discussions
  for items without a GitHub ID, and updates existing ones using three-way
  merge to handle concurrent remote edits.],
  flags: (
    ([`repo`], [positional], [Repository in `owner/repo` format.]),
    ([`-i`, `--input`], [path], [Input directory containing YAML files.
    Defaults to `$MX_HOME/cache/sync/<owner>-<repo>/`.]),
    ([`--dry-run`], [flag], [Preview what would be pushed without modifying
    GitHub.]),
  ),
  examples: (
    "mx sync push coryzibell/mx",
    "mx sync push coryzibell/mx --input ./local-issues",
    "mx sync push coryzibell/mx --dry-run",
  ),
)

=== Item types

Push routes items based on their `type` field:

- *issue* (default) -- created and updated via the REST API. Supports title,
  body, labels, and assignees.
- *idea* or *discussion* -- created and updated via the GraphQL API. Supports
  title, body, labels, and discussion category.

=== Creating new items

A YAML file without a `github_issue_number` or `github_discussion_id` is
treated as a new item. Push creates it on GitHub and then updates the local
file with the assigned number/ID, timestamp, and a `last_synced` snapshot.
The file is also renamed to include the newly assigned number.

For new discussions, push looks up the repository's discussion categories by
slug. If the category specified in the YAML does not exist on the repository,
the item is skipped.

=== Updating existing items

For items that already have a GitHub ID, push uses a three-way merge to
reconcile local edits, remote edits, and the `last_synced` base:

+ The local state is read from the YAML file.
+ The current remote state is fetched from GitHub.
+ The `last_synced` snapshot provides the common base.
+ Each field (title, body, labels, assignees) is compared across all three
  states to determine what changed and where.

The merge engine handles five cases per field:

- *Unchanged* -- all three match. Nothing to do.
- *Local only* -- local differs from base, remote matches base. Local wins.
- *Remote only* -- remote differs from base, local matches base. Remote wins.
- *Both same* -- both changed to the same value. Either wins (they agree).
- *Conflict* -- both changed to different values. Resolved automatically by
  preferring the local value.

=== Label merge semantics

Labels use union merge rather than the field-level conflict model. The formula
is:

```
merged = base + local_additions + remote_additions - local_deletions - remote_deletions
```

This means labels added on either side are preserved, and labels deleted on
either side are removed. There are no label conflicts -- both sides'
intentions are honored. Assignees follow the same union merge logic.

=== Push output

Push prints a summary matching the pull format:

```
Issues: 1 created, 3 updated, 8 unchanged
Discussions: 0 created, 1 updated, 2 unchanged
```

== Issues

#command("mx sync issues <repo>",
  [Run a full bidirectional sync: pull from GitHub, then push local changes
  back. This is a convenience wrapper that calls `pull` followed by `push`
  with default directories.],
  flags: (
    ([`repo`], [positional], [Repository in `owner/repo` format.]),
    ([`--dry-run`], [flag], [Preview both pull and push without making any
    changes.]),
  ),
  examples: (
    "mx sync issues coryzibell/mx",
    "mx sync issues coryzibell/mx --dry-run",
  ),
)

The output separates the two phases with headers:

```
=== Bidirectional Issue Sync ===

--- Pull (GitHub -> Local) ---
...pull output...

--- Push (Local -> GitHub) ---
...push output...

=== Sync Complete ===
```

== YAML file format

Each synced item is stored as a YAML file with this structure:

```yaml
metadata:
  title: "Issue title"
  type: issue        # or "idea" for discussions
  labels:
    - bug
    - enhancement
  assignees:
    - username
  state: open
  category: ideas    # discussions only
  github_issue_number: 42
  github_updated_at: "2025-01-15T10:30:00Z"
  last_synced:
    title: "Issue title"
    body: "Body at last sync"
    labels:
      - bug
      - enhancement
    updated_at: "2025-01-15T10:30:00Z"
    assignees:
      - username
body_markdown: |
  The full issue body in markdown.
comments:
  - id: "123456"
    author: username
    created_at: "2025-01-15T10:30:00Z"
    body: "Comment text"
```

Fields can also be placed at the root level (`title`, `body`, `type`,
`labels`, `assignees`, `category`) for convenience when authoring new items
by hand. Root-level fields take precedence over their `metadata.*`
counterparts during push.

#tip[To create a new issue from scratch, write a minimal YAML file with just
`title`, `body`, and optionally `labels`, then run `mx sync push`. The file
will be updated with the GitHub issue number and renamed automatically.]

== Authentication

Sync reads the GitHub token from `~/.claude.json`, looking for
`projects.<project>.mcpServers.github.env.GITHUB_PERSONAL_ACCESS_TOKEN`
across all configured projects.

#note[The token needs `repo` scope for issues, and `read:discussion`
plus `write:discussion` for discussions.]

== Related commands

- `mx convert md2yaml` -- convert markdown files to the YAML format used by
  sync. Useful for bulk-importing issues from markdown notes.
