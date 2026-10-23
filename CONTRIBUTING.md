# Contributing

Welcome! We are glad you are interested in contributing to ephyra. This guide
explains what we ask of contributions and how to work on the code.

## Contributing to code

### Signing commits

Datadog requires all contributors to sign their commits. If you don't sign
your commits yet, follow [GitHub's documentation on setting up signing keys and
signing commits](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits).
Pull requests with unsigned commits cannot be merged.

### New features

Before writing code for a new feature, get your proposal accepted by the
maintainers, so that you don't put effort into something that is already being
worked on in another way or is outside the scope of the project.

Open an issue titled "[RFC] Title of your proposal", using the RFC issue
template. Explain why the feature is needed, why it is useful, and how you plan
to implement it. The maintainers will label the issue `type/feature` or
`type/major_change` and `rfc/discussion` and discuss it with you. If the
proposal is accepted it is labeled `rfc/approved`; start coding then and open a
pull request that links the issue. If it is rejected, the maintainers explain
why, label it `rfc/rejected` and close it.

### Bug fixes

If an issue labeled `type/bug` has no assignee, feel free to claim it and ask a
maintainer to assign you. When your fix is ready, open a pull request and
[link it to the issue](https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue#manually-linking-a-pull-request-to-an-issue-using-the-pull-request-sidebar).
If the fix is not trivial, discuss the options in the issue first, as for a
feature.

### Setting up your development environment

- Rust stable; the minimum supported Rust version is 1.88
  (`rust-toolchain.toml` selects stable).
- `rustup toolchain install nightly-2026-03-26 --component rustfmt`: the
  repository is formatted with this nightly's rustfmt, and CI checks it.
- For the instrumented (coverage) mode: `rustup component add llvm-tools`.

The library is the Cargo workspace at the root (`crates/`). The case studies in
`case-studies/` are separate crates with their own READMEs and prerequisites;
some need `case-studies/sqlite/vendor/fetch.sh` first, or a Turso or Cedar checkout at
the repository root.

### Testing and linting your changes

CI runs, with `RUSTFLAGS=-D warnings` and `RUSTDOCFLAGS=-D warnings`:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo +nightly-2026-03-26 fmt --all --check
ci/package-smoke.sh            # a consumer built against the packaged crates
# instrumented mode, one test thread so coverage is attributed per input:
RUSTFLAGS="-C instrument-coverage" RUST_TEST_THREADS=1 cargo test -p ephyra --features coverage
```

The text and JSON of ephyra's reports are snapshot-tested under
`crates/ephyra/tests/reports/`; a change to what a report says fails until the
new snapshot is accepted (`cargo insta review`).

Third-party dependencies are listed in `LICENSE-3rdparty.csv`; a pull request
that adds or updates one should update that file
([dd-rust-license-tool](https://github.com/DataDog/rust-license-tool)).

### AI code assistants

You are welcome to use AI code assistants for your contributions, but the rest
of these guidelines still apply, whatever the tools used. For example, a new
feature still goes through the RFC process ([above](#new-features)) before any
code is written.

Any contribution, even one made with an AI assistant, must be yours: you are
responsible for understanding the project, its code, and the changes you make.
The maintainers may close a contribution that appears to be heavily generated
by AI without review or tests from the contributor.

## Contributing to issues

### Reporting bugs

If you think you have found a bug in ephyra, please report it. The issue
template asks for the steps to reproduce it, what you expected, and what
happened; the more of it you fill in, the faster we can address it. For a
security vulnerability, follow [SECURITY.md](SECURITY.md) instead of opening a
public issue.

The bugs that ephyra's case studies found in other projects are under
`case-studies/<target>/bugs/`, each with a link to the report filed upstream; report
problems with those projects to them.

### Triaging issues

Triaging issues is a great way to contribute. On an issue opened by someone
else, you can:

- Try to reproduce it. If you can, following the reporter's steps, say so in a
  comment.
- Find duplicates. If the same problem was reported in another issue, comment
  "Duplicate of" followed by that issue's number.
- Ask for missing information, such as steps to reproduce or the affected
  version, by commenting and mentioning the reporter.
