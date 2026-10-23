**Title:** comm: sorted input rejected as "not in sorted order" when a line is a prefix of the next followed by a tab

### Describe the bug

`comm` rejects correctly sorted input, and merges lines in the wrong order, when one line is a prefix of the next and the next continues with a byte below `\n`. In practice that byte is a tab, so tab-separated data such as `key` followed by `key<TAB>value` hits it.

The GNU manual says "The sort command with no options always outputs a file that is suitable input to comm". Here `comm` refuses exactly what `LC_ALL=C sort` prints, from both uutils `sort` and GNU's:

| input | GNU coreutils 9.12 | uutils main |
|---|---|---|
| `comm empty sorted`, where `sorted` = `""` then `"\t"` (= `printf '\t\n\n' \| LC_ALL=C sort`) | both lines in column 2, exit 0 | `comm: file 2 is not in sorted order`, exit 1 |
| `comm f1 f2`, where f1 = `a` and f2 = `a<TAB>b` | `a` (column 1), then `a<TAB>b` (column 2) | `a<TAB>b` first, then `a` |

**Versions.** Fails on uutils main 2e28cc3042006f5811e05040232b6154bb905465 and on the 0.12.0 release binary (Linux x86_64, `LC_ALL=C`), compared with GNU coreutils 9.12. By the code it is older still: before `--zero-terminated` (0d782e09c, 0.0.18), lines were read with `BufRead::read_line`, which also keeps the newline, and compared as they are.

**Cause.** `LineReader::read_line` reads with `read_until`, which keeps the terminator in the buffer, and adds one when the last line lacks it. `line_cmp` then compares those buffers as they are, both in `--check-order` and when merging the two files. When one line is a prefix of the next, the comparison reaches the shorter line's terminator. `""` versus `"\t"` becomes `"\n"` (0x0a) versus `"\t\n"` (0x09 0x0a), and 0x0a > 0x09. With `-z` the terminator is NUL, which is never greater, so `-z` is not affected.

**Fix.** Strip the terminator before comparing. `line_cmp` takes the line ending and uses `strip_suffix`, so nothing is allocated and an unterminated slice is left alone. `OrderChecker` keeps the line ending so it can do the same. The patch at the end applies to main with `git am`. It adds `test_line_terminator_not_compared`, which fails without the fix. With the patch, all 50 `comm` tests pass, `cargo fmt` and `clippy -p uu_comm -D warnings` are clean, and the two cases above, `-z`, and an unterminated last line all match GNU 9.12 byte for byte.

Related but different: #12253 and #12912 report the same "not in sorted order" message, caused by locale collation, which #14047 addressed. This bug is in the `LC_ALL=C` byte path and is still present on main.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main. The patch was written with AI assistance, without reference to GNU source, and I have reviewed it.

### Reproduction steps

```sh
export LC_ALL=C
: > empty
printf '\n\t\n' > sorted          # lines "" and "\t", the same as: printf '\t\n\n' | sort
comm empty sorted; echo "exit=$?"
# GNU 9.12: "\t\n\t\t\n", exit=0
# uutils:   comm: file 2 is not in sorted order / comm: input is not in sorted order, exit=1

printf 'a\n' > f1
printf 'a\tb\n' > f2
comm f1 f2 | od -c
# GNU 9.12: a \n \t a \t b \n
# uutils:   \t a \t b \n a \n
```

### Patch

Applies to main (2e28cc30) with `git am`: save the block below as `fix.patch` and run `git am fix.patch`.

<details>
<summary>fix.patch</summary>

```diff
From 8b550961ea7fd2e71f803ae8013dd3c614268575 Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Fri, 16 Oct 2026 21:30:39 +0000
Subject: [PATCH] comm: do not compare the line terminator

LineReader::read_line keeps the line terminator in the buffer (and adds
one to an unterminated last line), and line_cmp compared those buffers
as they are, both for --check-order and for merging the two files. When
one line is a prefix of the next and the next continues with a byte
below the terminator, the comparison reached the terminator: "" sorts
before "\t", yet "\n" is greater than "\t\n". Sorted input such as the
output of LC_ALL=C sort was then reported as not in sorted order, and
lines from the two files were printed in the wrong order.

Strip the terminator before comparing. OrderChecker keeps the line
ending so it can do the same.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
---
 src/uu/comm/src/comm.rs    | 25 +++++++++++++++++++------
 tests/by-util/test_comm.rs | 24 ++++++++++++++++++++++++
 2 files changed, 43 insertions(+), 6 deletions(-)

diff --git a/src/uu/comm/src/comm.rs b/src/uu/comm/src/comm.rs
index bfe5ffd50..cbd81e363 100644
--- a/src/uu/comm/src/comm.rs
+++ b/src/uu/comm/src/comm.rs
@@ -62,6 +62,7 @@ struct OrderChecker {
     mode: CheckOrder,
     has_error: bool,
     use_locale: bool,
+    line_ending: u8,
 }
 
 /// Compare two lines the way the input was ordered.
@@ -69,7 +70,12 @@ struct OrderChecker {
 /// `sort` orders with the locale collation, and so do `join` and `ls`. Reading
 /// that order back with a byte comparison rejects input that is in order and
 /// puts lines in the wrong column, so `comm` has to measure it the same way.
-fn line_cmp(a: &[u8], b: &[u8], use_locale: bool) -> Ordering {
+///
+/// The line terminator is not part of the line and must not take part in the
+/// comparison: `""` sorts before `"\t"`, yet `"\n"` is greater than `"\t\n"`.
+fn line_cmp(a: &[u8], b: &[u8], line_ending: u8, use_locale: bool) -> Ordering {
+    let a = a.strip_suffix(&[line_ending]).unwrap_or(a);
+    let b = b.strip_suffix(&[line_ending]).unwrap_or(b);
     if use_locale {
         locale_cmp(a, b)
     } else {
@@ -121,13 +127,14 @@ impl LineReader {
 }
 
 impl OrderChecker {
-    fn new(file_num: FileNumber, mode: CheckOrder, use_locale: bool) -> Self {
+    fn new(file_num: FileNumber, mode: CheckOrder, use_locale: bool, line_ending: u8) -> Self {
         Self {
             last_line: Vec::new(),
             file_num,
             mode,
             has_error: false,
             use_locale,
+            line_ending,
         }
     }
 
@@ -143,7 +150,12 @@ impl OrderChecker {
             return true;
         }
 
-        let is_ordered = line_cmp(current_line, &self.last_line, self.use_locale) != Ordering::Less;
+        let is_ordered = line_cmp(
+            current_line,
+            &self.last_line,
+            self.line_ending,
+            self.use_locale,
+        ) != Ordering::Less;
         if !is_ordered && !self.has_error {
             let _ = writeln!(
                 stderr(),
@@ -211,8 +223,9 @@ fn comm(
     };
 
     let use_locale = should_use_locale_collation();
-    let mut checker1 = OrderChecker::new(FileNumber::One, mode, use_locale);
-    let mut checker2 = OrderChecker::new(FileNumber::Two, mode, use_locale);
+    let line_ending: u8 = a.line_ending.into();
+    let mut checker1 = OrderChecker::new(FileNumber::One, mode, use_locale, line_ending);
+    let mut checker2 = OrderChecker::new(FileNumber::Two, mode, use_locale, line_ending);
     let mut files_differ = false;
     let mut input_error = false;
 
@@ -220,7 +233,7 @@ fn comm(
         let ord = match (na, nb) {
             (0, _) => Ordering::Greater,
             (_, 0) => Ordering::Less,
-            (_, _) => line_cmp(ra, rb, use_locale),
+            (_, _) => line_cmp(ra, rb, line_ending, use_locale),
         };
 
         if ord != Ordering::Equal {
diff --git a/tests/by-util/test_comm.rs b/tests/by-util/test_comm.rs
index 88a01f211..95deaebd8 100644
--- a/tests/by-util/test_comm.rs
+++ b/tests/by-util/test_comm.rs
@@ -859,3 +859,27 @@ fn test_both_inputs_stdin() {
         .stdout_is("\ta\nb\n\t\tc\n")
         .no_stderr();
 }
+
+#[test]
+fn test_line_terminator_not_compared() {
+    // "" sorts before "\t" and "a" before "a\tb"; the newline that ends each
+    // line must not take part in the comparison.
+    let scene = TestScenario::new(util_name!());
+    let at = &scene.fixtures;
+    at.write("empty", "");
+    at.write("sorted", "\n\t\n");
+    at.write("f1", "a\n");
+    at.write("f2", "a\tb\n");
+    scene
+        .ucmd()
+        .env("LC_ALL", "C")
+        .args(&["--check-order", "empty", "sorted"])
+        .succeeds()
+        .stdout_only("\t\n\t\t\n");
+    scene
+        .ucmd()
+        .env("LC_ALL", "C")
+        .args(&["f1", "f2"])
+        .succeeds()
+        .stdout_only("a\n\ta\tb\n");
+}
-- 
2.55.0

```

</details>
