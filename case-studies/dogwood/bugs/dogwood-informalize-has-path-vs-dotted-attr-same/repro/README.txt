run.sh <path-to-dogwood-binary>
  Prints `dogwood explain` and `dogwood replay` for policy-path.dw and policy-dotted.dw.
  Exit 1: the two explanations are identical (bug reproduces). 0: they differ.
  Build the binary from an upstream checkout: cargo build --release -p dogwood-cli  (-> target/release/dogwood)
variants.sh <path-to-dogwood-binary>
  Prints the explanation line for four pairs of conditions.
