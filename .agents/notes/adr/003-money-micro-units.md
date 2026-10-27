# ADR-003: Money as integer micro-units; no floats

**Status:** Accepted, 2026-10-27

## Context

Invoices are arithmetic that must reconcile to the cent and reproduce years
later. A binary float cannot represent 0.10 exactly, and a cumulative rounding
drift in a VAT total is a filing error, not a cosmetic bug.

## Decision

`Money { micros: i64, currency: Currency }`, where the scale is fixed at 10⁻⁶.
`Quantity` uses the same representation. VAT rates are basis points (`u16`).
Rounding happens only at documented boundaries (`Money::scale_to`, `line_net`,
`tax_amount`), half away from zero. The workspace denies
`clippy::float_arithmetic`, so a float cannot appear in a money path even by
accident.

`rust_decimal` is used only at input/output boundaries, where parsing human
strings is easier than hand-rolling it; it never appears in the model.

## Consequences

- Exact, reproducible arithmetic and stable hashing of amounts.
- Unit prices and quantities keep more precision than the two decimals EN 16931
  allows on totals.
- Developers must remember to scale explicitly; the lints and tests carry that
  weight.

## Alternatives

- **`f64`**: fails the "reconciles to the cent" requirement.
- **`rust_decimal` everywhere**: correct but slower, larger, and still needs a
  storage encoding; the benefit over scaled integers is small here.
