import Lean.Data.NameTrie
open Lean

-- Keys: `a` ↦ 1 and `a.b.c` ↦ 2. The node for `a.b` exists but holds no value.
def t : NameTrie Nat := ((∅ : NameTrie Nat).insert `a 1).insert `a.b.c 2

#version
#eval t.findLongestPrefix? `a.x    -- longest key that is a prefix: `a`
#eval t.findLongestPrefix? `a.b.d  -- longest key that is a prefix: `a`
