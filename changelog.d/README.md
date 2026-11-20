# Changelog fragments

For a change someone using or operating Starling will notice, add a file in
this directory instead of editing the release history in every PR. Use
`short-slug.category.md`, with lowercase letters, digits and hyphens in the
slug. Categories: `added`, `changed`, `deprecated`, `removed`, `fixed`,
`security`. For example, `report-export.fixed.md` would describe a change to
export behavior.

Write a short paragraph, without a heading or bullet. Say **what the reader
can now do or must do differently**. Don't paste a commit title, private
workload, or vulnerability reproduction. Pure refactoring, tests and spelling
fixes do not need a fragment.

**At release:** group pending fragments by category in
[CHANGELOG.md](../CHANGELOG.md). Move the `Unreleased` baseline into the
actual version/date entry, add the grouped changes without duplicating them,
and leave a fresh `Unreleased` heading. Remove only the fragments included
in that release, in the same change. This is a manual rollup; committing a
fragment does not publish a release.
