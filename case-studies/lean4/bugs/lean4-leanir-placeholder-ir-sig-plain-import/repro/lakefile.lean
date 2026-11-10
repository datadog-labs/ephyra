import Lake
open Lake DSL

package l where
  leanOptions := #[⟨`compiler.postponeCompile, true⟩]

@[default_target]
lean_lib L
