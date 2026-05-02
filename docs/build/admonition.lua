-- admonition.lua — Pandoc Lua filter for mx docs
--
-- Typst's block() function does not produce a Div in Pandoc's AST.
-- Instead, lib.typ admonitions render as a Para whose first inline
-- is Strong containing "NOTE:" (or WARNING:, DEPRECATED:, TIP:).
--
-- This filter wraps those paragraphs in a styled Div with appropriate
-- CSS classes for the site stylesheet.

local KINDS = {"NOTE", "WARNING", "DEPRECATED", "TIP"}

function Para(el)
  -- Check if first inline is a Strong element
  if #el.content == 0 then return nil end
  local first = el.content[1]
  if first == nil or first.t ~= "Strong" then return nil end

  local strong_text = pandoc.utils.stringify(first)

  for _, kind in ipairs(KINDS) do
    if strong_text == kind .. ":" then
      -- Build the content: everything after the Strong and the
      -- space that follows it
      local inlines = pandoc.List()
      local skip_space = true
      for i = 2, #el.content do
        if skip_space and el.content[i].t == "Space" then
          skip_space = false
        else
          skip_space = false
          inlines:insert(el.content[i])
        end
      end

      -- Create the admonition div
      local label = pandoc.Para({
        pandoc.Strong({pandoc.Str(kind .. ":")}),
        pandoc.Space(),
        table.unpack(inlines)
      })
      return pandoc.Div(
        {label},
        pandoc.Attr("", {"admonition", kind:lower()})
      )
    end
  end

  return nil
end
