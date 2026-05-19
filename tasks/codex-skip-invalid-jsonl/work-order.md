## Problem

`mx codex save` fails entirely when any JSONL line is invalid:

```
Error: Failed to parse JSONL line

Caused by:
    expected value at line 1 column 1
```

A single null-byte line (line 2000 out of 2956) caused the entire session archive to fail. The other 2955 lines were perfectly valid JSON. Workaround was filtering the bad line with Python and feeding the clean file back.

## Root Cause

Line 2000 contained all `\x00` null bytes — likely a sparse write or interrupted flush during heavy parallel companion I/O.

## Proposed Fix

When parsing JSONL lines during `codex save`:
1. Skip lines that fail JSON parsing instead of returning an error
2. Log a warning: `"Skipping invalid JSONL line {N}: {first 50 chars}"`
3. Include a count of skipped lines in the final summary: `"Archived session (2955/2956 lines, 1 skipped)"`
4. If ALL lines are invalid, then fail — that's a genuinely corrupt file

One bad byte should never nuke an entire session archive.

## Impact

Without this fix, any session with a single corrupted byte loses its entire codex archive unless manually cleaned.

---

## Repo

This is the mx codebase. Rust. `cargo test` to run tests. The codex module is at `src/codex/`.

## Done Looks Like

Invalid JSONL lines don't kill the archive. All existing tests still pass. New tests cover the error paths.
