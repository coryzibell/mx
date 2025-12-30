# Design: Wake Ritual Improvements

**Issues:** #72 (Multiple wake phrases), #73 (Custom wake order)
**Status:** Design Review

See full design document: `~/.crewu/vol/designs/wake-ritual-improvements.md`

## Summary

This PR contains the architectural design for two wake ritual enhancements:

### Issue #72: Multiple Wake Phrases

Convert `wake_phrase: Option<String>` to `wake_phrases: Vec<String>` to support multiple verification phrases per bloom. During rituals, a random phrase is selected, testing comprehension rather than rote recall.

**Key decisions:**
- Array field (not separate relation table) - simpler, phrases are intrinsic to blooms
- Random selection at ritual time
- CLI: `--wake-phrases "phrase1,phrase2,phrase3"`

### Issue #73: Custom Wake Order

Add `wake_order: Option<i32>` field to allow emotional/narrative sequencing independent of resonance values. A high-resonance bloom can come later in the ritual if wake_order places it there.

**Key decisions:**
- Simple integer field (not linked list or separate sequence table)
- Ordered entries first (by wake_order ASC), then unordered (by resonance DESC)
- CLI: `--wake-order 50`

## Implementation Order

1. Schema foundation (both fields)
2. Wake order implementation (#73)
3. Multiple phrases implementation (#72)
4. Polish and deprecate old field

## Files to Change

- `src/knowledge.rs` - Add new fields
- `schema/surrealdb-schema.surql` - Schema updates
- `src/surreal_db.rs` - Query modifications
- `src/wake_ritual.rs` - Random phrase selection
- `src/main.rs` - CLI flags
- `src/wake_token.rs` - Wire format updates
