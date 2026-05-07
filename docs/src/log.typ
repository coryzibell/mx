#import "lib.typ": *

#page-header("log", "Decoded git log for encoded commits.")

== Overview

`mx log` decodes the commit history that `mx commit` encodes. Because
#link("commit.html")[`mx commit`] compresses and encodes every commit message
through a randomly selected #link("base-d.html")[base-d] dictionary, raw
`git log` output is unreadable glyphs. `mx log` reverses the encoding and
displays your original messages.

The round-trip works because each encoded commit carries a footer tag that
identifies the dictionary and compression algorithm used. `mx log` reads the
footer, looks up the dictionary, decompresses the body, and prints the
human-readable message.

#note[Always use `mx log` to read commit history. Raw `git log` will show
encoded noise.]

== Basic usage

Show the last 10 commits (the default):

```bash
mx log
```

Show the last 20 commits:

```bash
mx log -n 20
```

Show full commit details (hash, author, date, decoded message):

```bash
mx log --full
```

== Output formats

*Compact* (default) -- one line per commit: short hash and decoded message,
truncated to 72 characters.

```
a1b2c3d fix session export crash on empty JSONL
e4f5g6h add retry logic to sync pull
```

*Full* (`--full`) -- full hash, author, date, and decoded message, styled like
`git log`. If the commit has trailing post-footer content (e.g. a dejavu
marker), it is rendered in dim text beneath the decoded message.

== Flags reference

#command("mx log",
  [Display decoded git log. Commits encoded by `mx commit` are decoded back to their original messages. Non-encoded commits pass through unchanged.],
  flags: (
    ([`-n`, `--count`], [integer], [Number of commits to show. Defaults to `10`.]),
    ([`--full`], [flag], [Show full commit details: full hash, author, date, and complete decoded message. Without this flag, output is one compact line per commit.]),
  ),
)

=== Trailing arguments

Any additional arguments after the flags are passed through to the underlying
`git log` call. This lets you filter by path, author, date range, or any other
git-log option:

```bash
mx log -- src/handlers/mod.rs
mx log -n 5 --full -- docs/
mx log -- --author="charlie"
```

== Relationship to mx commit and mx show

`mx commit`, `mx log`, and #link("show.html")[`mx show`] form the encoding
round-trip:

+ `mx commit` compresses your message, encodes it through a random dictionary,
  and writes the encoded result as the git commit body with a footer tag.
+ `mx log` reads the footer tag, reverses the encoding, decompresses, and
  displays your original message across the commit history.
+ `mx show` does the same decoding for individual commits, replacing
  `git show`.

Non-encoded commits (e.g. commits made with raw `git commit`) pass through
unchanged -- both `mx log` and `mx show` detect the absence of a footer tag
and display the original message.

For the full encoding specification, see #link("commit.html")[commit].
