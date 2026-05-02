#!/usr/bin/env bash
set -euo pipefail

# mx docs build script
# Converts Typst source files to HTML (site) and Markdown (LLM context).
#
# LOCAL PREVIEW: ruby -run -e httpd docs/out/html -p 8080

DOCS_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC_DIR="$DOCS_DIR/src"
BUILD_DIR="$DOCS_DIR/build"
OUT_HTML="$DOCS_DIR/out/html"
OUT_LLM="$DOCS_DIR/out/llm"

TEMPLATE="$BUILD_DIR/template.html"
STYLESHEET="$BUILD_DIR/style.css"
ADMONITION_FILTER="$BUILD_DIR/admonition.lua"
CROSSREF_FILTER="$BUILD_DIR/crossref.lua"

# Ordered list for LLM output (getting-started first, architecture last)
LLM_ORDER=(
  index getting-started commit log memory codex kv state
  sync pr github convert session wiki heartbeat
  base-d paths architecture
)

# --- Setup ---

mkdir -p "$OUT_HTML" "$OUT_LLM"
cp "$STYLESHEET" "$OUT_HTML/style.css"

# Temp dir for per-page metadata files (nested YAML keys needed for
# Pandoc template conditionals — --metadata "a.b=true" doesn't nest)
META_TMP="$(mktemp -d)"
trap 'rm -rf "$META_TMP"' EXIT

# --- HTML build ---

count=0
for src in "$SRC_DIR"/*.typ; do
  name="$(basename "$src" .typ)"

  # Skip lib.typ — it's a shared component file, not a page
  [ "$name" = "lib" ] && continue

  # Write a tiny YAML file so the template can test current-page.<name>
  # Also derive a human-readable title from the filename for <title>.
  meta_file="$META_TMP/$name.yaml"
  page_title="$(echo "$name" | sed 's/-/ /g; s/\b\(.\)/\u\1/g')"
  printf 'current-page:\n  %s: true\ntitle: "%s"\n' "$name" "$page_title" > "$meta_file"

  pandoc "$src" \
    -f typst \
    -t html \
    --standalone \
    --mathml \
    --toc \
    --section-divs \
    --template "$TEMPLATE" \
    --css style.css \
    --lua-filter "$ADMONITION_FILTER" \
    --lua-filter "$CROSSREF_FILTER" \
    --metadata-file "$meta_file" \
    -o "$OUT_HTML/$name.html"

  count=$((count + 1))
done

# --- LLM markdown build ---

# Collect only the source files that exist, in the specified order
llm_sources=()
for name in "${LLM_ORDER[@]}"; do
  src="$SRC_DIR/$name.typ"
  [ -f "$src" ] && llm_sources+=("$src")
done

if [ ${#llm_sources[@]} -gt 0 ]; then
  pandoc "${llm_sources[@]}" \
    -f typst \
    -t markdown \
    --lua-filter "$ADMONITION_FILTER" \
    --lua-filter "$CROSSREF_FILTER" \
    --lua-filter "$BUILD_DIR/strip-links.lua" \
    -o "$OUT_LLM/mx-docs.md"
fi

# --- Summary ---

echo "mx docs build complete"
echo "  HTML: $count pages -> $OUT_HTML/"
echo "  LLM:  $([ -f "$OUT_LLM/mx-docs.md" ] && echo "$OUT_LLM/mx-docs.md" || echo "(none)")"
