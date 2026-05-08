#import "lib.typ": *

#page-header("show", "Decoded git show for encoded commits.")

== Overview

`mx show` decodes the output of `git show` the same way
#link("log.html")[`mx log`] decodes `git log`. Because
#link("commit.html")[`mx commit`] encodes every commit message through a
randomly selected #link("base-d.html")[base-d] dictionary, raw `git show`
displays unreadable glyphs where the commit message should be. `mx show`
reverses the encoding and displays your original message while passing
everything else -- diffs, stats, file content -- through unchanged.

It is a drop-in replacement for `git show`. Every flag that `git show`
accepts works with `mx show`.

#note[Always use `mx show` to inspect commits. Raw `git show` will show
encoded noise for any commit made with `mx commit`.]

== Basic usage

Show the most recent commit with its diff:

```bash
mx show
```

Show a specific commit:

```bash
mx show abc1234
```

Show a commit with diffstat instead of the full diff:

```bash
mx show --stat
```

Show only the commit message (no diff):

```bash
mx show --no-patch
```

Show only filenames changed:

```bash
mx show --name-only
```

== How it works

`mx show` uses a two-pass approach:

+ *Pass 1* runs `git show` with `--no-patch` and a structured format to
  retrieve commit metadata (hash, author, date, parent hashes) and the
  encoded message body. The body is decoded using the same pipeline as
  `mx log` -- the footer tag identifies the dictionary and compression
  algorithm, and the body is decompressed back to your original message.

+ *Pass 2* runs `git show` with an empty format string to retrieve just
  the diff output. This is streamed to your terminal as-is, identical to
  what `git show` would produce.

The result looks exactly like `git show` output, except the commit message
is readable.

=== Passthrough modes

In certain cases, `mx show` skips decoding entirely and runs raw
`git show`:

- *File content* (`ref:path` syntax) -- when you use `mx show HEAD:src/main.rs`
  to view a file at a specific revision, there is no commit message to
  decode. The command passes through to `git show` directly.

- *Custom format* (`--format` or `--pretty`) -- when you control the output
  format yourself, decoding would interfere. The command passes through
  unchanged.

=== Fallback behavior

If decoding fails for any reason -- the commit was not made with
`mx commit`, the footer is missing, or the dictionary lookup fails --
`mx show` falls back to displaying the raw message exactly as `git show`
would. It is always safe to use `mx show` in place of `git show`, even in
repositories with a mix of encoded and non-encoded commits.

== Merge commits

Merge commits display a `Merge:` line showing the parent hashes, matching
the default `git show` format:

```
commit abc1234def5678...
Merge:  aaa1111 bbb2222
Author: Charlie <charlie@example.com>
Date:   Wed May 7 2026

    the decoded merge commit message
```

== Multiple refs

You can pass multiple refs and `mx show` will decode each one:

```bash
mx show HEAD HEAD~1 HEAD~2
```

== Tags

When showing a tag, `mx show` displays the tag metadata followed by the
decoded commit it points to. If the tag object itself is not a commit (e.g.
an annotated tag preamble), its content is printed as-is.

== Flags reference

`mx show` accepts all flags that `git show` accepts. There are no
mx-specific flags -- the command is designed to be a transparent wrapper.

Common flags:

#table(
  columns: (auto, 1fr),
  table.header([*Flag*], [*Description*]),
  [`--stat`], [Show a diffstat summary instead of the full diff.],
  [`--no-patch`], [Show only the commit header and message, no diff.],
  [`-s`], [Shorthand for `--no-patch`.],
  [`--name-only`], [Show only the names of changed files.],
  [`--name-status`], [Show names and status (added, modified, deleted) of changed files.],
  [`--raw`], [Show the diff in raw format.],
  [`--format=<fmt>`], [Custom format string (passthrough -- skips decoding).],
  [`--pretty=<fmt>`], [Alias for `--format` (passthrough -- skips decoding).],
)

Any arguments not listed here are passed directly to `git show`.

== Relationship to mx log and mx commit

`mx commit`, `mx log`, and `mx show` form a complete encoding round-trip:

+ `mx commit` encodes your message and writes it as an encoded git commit.
+ `mx log` decodes the commit history (replaces `git log`).
+ `mx show` decodes individual commit details (replaces `git show`).

Both `mx log` and `mx show` have full parity with their git counterparts.
Every flag that `git log` or `git show` accepts works with the mx versions,
with transparent decoding applied to encoded messages. `mx log` supports
`--oneline`, `--stat`, `-p`, format presets (`short`, `medium`, `full`,
`fuller`), and all git log filter flags (`--author`, `--since`, `-- <path>`,
etc.). Non-encoded commits pass through unchanged in all three commands.

For the full encoding specification, see #link("commit.html")[commit].
For the full flag reference and architecture details, see #link("log.html")[log].
