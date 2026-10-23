# `--translate-to-exnref` / `--emit-exnref` produce invalid IR when a delegate target's unreachable `try` body ends in a none-typed instruction

Posted as https://github.com/WebAssembly/binaryen/issues/9291 on 2026-10-21. The text below is the issue as posted.

---

`wasm-opt` validates the module after running passes, and a pass should not turn a valid module into an invalid one. On the input below, `--translate-to-exnref` (also reached through `--emit-exnref`) does: the validator rejects the pass's output with "block with value must not have last element that is none" and `wasm-opt` aborts with `Fatal: error after opts`.

Versions: the `version_133` release binary (`binaryen-version_133-x86_64-linux.tar.gz`) and `main` at 6e5257bd (2026-10-21).

The input is a `try` that is the target of a `delegate`, whose body is unreachable and ends in a none-typed instruction (`(unreachable) (nop)`). In `TranslateToExnref::processDelegateTarget` (`src/passes/TranslateEH.cpp`), no `br` is appended for an unreachable body, and `Builder::blockifyWithName` then reuses the unnamed body block as the `(result exnref)` trampoline block. That block still ends in `(nop)`, which is invalid once the block has a value. This code has not changed since the pass was added in #6210.

### Reproduction steps

`poc.wat`:

```wat
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
```

```
wasm-opt poc.wat -all -o before.wasm
# expected: exit 0
# version_133: exit 0 (the input is valid)

wasm-opt poc.wat --enable-exception-handling --translate-to-exnref -o out.wasm
# expected: exit 0, a valid out.wasm
# version_133: exit 1
#   [wasm-validator error in function 0] none == none: block with value must not have last element that is none, on
#   (block $outer0 (result exnref)
#    (try_table (catch_all_ref $outer0)
#    )
#    (unreachable)
#    (nop)
#   )
#   Fatal: error after opts

wasm-opt poc.wat -all --emit-exnref -o out.wasm
# expected: exit 0
# version_133: exit 1, same validator error
```

It does not trigger when:

```
wasm-opt before.wasm -all --translate-to-exnref -o out.wasm     # binary input: the reader drops the dead (nop)
# version_133: exit 0
wasm-opt poc.wat -all -O1 --translate-to-exnref -o out.wasm     # -O1 removes the dead code first
# version_133: exit 0
```

and it does not trigger when the body ends in `(unreachable)` itself (remove the `(nop)`).

### Proposed Patch

The patch below wraps such a body in a new block instead of reusing it, and adds a lit test. With it, the input above and the full `test/lit/passes/translate-to-exnref.wast` (both RUN lines) pass.

Applies to main (6e5257bd) with `git am`: save the block below as `fix.patch` and run `git am fix.patch`.

<details>
<summary>fix.patch</summary>

```diff
From c9897d52b449323b25a8ad7bd791f1da42ad6781 Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Wed, 21 Oct 2026 17:38:20 -0400
Subject: [PATCH] [EH] Fix invalid trampoline block for unreachable delegate
 target bodies

When a try that is the target of a delegate has an unreachable body,
TranslateToExnref::processDelegateTarget adds no br to the outer block,
and Builder::blockifyWithName reuses the unnamed body block as the
(result exnref) trampoline block. If that block ends with a none-typed
expression, e.g. `(unreachable) (nop)`, it becomes a block with a value
whose last element is none, and validation after the pass fails with
"block with value must not have last element that is none".

Wrap such a body in a new block instead of reusing it.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
---
 src/passes/TranslateEH.cpp               | 12 ++++++-
 test/lit/passes/translate-to-exnref.wast | 45 ++++++++++++++++++++++++
 2 files changed, 56 insertions(+), 1 deletion(-)

diff --git a/src/passes/TranslateEH.cpp b/src/passes/TranslateEH.cpp
index 23d21e6..94d6060 100644
--- a/src/passes/TranslateEH.cpp
+++ b/src/passes/TranslateEH.cpp
@@ -302,7 +302,17 @@ struct TranslateToExnref : public WalkerPass<PostWalker<TranslateToExnref>> {
       auto* brToOuter = curr->body->type == Type::unreachable
                           ? nullptr
                           : builder.makeBreak(outerBlock->name);
-      innerBody = builder.blockifyWithName(curr->body,
+      // An unreachable body can be a block that ends with a none-typed
+      // expression, which cannot end a block with a value, so wrap it instead
+      // of reusing it as the trampoline block.
+      auto* bodyBlock = curr->body->dynCast<Block>();
+      Expression* body = curr->body;
+      if (!brToOuter && bodyBlock && !bodyBlock->name.is() &&
+          !bodyBlock->list.empty() &&
+          bodyBlock->list.back()->type != Type::unreachable) {
+        body = builder.makeBlock(curr->body);
+      }
+      innerBody = builder.blockifyWithName(body,
                                            delegateTrampoline,
                                            brToOuter,
                                            Type(HeapType::exn, Nullable));
diff --git a/test/lit/passes/translate-to-exnref.wast b/test/lit/passes/translate-to-exnref.wast
index b416850..11b0ecb 100644
--- a/test/lit/passes/translate-to-exnref.wast
+++ b/test/lit/passes/translate-to-exnref.wast
@@ -1827,6 +1827,51 @@
     )
   )
 
+  ;; CHECK:      (func $delegate-target-outer-try-unreachable-ends-with-none (type $1)
+  ;; CHECK-NEXT:  (try_table
+  ;; CHECK-NEXT:   (throw_ref
+  ;; CHECK-NEXT:    (block $l00 (result exnref)
+  ;; CHECK-NEXT:     (block
+  ;; CHECK-NEXT:      (try_table (catch_all_ref $l00)
+  ;; CHECK-NEXT:       (call $foo)
+  ;; CHECK-NEXT:      )
+  ;; CHECK-NEXT:      (return)
+  ;; CHECK-NEXT:      (nop)
+  ;; CHECK-NEXT:     )
+  ;; CHECK-NEXT:    )
+  ;; CHECK-NEXT:   )
+  ;; CHECK-NEXT:  )
+  ;; CHECK-NEXT: )
+  ;; STACKIR-OPT:      (func $delegate-target-outer-try-unreachable-ends-with-none (type $1)
+  ;; STACKIR-OPT-NEXT:  try_table
+  ;; STACKIR-OPT-NEXT:   block $l00 (result exnref)
+  ;; STACKIR-OPT-NEXT:    try_table (catch_all_ref $l00)
+  ;; STACKIR-OPT-NEXT:     call $foo
+  ;; STACKIR-OPT-NEXT:    end
+  ;; STACKIR-OPT-NEXT:    return
+  ;; STACKIR-OPT-NEXT:   end
+  ;; STACKIR-OPT-NEXT:   throw_ref
+  ;; STACKIR-OPT-NEXT:  end
+  ;; STACKIR-OPT-NEXT:  unreachable
+  ;; STACKIR-OPT-NEXT: )
+  (func $delegate-target-outer-try-unreachable-ends-with-none
+    ;; Same as above, but the unreachable body ends with a none-typed
+    ;; instruction. The body block should not be reused as the exnref-typed
+    ;; trampoline block, which would be invalid.
+    (try $l0
+      (do
+        (try
+          (do
+            (call $foo)
+          )
+          (delegate $l0)
+        )
+        (return)
+        (nop)
+      )
+    )
+  )
+
   ;; CHECK:      (func $delegate-target-caller-none (type $1)
   ;; CHECK-NEXT:  (throw_ref
   ;; CHECK-NEXT:   (block $__binaryen_delegate_caller_target0 (result exnref)
-- 
2.56.0

```

</details>

### Notes

- Related, but different bugs in the same pass: #6274, #6518, #6519.
- Found by an automated differential tester (Ephyra); the cause analysis was drafted with AI assistance.
