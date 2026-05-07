#import "lib.typ": *

#page-header("commit", "Encoded git commits with base-d compression.")

== Overview

`mx commit` wraps `git commit` with automatic encoding. Your human-readable
commit message is compressed and encoded through a randomly selected
#link("base-d.html")[base-d] dictionary, and the diff is hashed through a
second (also random) dictionary. The result is a three-part commit:

- *Title* -- a hash of the staged diff, encoded through a random dictionary.
- *Body* -- your message, compressed and encoded through a random dictionary.
- *Footer* -- a tag identifying the hash algorithm, compression algorithm, and
  both dictionary names: `[hash:title_dict|compress:body_dict]`.

Raw `git log` and `git show` display encoded glyphs. `mx log` and
`mx show` decode them back to plain text.

When both the title and body randomly land on the _same_ dictionary, a dejavu
marker (`whoa.`) is appended to the footer -- a small easter egg that emerges
from pure chance.

#note[Always use `mx log` to read commit history and `mx show` to inspect
individual commits. Raw `git log` and `git show` output is intentionally
unreadable.]

== Basic usage

Stage your changes and commit with a message:

```bash
mx commit "fix session export crash on empty JSONL"
```

This commits whatever is already staged (via `git add`). If nothing is staged,
the command fails with an error.

To stage all changes automatically before committing:

```bash
mx commit "fix session export crash on empty JSONL" -a
```

To commit and push in one step:

```bash
mx commit "fix session export crash on empty JSONL" -p
```

Both flags compose:

```bash
mx commit "fix session export crash on empty JSONL" -a -p
```

== Flags reference

#command("mx commit [message]",
  [Create an encoded git commit. The message is required unless `--encode-only` is used with `--title` and `--body`.],
  flags: (
    ([`message`], [positional], [Human-readable commit message. Will be compressed and encoded as the commit body.]),
    ([`-a`, `--all`], [flag], [Stage all changes before committing (runs `git add -A`). Skipped during dry-run.]),
    ([`-p`, `--push`], [flag], [Push to the remote after committing. Pulls with rebase first to handle CI version bumps. Sets upstream automatically if needed.]),
    ([`--encode-only`], [flag], [Only generate and print the encoded message to stdout. Does not create a commit. Conflicts with `-a` and `-p`.]),
    ([`-t`, `--title`], [string], [Title text for PR-style encoding. Requires `--encode-only` and `--body`.]),
    ([`-b`, `--body`], [string], [Body text for PR-style encoding. Requires `--encode-only` and `--title`.]),
    ([`--show-encoded`], [flag], [Print the full encoded fields (Title, Body, Dejavu, Footer) instead of just the footer line. Conflicts with `--encode-only`.]),
    ([`--dry-run`], [flag], [Preview encoding and validation without mutating git state. Output is prefixed with `[dry-run]`. Conflicts with `--encode-only`.]),
  ),
)

== Dry run mode

The `--dry-run` flag runs the full encoding and validation pipeline but skips
all git mutations. No staging, no commit, no push. The output is prefixed with
`[dry-run]` on every line so it can never be confused with real output.

```bash
mx commit "add retry logic" --dry-run
```

Output (default):

```
[dry-run] Footer: [sha384:base62|lzma:uuencode]
[dry-run] Would commit.
```

With `--show-encoded`:

```bash
mx commit "add retry logic" --dry-run --show-encoded
```

```
[dry-run] Title:  <encoded glyphs>
[dry-run] Body:   <encoded glyphs>
[dry-run] Footer: [sha384:base62|lzma:uuencode]
[dry-run] Would commit.
```

If `-p` is also set, the preview includes `Would push.`:

```bash
mx commit "add retry logic" --dry-run -p
```

```
[dry-run] Footer: [sha384:base62|lzma:uuencode]
[dry-run] Would commit.
[dry-run] Would push.
```

Dry run still validates that staged changes exist. If there are no staged
changes, it exits with an error (also prefixed with `[dry-run]`).

#tip[Use `--dry-run` to verify your commit will encode cleanly before actually
committing. Useful when testing unfamiliar dictionary configurations.]

== Encode-only mode

The `--encode-only` flag generates encoded output without touching git at all.
It requires both `--title` and `--body` and prints the full three-part encoded
message (title, body, footer) to stdout.

```bash
mx commit --encode-only --title "refactor store" --body "split read/write backends"
```

This is useful for:

- Testing what base-d encoding produces for a given input.
- Generating encoded messages for use outside of git (scripts, PR bodies, etc.).
- Verifying dictionary behavior without needing staged changes.

`--encode-only` conflicts with `-a`, `-p`, `--show-encoded`, and `--dry-run`
because it has its own output path that does not involve git state.

== Show encoded

By default, `mx commit` prints only the footer line and `Committed.` (plus
`Pushed.` if `-p` is set). The encoded title and body are random-glyph noise
from a freshly-rolled dictionary, so they are not useful to read.

The `--show-encoded` flag opts into the full dump:

```bash
mx commit "add retry logic" -a --show-encoded
```

```
Title:  <encoded glyphs>
Body:   <encoded glyphs>
Footer: [sha384:base62|lzma:uuencode]
Committed.
```

When dejavu occurs (both title and body randomly got the same dictionary), an
extra line appears:

```
Title:  <encoded glyphs>
Body:   <encoded glyphs>
Dejavu: true (both used base62)
Footer: [sha384:base62|lzma:base62]
whoa.
Committed.
```

== How encoding works

The encoding uses #link("base-d.html")[base-d], a dictionary-based encoding
system that maps binary data to tokens from randomly selected dictionaries.

+ The staged diff is hashed (e.g. SHA-384) and the hash is encoded through a
  random dictionary. This becomes the commit *title*.
+ The commit message is compressed (e.g. LZMA, Zstd, Brotli, Gzip, LZ4, or
  Snappy) and the compressed bytes are encoded through a second random
  dictionary. This becomes the commit *body*.
+ A footer tag records the algorithms and dictionaries used:
  `[hash_algo:title_dict|compress_algo:body_dict]`.
+ `mx log` and `mx show` read the footer, look up the dictionaries, and
  reverse the process to recover the original message.

If the encoded output contains NUL bytes or control characters (which would
break git), the encoder retries with a different random dictionary, up to 5
attempts. Failed attempts are logged to stderr with the dictionary that
produced unsafe output.

When pushing (`-p`), mx pulls with rebase first to handle CI-pushed version
bumps, then pushes. If no upstream branch is set, it automatically runs
`git push -u origin <branch>`.

For the full encoding specification, see #link("base-d.html")[base-d].
