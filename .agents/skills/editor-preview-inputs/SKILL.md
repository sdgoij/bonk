---
name: editor-preview-inputs
description: Use when a template, document, schema, or config file is consumed by both an editor language server and an application runtime. Keep editor previews self-contained, pass real runtime data through explicit inputs, and test both paths so false diagnostics never become background noise.
---

# Editor preview inputs

## When to use

Use this skill when a file is compiled or validated in two contexts:

- an editor language server, previewer, or formatter opens it in place;
- the application renders or executes it with real data at runtime.

Typical cases: Typst templates, Jinja/Handlebars templates, JSON schemas,
GraphQL documents, config files with environment interpolation.

## Core rule

Treat editor tooling as a first-class consumer.

An editor preview must be able to compile with **no implicit filesystem
state**. Real runtime data must enter through an **explicit input**. Both paths
must have a test.

A false diagnostic is not harmless. Ignoring warnings teaches everyone to
ignore warnings, so fix the cause instead of suppressing the language server.

## Pattern

1. Give the artifact a small, valid, inline preview default.
2. Let the runtime override it through a declared input.
3. Test the editor path and the runtime path separately.

### Typst example

Template:

```typst
// The renderer passes --input data=data.json.
// The inline payload keeps editors and previewers self-contained.
#let data = if "data" in sys.inputs {
  json(sys.inputs.data)
} else {
  (
    number: "2026-0001",
    issue_date: "2026-10-27",
    due_date: "2026-11-26",
    currency: "EUR",
    seller: (name: "Seller BV", address: "Main 1, Amsterdam, NL", vat: "NL123456782B01"),
    customer: (name: "Customer BV", address: "Rotterdam, NL", vat: none),
    lines: ((name: "Consulting", qty: "2", price: "50", vat: "21", total: "100.00"),),
    net: "100.00",
    tax: "21.00",
    gross: "121.00",
    notes: none,
  )
}
```

Runtime:

```rust
.arg("--input")
.arg("data=data.json")
```

Tests:

- Compile the template with no `--input`; this is the editor path.
- Render with the real payload; this is the production path.
- Keep the inline preview minimal but structurally identical to the real data.

## Checklist

- [ ] The artifact compiles or validates in place with no sibling data file.
- [ ] Runtime data is passed through an explicit named input.
- [ ] The editor path has a test that does not depend on runtime setup.
- [ ] The runtime path has a test that includes real serialization/parsing.
- [ ] Editor diagnostics are genuinely clean, not suppressed.
- [ ] Any unavoidable preview fixture is valid and contract-tested.

## Anti-patterns

- Adding an empty or placeholder `data.json` just to silence the LSP.
- Disabling the language server because its error is inconvenient.
- Relying on the current working directory or an untracked file for previews.
- Testing only the runtime path and assuming the editor path still works.
- Keeping a preview fixture that is not validated against the real schema; it
  will drift and produce the next false error.
