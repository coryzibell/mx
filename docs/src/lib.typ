// lib.typ — Shared components for mx documentation
//
// Usage: #import "lib.typ": *

// ---------------------------------------------------------------------------
// Admonitions
// ---------------------------------------------------------------------------

// Generic admonition block. Maps to styled HTML via admonition.lua.
#let admonition(kind, body) = {
  block(
    width: 100%,
    inset: 12pt,
    stroke: 0.5pt,
    [*#upper(kind):* #body]
  )
}

#let note(body) = admonition("note", body)
#let warning(body) = admonition("warning", body)
#let deprecated(body) = admonition("deprecated", body)
#let tip(body) = admonition("tip", body)

// ---------------------------------------------------------------------------
// Command reference formatting
// ---------------------------------------------------------------------------

#let command(name, description, flags: (), examples: ()) = {
  [== `#name`]
  [#description]

  if flags.len() > 0 {
    [=== Flags]
    table(
      columns: (auto, auto, auto),
      table.header([*Flag*], [*Type*], [*Description*]),
      ..flags.flatten()
    )
  }

  if examples.len() > 0 {
    [=== Examples]
    for ex in examples {
      raw(ex, lang: "bash", block: true)
    }
  }
}

// ---------------------------------------------------------------------------
// Version markers
// ---------------------------------------------------------------------------

#let since(version) = {
  text(size: 0.85em, fill: rgb("#666"))[_since v#version _]
}

#let deprecated-since(version, replacement) = {
  admonition("deprecated",
    [Deprecated since v#version. Use `#replacement` instead.])
}

// ---------------------------------------------------------------------------
// Page header
// ---------------------------------------------------------------------------

#let page-header(title, description) = {
  [= #title]
  [#description]
  line(length: 100%)
}
