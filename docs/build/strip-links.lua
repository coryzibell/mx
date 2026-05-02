-- strip-links.lua — Pandoc Lua filter for mx docs LLM output
--
-- Replaces hyperlinks with their display text. The LLM markdown
-- output has no HTML pages to link to, so .html hrefs are meaningless.
-- Stripping links keeps the prose readable without dead references.

function Link(el)
  return el.content
end
