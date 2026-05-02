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

// Heading level is hardcoded to h2. This is intentional: each command page
// uses h1 for the page title, h2 for individual commands, and h3 for
// sub-sections like flags and examples. If the page structure changes (e.g.,
// grouping commands under h2 categories), this level may need to become a
// configurable parameter.
#let command(name, description, flags: (), examples: ()) = {
  heading(level: 2, raw(name))
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
    [Deprecated since v#version. Use #raw(replacement) instead.])
}

// ---------------------------------------------------------------------------
// Page header
// ---------------------------------------------------------------------------

#let page-header(title, description) = {
  [= #title]
  [#description]
  line(length: 100%)
}
