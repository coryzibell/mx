-- crossref.lua — Pandoc Lua filter for mx docs
--
-- Fix cross-reference display text. When Pandoc produces links with
-- bracket-wrapped display text like "[getting-started]", clean them
-- up to human-readable form: "getting started".

function Link(el)
  local display = pandoc.utils.stringify(el.content)
  if display:match("^%[.*%]$") then
    local clean = display:sub(2, -2):gsub("-", " ")
    el.content = {pandoc.Str(clean)}
  end
  return el
end
