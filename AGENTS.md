# Agent Working Agreement

This file is for how agents work in this repository. Product and architecture
rules live in `.agents/notes/`.

## Change workflow

1. Make one focused change.
2. Add or update tests that would fail without the change. For docs-only
   changes, verify formatting and whitespace instead.
3. Run the relevant gates.
4. Commit when everything is green.
5. Stop and ask only when a decision, spec ambiguity, or missing environment
   genuinely blocks progress.

Do not claim a gate passed unless the command was run in this session and its
result was observed. If a gate fails, fix it when the root cause is clear;
otherwise report the failing command and output.

Commit green, tested changes without asking for permission. Ask before
committing when the change alters product scope or an external interface, or
when a required gate cannot be run.

## Verification gates

Use the Strawberry Perl wrapper for Cargo on Windows:

```sh
cargo fmt --all --check
sh tools/cargo-strawberry.sh clippy --workspace --all-targets -- -D warnings
sh tools/cargo-strawberry.sh test --workspace
sh tools/cargo-strawberry.sh test --workspace --no-default-features --features plain
```

For UBL serialisation, validation, or PDF rendering changes, also run:

```sh
sh tools/validate-m1.sh
```

The validation script downloads Mustang into the ignored `target/validator/`
directory and requires Java 17+. It is a development gate, not part of the
offline test suite.

Docs-only changes require:

```sh
git diff --check
cargo fmt --all --check
```

## Commit rules

- Use an imperative subject with a type prefix, e.g.
  `feat: add EN 16931 conformance foundation`.
- Explain what changed and why in the body.
- Write each body paragraph as one unwrapped line.
- Never commit generated artefacts, databases, keystores, or downloaded
  validator jars. `target/` and the secret patterns in `.gitignore` must stay
  ignored.
- Keep the tree clean after committing.

## Project invariants

- Money never touches floating point. Amount arithmetic follows the fixed
  rounding order in `crux`.
- Issued documents are immutable; corrections are credit notes.
- Every validation finding carries a stable code, a BT reference when one
  exists, and a field path the UI can focus.
- UBL output must be byte-deterministic.
- Update `.agents/notes/implementation-plan.md` and the README status when a
  milestone or user-visible capability changes.
- The `plain` build is a development fallback and never ships.
- Editor-facing templates must stay self-contained; see the project-local
  `editor-preview-inputs` skill.
- Do not run the GUI in the terminal; it blocks.
