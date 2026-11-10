def main : IO Unit := do
  IO.println (repr <| (",x".toSlice.split ',').toList.map (·.copy))
  IO.println (repr <| (",x".toSlice.revSplit ',').toList.map (·.copy))
  IO.println (repr <| ("x,".toSlice.revSplit ',').toList.map (·.copy))
  IO.println (repr <| ("".toSlice.revSplit ',').toList.map (·.copy))
