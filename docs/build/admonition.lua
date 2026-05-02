-- admonition.lua — Pandoc Lua filter for mx docs
--
-- Typst's block() function does not produce a Div in Pandoc's AST.
-- Instead, lib.typ admonitions render as a Para whose first inline
-- is Strong containing "NOTE:" (or WARNING:, DEPRECATED:, TIP:).
--
-- This filter operates at the Blocks level so that multi-paragraph
-- admonitions are fully captured. When an admonition Para is found,
-- all consecutive following Paras (that are not themselves admonitions)
-- are collected into the same Div.

local KINDS = {"NOTE", "WARNING", "DEPRECATED", "TIP"}

-- Check if a Para block starts with a Strong element matching an
-- admonition kind. Returns the kind string or nil.
local function detect_admonition(block)
  if block.t ~= "Para" then return nil end
  if #block.content == 0 then return nil end
  local first = block.content[1]
  if first == nil or first.t ~= "Strong" then return nil end

  local strong_text = pandoc.utils.stringify(first)
  for _, kind in ipairs(KINDS) do
    if strong_text == kind .. ":" then
      return kind
    end
  end
  return nil
end

function Blocks(blocks)
  local result = pandoc.List()
  local i = 1
  while i <= #blocks do
    local block = blocks[i]
    local kind = detect_admonition(block)
    if kind then
      -- Collect this para and all following non-admonition paras
      local collected = pandoc.List({block})
      local j = i + 1
      while j <= #blocks and blocks[j].t == "Para" and not detect_admonition(blocks[j]) do
        collected:insert(blocks[j])
        j = j + 1
      end
      result:insert(pandoc.Div(collected, pandoc.Attr("", {"admonition", kind:lower()})))
      i = j
    else
      result:insert(block)
      i = i + 1
    end
  end
  return result
end
