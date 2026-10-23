(module
  (func (export "f")
    (try $outer
      (do
        (try
          (do)
          (delegate $outer))
        (unreachable)
        (nop))
      (catch_all))))
