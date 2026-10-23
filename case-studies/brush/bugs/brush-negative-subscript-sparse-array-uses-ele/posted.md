# Negative subscript on a sparse array counts from the number of elements, not from the highest index

Posted as https://github.com/reubeno/brush/issues/1570 on 2026-10-21. The text below is the issue as posted.

---

### brush version

`main` at fc401d784d93aea0e956fff7d9755199a8bf834b (2026-10-20, `brush 0.4.0 (git:fc401d78)`), release build

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
a=(x y); a[5]=z; echo "${a[-1]}"
```

### Actual behavior (brush)

```sh
a=(x y); a[5]=z; echo "${a[-1]}"
# brush: (empty line)
```


### Expected behavior

```sh
a=(x y); a[5]=z; echo "${a[-1]}"
# bash 5.3: z
```

The bash manual (Arrays) says a negative subscript "is interpreted as relative to one greater than the maximum index of the array". Here the maximum index is 5, so `-1` is index 5. brush counts 3 elements and uses index 2, which is unset. Assignment and `unset` are affected too: after `a=(x y); a[5]=z; a[-2]=Q`, bash sets `a[4]` and brush overwrites `a[1]`.

### Additional context

`get_key_for_indexed_array` (`brush-core/src/variables.rs`) adds `values.len()` (the element count) to the negative index instead of the highest index + 1. Negative subscripts came from #468 and #1005, whose tests use dense arrays only.

Potential fix (applies to main (0574ed13) with `git am`):

<details>
<summary>fix.patch</summary>

```diff
From ae497315e196217324419b598ca86c08dfbdaa6d Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Wed, 21 Oct 2026 09:16:18 -0400
Subject: [PATCH] fix(core): count a negative subscript from the highest index
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

A negative subscript of an indexed array counts back from the highest
index plus one, not from the number of elements: for `a=(x y); a[5]=z`,
`${a[-1]}` is `z`. `get_key_for_indexed_array` added the element count to
the negative index, which picks the wrong element in a sparse array.

Add the highest index plus one instead.

Signed-off-by: Alperen Keleş <alpkeles99@gmail.com>
Assisted-by: Claude Code:claude-opus-5-5
---
 brush-core/src/variables.rs                |  9 +++++++--
 brush-shell/tests/cases/compat/arrays.yaml | 11 +++++++++++
 2 files changed, 18 insertions(+), 2 deletions(-)

diff --git a/brush-core/src/variables.rs b/brush-core/src/variables.rs
index 8f3819c6..bd43594c 100644
--- a/brush-core/src/variables.rs
+++ b/brush-core/src/variables.rs
@@ -1059,10 +1059,15 @@ fn get_key_for_indexed_array(
 ) -> Result<u64, error::Error> {
     let mut index_value = index_str.parse::<i64>().unwrap_or(0);
 
-    // Handle negative indices, but check for out-of-range values.
+    // Handle negative indices, but check for out-of-range values. A negative index is
+    // relative to one greater than the maximum index (not the element count, which
+    // differs for sparse arrays).
     #[expect(clippy::cast_possible_wrap)]
     if index_value < 0 {
-        index_value += values.len() as i64;
+        let end = values
+            .last_key_value()
+            .map_or(0, |(k, _)| (*k as i64).saturating_add(1));
+        index_value += end;
         if index_value < 0 {
             return Err(error::ErrorKind::ArrayIndexOutOfRange(index_str.to_owned()).into());
         }
diff --git a/brush-shell/tests/cases/compat/arrays.yaml b/brush-shell/tests/cases/compat/arrays.yaml
index 64da78a2..c639eb9f 100644
--- a/brush-shell/tests/cases/compat/arrays.yaml
+++ b/brush-shell/tests/cases/compat/arrays.yaml
@@ -422,3 +422,14 @@ cases:
       i=1
       declare -a b=([i+1]=y [i*5]=z)
       declare -p b
+
+  - name: "Negative index into sparse indexed array"
+    stdin: |
+      b=(a b c d)
+      b[5]=q
+      echo "<${b[-2]}> <${b[-1]}>"
+      b[-2]=Z
+      b[-1]+=r
+      declare -p b
+      unset 'b[-3]'
+      declare -p b
```

</details>
