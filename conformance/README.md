# The conformance suite

One directory per case; three languages run every case and must agree.
This is PATH-TO-1.0 §1 made real: the executable definition of what a
layer stack resolves to, versioned WITH the engine semantics it pins.

```text
cases/<name>/
  config.toml     the document(s) — `config.<profile>.toml` variants sit beside it
  env.json        environment variables to set for the case (own prefix per case!)
  args.json       the spec: key, env_prefix, profile_env, defaults, set,
                  overrides, secrets_dir, env_files, aliases, whole_document
  expected.json   the resolved document, exactly
```

Runners: the engine's `tests/conformance.rs`; the Python and Node
repositories each carry a ~50-line runner that fetches this directory
as a tarball pinned to an engine ref in their CI. A disagreement names
the case — and a disagreement IS a finding, never something to paper
over in a runner.

Rules for new cases: one behaviour per case; a case's environment
variables use a prefix unique to it; nothing in any file is a real
credential shape (the secret-scrubbing suites live elsewhere — this
suite is about resolution).
