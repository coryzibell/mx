#import "lib.typ": *

#page-header("PR", "Pull request merge with encoded commits.")

== Overview

`mx pr merge` merges a GitHub pull request through the `gh` CLI and encodes
the resulting commit message with #link("base-d.html")[base-d], keeping the
merge commit consistent with the encoding applied by `mx commit`. Without
this command, PR merges would produce plain-text commit messages that break
the encoded history visible through `mx log`.

The command fetches the PR diff and metadata from GitHub, encodes the title
and body, and passes the encoded values to `gh pr merge`. After a successful
merge it performs automatic post-merge cleanup unless told otherwise.

== Basic usage

```bash
mx pr merge 42
```

This squash-merges PR \#42 with an encoded commit message, then switches
your local checkout to the target branch and deletes the local source branch.

== Merge strategies

Three merge strategies are available. They are mutually exclusive -- at most
one flag may be passed.

#command("mx pr merge <number>",
  [Squash merge (default). All commits on the PR are collapsed into a single
  encoded commit on the target branch. This is the most common workflow and
  keeps the target branch history linear.],
  flags: (
    ([`--rebase`], [flag], [Use rebase merge instead of squash. Replays the
    PR's commits onto the target branch individually. The final commit
    message is still encoded.]),
    ([`--merge-commit`], [flag], [Use a standard merge commit instead of
    squash. Preserves the full branch topology in the target branch
    history.]),
  ),
  examples: (
    "mx pr merge 42",
    "mx pr merge 42 --rebase",
    "mx pr merge 42 --merge-commit",
  ),
)

When deciding which strategy to use:

- *Squash* (default) is best for feature branches where individual commits
  are implementation noise. One clean encoded commit on `main`.
- *Rebase* preserves each commit as a separate entry but linearizes the
  history. Useful when each commit is meaningful on its own.
- *Merge commit* preserves full branch topology. Useful for long-lived
  branches where the merge point itself is significant.

== Post-merge cleanup

After a successful merge, `mx pr merge` performs automatic cleanup to keep
your local repository in sync with the remote. This prevents the common
footgun where you are left on a dead branch whose remote ref was deleted by
GitHub, causing the next `git pull --rebase` to fail.

The cleanup sequence:

+ `git fetch origin --prune` -- sync remote state and remove stale tracking
  refs.
+ `git checkout <target-branch>` -- switch to the branch the PR was merged
  into (usually `main`).
+ `git pull --ff-only` -- fast-forward the target branch to include the
  merge commit.
+ `git branch -d <source-branch>` -- delete the local source branch using a
  safe delete. The `-d` flag (not `-D`) refuses to delete the branch if it
  contains commits that have not been merged, preventing accidental data
  loss.

Each cleanup step is best-effort. The merge has already succeeded on GitHub
at this point, so a cleanup failure emits a warning but does not cause the
command to exit non-zero.

=== Safety guards

Cleanup is skipped entirely when any of these conditions are detected:

- *Uncommitted changes.* If your working tree has staged or unstaged
  modifications to tracked files, cleanup is skipped with a warning to stash
  or commit first. Untracked files do not block cleanup.
- *Unpushed commits.* If the local source branch has commits that are not on
  `origin/<source-branch>`, cleanup is skipped to avoid deleting a branch
  with unreplicated work.
- *Missing branch metadata.* If the PR metadata does not include source or
  target branch names, cleanup is skipped because the command cannot
  determine where to switch.
- *Same source and target.* If the source and target branches are the same
  (unusual but possible), cleanup is a no-op.
- *Source branch does not exist locally.* If the source branch has no local
  ref (e.g., you merged someone else's PR), the unpushed-commits check and
  branch deletion are skipped, but fetch, checkout, and pull still run.

== Opting out

```bash
mx pr merge 42 --no-cleanup
```

The `--no-cleanup` flag skips the entire post-merge cleanup sequence. The PR
is merged on GitHub but your local checkout stays on whatever branch you
were on, and no local branches are deleted. Useful when you want to continue
working on the source branch or handle cleanup manually.

== Encoding

The merge commit is encoded the same way as a regular `mx commit`:

+ The *PR diff* (fetched via `gh pr diff`) is hashed with a randomly
  selected dictionary to produce the encoded commit title.
+ The *PR title and body* (fetched via `gh pr view`) are concatenated,
  compressed, and encoded with a second randomly selected dictionary to
  produce the commit body.
+ A *footer* tag in the format `[hash_algo:title_dict|compress_algo:body_dict]`
  is appended so `mx log` can decode the message later.

The encoded title and body are passed to `gh pr merge` via the `--subject`
and `--body` flags, so the merge commit on GitHub contains the full encoded
message. Use `mx log` to read the decoded history.

#note[The encoding uses the same `base-d` pipeline as `mx commit`. See
#link("base-d.html")[base-d] for details on dictionaries, hash algorithms,
and compression codecs.]
