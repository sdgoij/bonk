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

Every recipe lives in the `Justfile`; `just` is the one place to look for the
commands, and the recipes pick the Strawberry Perl wrapper on Windows by
themselves. `just gate` runs the five gates in the order CI does. The licence
and advisory gate needs `cargo-deny`, which Cargo does not ship:

```sh
cargo install cargo-deny --locked
```

The first run fetches the advisory database; later runs use the cache. If the
tool is missing, `just deny` fails and the pre-commit hook skips that one check
with a warning, so a fresh clone can still commit.

Enable the pre-commit gate once per clone (`just hooks` does the same); it is a
convenience, not a replacement for the gates below:

```sh
git config core.hooksPath .githooks
```

The hook runs formatting and clippy through the Strawberry wrapper and blocks
the commit on failure. `BONK_PRECOMMIT_TESTS=1` adds the workspace tests.

The gates, spelled out. On Windows these need the Strawberry Perl wrapper, which
`just` applies for you:

```sh
just fmt
just clippy
just deny
just test
just test-plain
```

For UBL serialisation, validation, or PDF rendering changes, also run:

```sh
just validate-m1
```

For VAT-category, exemption-reason, or breakdown changes, also run:

```sh
just validate-vat
```

The validation scripts download Mustang into the ignored `target/validator/`
directory and require Java 17+. They are development gates, not part of the
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
- Tracked text is LF in the index and in the working tree, on every platform;
  `.gitattributes` pins that. Do not commit CRLF files or a CRLF-only diff.
- Never commit generated artefacts, databases, keystores, or downloaded
  validator jars. `target/` and the secret patterns in `.gitignore` must stay
  ignored.
- Keep the tree clean after committing.

## Project invariants

- Money never touches floating point. Amount arithmetic follows the fixed
  rounding order in `crux`.
- Issued documents are immutable; corrections are credit notes.
- Every schema migration declares a down migration when a safe rollback exists,
  or explicitly marks itself irreversible. A down migration must never delete
  issued-document or audit evidence.
- Every validation finding carries a stable code, a BT reference when one
  exists, and a field path the UI can focus.
- UBL output must be byte-deterministic.
- Update `.agents/notes/implementation-plan.md` and the README status when a
  milestone or user-visible capability changes.
- The `plain` build is a development fallback and never ships.
- Editor-facing templates must stay self-contained; see the project-local
  `editor-preview-inputs` skill.
- Do not run the GUI in the terminal; it blocks.
