# Generator exercises

Three standalone consumers of the local ephyra library, each with one
`#[ephyra::generator]` predicate and no generator for it. Each owns its
Cargo workspace and lockfile; none is a member of the root workspace.

| Project | Predicate | What it exercises |
|---|---|---|
| `positive` | `value > 0` over `u8` | Small scalar domain |
| `ordered-pair` | `lo < hi` over `(u8, u8)` | Related fields, not independent generation |
| `sorted-vec` | Nondecreasing `&[u8]`, generated as `Vec<u8>` | Collections, empty inputs, size variation |

`ephyra check` in a project lists its predicate as needing a generator.
Ask a coding agent to write one (`ephyra guide` explains how) until
`ephyra check` passes:

```sh
cargo build -p ephyra-cli
cd examples/synthesis/sorted-vec
../../../target/debug/ephyra check --list   # what is missing
../../../target/debug/ephyra check          # check what was written
```

The check runs thousands of inputs through the generator, its shrinker and its
mutator against the predicate, with coverage and any metrics the generator
declares. These are finite checks, not proofs of soundness, completeness or
distribution quality. CI only checks that the projects compile.
