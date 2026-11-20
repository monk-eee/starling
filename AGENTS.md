# Working on Starling

- Use the [developer guide](developer.md) and [public workload example](examples/local.yaml)
  for current behavior. The [specification](docs/SPEC.md) also contains future
  interfaces; an example in the spec is not proof that a command exists.
- Check the [decision index](docs/adr.d/README.md) when changing telemetry,
  measurements, or exported data. Distinguish a proposed ADR from acceptance.
  Don't invent a decision-maker, release, or verification result.
- Build the frontend before compiling Rust. Use the existing Cargo and npm
  lockfiles. Prefer portable CLI examples and exercise the relevant macOS or
  Linux container path; report what actually ran.
- Keep workload inputs, full target paths, raw results, keys and generated
  reports out of public changes. Use the built-in fixture for examples and
  tests. Don't register a whole new example directory as public just to make
  a local scenario compile.
- For a user-visible change, update the relevant guide and add a
  [changelog fragment](changelog.d/README.md). The
  [pre-commit setup](CONTRIBUTING.md#optional-pre-commit-hook) is opt-in;
  don't change someone else's Git configuration.
