# ADR-010: egui accepted with a documented accessibility limitation

**Status:** Accepted, 2026-10-27

## Context

The brief specifies egui. Immediate-mode GUI is excellent for dense, keyboard-
driven data entry and has a small dependency footprint. It is weak on screen
reader support and platform text-input edge cases, which matters if the product
is sold to public bodies with WCAG obligations.

## Decision

Use egui/eframe, with these constraints:

- All text input goes through egui's `TextEdit`; no custom text widgets.
- Tables are virtualised (`egui_extras`); no widget per row.
- The application layer is UI-agnostic, so a different frontend can be added
  without touching domain or storage code.

Do not market the v1 product to buyers that require WCAG 2.1 AA. Revisit if a
tender makes it a hard requirement.

## Consequences

- Fast iteration and a small binary.
- Accessibility gap is a known limitation, not an accident; it is recorded here
  and in the risk table.
- If a webview frontend is needed later, it reuses the command/query layer.

## Alternatives

- **Tauri/web UI now**: better accessibility and text handling, but violates the
  stated stack constraint and adds a browser runtime.
- **Qt**: licensing and binding risk, plus a much larger surface.
