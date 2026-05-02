-- crossref.lua — Pandoc Lua filter for mx docs
--
-- Fix cross-reference display text. When Pandoc produces links with
-- bracket-wrapped display text like "[getting-started]", clean them
-- up to human-readable form: "getting started".
--
-- STATUS: Forward-looking. Currently no Typst source pages use @label
-- cross-references, so this filter is effectively inert. It does no
-- harm in the pipeline and will activate automatically when future
-- pages use Typst @label references, which Pandoc renders with
-- bracket-wrapped display text. Kept in the build to avoid a gap
-- when that day comes.

function Link(el)
  local display = pandoc.utils.stringify(el.content)
  if display:match("^%[.*%]$") then
    local clean = display:sub(2, -2):gsub("-", " ")
    el.content = {pandoc.Str(clean)}
  end
  return el
end
