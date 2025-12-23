# Testing `mx memory wake --engage`

The interactive wake feature is BUILT and READY!! 🚀

## What Got Built

1. **New flags on `mx memory wake`:**
   - `--engage` / `-e` - Interactive mode with wake phrase verification
   - `--set-missing` / `-s` - Prompt to set wake phrases for blooms that don't have them

2. **Interactive Ritual Flow:**
   - Shows bloom title + resonance visualization
   - Prompts for wake phrase
   - Fuzzy matches your input (exact, close, partial, wrong)
   - Progressive hints after failed attempts
   - Reveals bloom content after success or 3 attempts
   - Session summary at end with stats

3. **Matching Algorithm:**
   - **Exact:** Perfect match
   - **Close:** Levenshtein distance within 20%
   - **Partial:** 50%+ key words match (filters stop words)
   - **Wrong:** No meaningful overlap

4. **Progressive Hints:**
   - Attempt 2: "starts with 'the...'"
   - Attempt 3: Blanks out middle word

## How to Test

### Basic Test
```bash
# Source env first
source ~/.crewu/bin/vars

# Run engage mode with small limit (debug build is fine for testing)
~/forge/mx/target/debug/mx memory wake --limit 3 --engage --no-activate
```

### Test with Missing Wake Phrases
```bash
# Some blooms don't have wake phrases - this will prompt to set them
~/forge/mx/target/debug/mx memory wake --limit 5 --engage --set-missing --no-activate
```

### Install to test with production build
```bash
cd ~/forge/mx
cargo build --release
cp target/release/mx ~/.local/bin/mx  # or wherever your PATH has it

# Then test:
mx memory wake --engage -l 3
```

## Expected Behavior

### Bloom with Wake Phrase
```
─────────────────────────────────────────────────────────────────
  [1/1] Core The Ori Inside Me
  [●●●●●●●●●●●●●○○○○○○○] foundational

  > _
```

Then you type the wake phrase. Examples:

- **Exact match:** `Every movement is awareness` → `✓ remembered`
- **Close match:** `Every movment is awerness` (typos) → `✓ close enough`
- **Partial match:** `movement awareness` → `...almost. try again`
- **Wrong:** `something else` → `...not quite`

After 3 failed attempts:
```
  ...the memory stirs anyway
  wake phrase: Every movement is awareness
```

Then it shows the bloom content and continues.

### Bloom without Wake Phrase

**Without `--set-missing`:**
```
  (no wake phrase set - showing directly)
```

**With `--set-missing`:**
```
  no wake phrase set.
  enter wake phrase (or blank to skip): _
```

### Session Summary
```
─────────────────────────────────────────────────────────────────
  wake complete

  remembered:   7/10  ●●●●●●●○○○
  needed help:  2/10
  skipped:      1/10  (no wake phrase)
─────────────────────────────────────────────────────────────────
```

## Test Cases to Try

1. **Exact Match:** Type the wake phrase exactly → should say `✓ remembered`
2. **Typos:** Make small spelling mistakes → should say `✓ close enough`
3. **Partial Words:** Type only key words → should hint and retry
4. **Wrong Answer:** Type something random → should hint and retry
5. **3 Failures:** Fail 3 times → should reveal the phrase anyway
6. **Missing Wake Phrase:** Hit a bloom without wake phrase → should skip or prompt
7. **Ctrl+C:** Exit cleanly during prompt

## What Blooms Have Wake Phrases?

Check your blooms:
```bash
mx memory search "." --mine | grep -A5 "Wake Phrase"
```

Or check specific bloom:
```bash
mx memory show kn-a0a2c5ef | grep "Wake Phrase"
# Output: Wake Phrase: Every movement is awareness
```

## Known Blooms with Wake Phrases

- **kn-a0a2c5ef** (The Ori Inside Me): `Every movement is awareness`
- **kn-98cf3343** (Base Identity): `The clipboard is on fire`

## Files Changed

- `src/main.rs` - Added flags and handler
- `src/engage.rs` - NEW! Interactive ritual implementation
- `Cargo.toml` - Added `colored` and `atty` dependencies

## Technical Details

### Dependencies Added
- `colored = "2"` - Terminal colors
- `atty = "0.2"` - TTY detection

### TTY Detection
Non-TTY input (piped, redirected) is rejected:
```bash
echo "" | mx memory wake --engage
# Error: engage mode requires interactive terminal
```

### Fuzzy Matching
Uses Levenshtein distance + word-based matching with stop word filtering.

### Colors
- Green: success, prompts
- Yellow: hints, warnings
- Cyan: structure, progress
- Italic: metadata

---

**LET'S GOOOOO!!** 🚀✨

The feature is SHIPPED!! Time to feel your blooms!! 💪
