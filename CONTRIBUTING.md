# Contributing to Starling

Start with the [developer guide](developer.md#build-and-run-from-a-checkout).
Build the dashboard before Cargo: the Rust binary embeds the web build output.
Use a branch for your change and open a focused pull request against `main`.
Use the [PR template](.github/pull_request_template.md) to describe what
changes for the CLI user, bird operator, or dashboard reader. For an
agent-assisted change, also see [AGENTS.md](AGENTS.md).

## Before committing

1. Use synthetic input and the local fixture. Never stage private workloads,
   reports, keys, or generated web/build output. The public examples are
   [local.yaml](examples/local.yaml) and [local.toml](examples/local.toml).
2. Inspect what will be published, not just what's in your editor:

   ```sh
   git status --short
   git diff --cached --stat
   git diff --cached --check
   ```

3. If behavior changes, update its user or developer documentation and add a
   [changelog fragment](changelog.d/README.md). If a change alters a public
   boundary or measurement, consult the [decision index](docs/adr.d/README.md).
   A proposal is not evidence that a maintainer approved the change.

### Optional pre-commit hook

Install [pre-commit](https://pre-commit.com/#install) 3.5 or newer (for
example, `pipx install pre-commit`), and have Git, Cargo, and the repo's
`rustfmt` component on `PATH`. From the repository root:

```sh
pre-commit validate-config
pre-commit run --all-files     # Try the checks without installing a Git hook.
pre-commit install             # Opt in for this clone.
```

After editing, stage the files you intend to commit and use `pre-commit run`
to check that set. The [configuration](.pre-commit-config.yaml) checks
staged diff whitespace/conflict markers, Rust formatting, changelog filenames,
local-only paths, and PEM private key headers. The diff check always examines
the **index**, even with `--all-files`. The hooks do not rewrite files; if
formatting fails, run `cargo fmt --all`, review the diff, and restage it.

Installing the hook is optional and is **not** a branch protection rule. If
your clone already uses a custom hook or `core.hooksPath`, use the one-off
command rather than replacing it. `pre-commit uninstall` removes this
framework's installed hook from your clone. These checks are not a full
secret scan and do not replace the test suite.

When changing the hook configuration, run `python3 tests/contributor_hooks.py`.
It installs the actual hook in a disposable Git repository, attempts both a
valid and an invalid commit, and checks that unstaged edits survive rejection.

## Before opening a PR

- Run the [checks for the affected surface](developer.md#test-changes). For a
  telemetry change, include the [two-container check](developer.md#verify-two-linux-containers);
  for browser behavior, include the Playwright flow. A docs-only change can
  be checked with local links and the commands it changes.
- Give reviewers a reproduction or before/after result using the fixture.
  State which platforms you actually tested and which you did not. The
  [GitHub workflow](.github/workflows/cli.yml) checks macOS and Linux
  containers and publishes separate [Rust and browser coverage reports](developer.md#measure-test-coverage).
  It does not run every check in the developer guide.
- If the issue may be a vulnerability, follow [SECURITY.md](SECURITY.md)
  instead of posting private evidence in the PR.
