# `"${a[@]+x}"` with an empty array gives one empty word instead of no words

Posted as https://github.com/reubeno/brush/issues/1568 on 2026-10-21. The text below is the issue as posted.

---

### brush version

`main` at fc401d784d93aea0e956fff7d9755199a8bf834b (2026-10-20, `brush 0.4.0 (git:fc401d78)`), release build.

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

```sh
a=(); set -- "${a[@]+x}"; echo $#
```

### Actual behavior (brush)

```sh
a=(); set -- "${a[@]+x}"; echo $#
# brush: 1
```

### Expected behavior

```sh
a=(); set -- "${a[@]+x}"; echo $#
# bash 5.3: 0
```

### Additional context

When the alternative is not used, a quoted `[@]` expansion of an empty array should disappear, like `"${a[@]}"`. The same happens with `${a[@]:+x}`, with an unset array, and with `"${@+x}"` / `"${@:+x}"` when there are no positional parameters. This breaks the common idiom `cmd "${opts[@]+"${opts[@]}"}"`, which passes an extra empty argument when `opts` is empty.

A potential local fix (applies to main (0574ed13) with `git am`):

<details>
<summary>fix.patch</summary>

```diff
From 739832aadcbd27fbdae7275fc509380456e6cd23 Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Wed, 21 Oct 2026 09:16:19 -0400
Subject: [PATCH] fix(core): expand "${a[@]+x}" of an empty array to no words
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

For an empty array, `"${a[@]+x}"` expands to no words in bash, as
`"${a[@]}"` does (and likewise `"${@+x}"` with no positional parameters).
The not-taken branch of `UseAlternativeValue` always returned one empty
string, so `a=(); set -- "${a[@]+x}"; echo $#` printed 1 instead of 0.
#1213 / #1217 fixed only the unquoted form.

When the parameter expands to separate fields and there are none, expand
to no words.

Signed-off-by: Alperen Keleş <alpkeles99@gmail.com>
Assisted-by: Claude Code:claude-opus-5-5
---
 brush-core/src/expansion.rs                     |  7 +++++++
 .../cases/compat/word_expansion/params.yaml     | 17 +++++++++++++++++
 2 files changed, 24 insertions(+)

diff --git a/brush-core/src/expansion.rs b/brush-core/src/expansion.rs
index 7dce5d3f..451de7fd 100644
--- a/brush-core/src/expansion.rs
+++ b/brush-core/src/expansion.rs
@@ -1372,6 +1372,13 @@ impl<'a, SE: extensions::ShellExtensions> WordExpander<'a, SE> {
                         brush_parser::word::ParameterTestType::Unset,
                         ParameterState::DefinedEmptyString,
                     ) => Ok(self.expand_parameter_word(alternative_value).await?),
+                    // "${arr[@]+word}" or "${@+word}" with no elements expands to nothing
+                    // (zero words), just as "${arr[@]}" and "$@" do.
+                    _ if !expanded_parameter.concatenate
+                        && expanded_parameter.fields.is_empty() =>
+                    {
+                        Ok(expanded_parameter)
+                    }
                     _ => Ok(Expansion::from(String::new())),
                 }
             }
diff --git a/brush-shell/tests/cases/compat/word_expansion/params.yaml b/brush-shell/tests/cases/compat/word_expansion/params.yaml
index 36636f61..609b8548 100644
--- a/brush-shell/tests/cases/compat/word_expansion/params.yaml
+++ b/brush-shell/tests/cases/compat/word_expansion/params.yaml
@@ -175,6 +175,23 @@ cases:
       echo "single[@]+     : [${single[@]+ALT}]"
       echo "single[@]-     : [${single[@]-DEF}]"
 
+  - name: "Parameter expression: quoted [@]/@ alternative with no elements"
+    stdin: |
+      # When the alternative is not used, these expand to zero words,
+      # just as "${empty[@]}" and "$@" do.
+      empty=()
+      unset unset_arr
+      printf '<%s>' "${empty[@]+ALT}" x; echo
+      printf '<%s>' "${empty[@]:+ALT}" x; echo
+      printf '<%s>' "${unset_arr[@]+ALT}" x; echo
+      set --
+      printf '<%s>' "${@+ALT}" x; echo
+      printf '<%s>' "${@:+ALT}" x; echo
+
+      # Unchanged: [*] still yields one empty word; inside a word nothing is added.
+      printf '<%s>' "${empty[*]+ALT}" x; echo
+      printf '<%s>' a"${empty[@]+ALT}"b; echo
+
   - name: "Parameter expression: index-listing combined with conditional operator"
     # brush currently fails to parse the ${!arr[@]} index-listing form when it is
     # combined with a conditional operator (+/-/:+/:-). bash accepts it; for an
```

</details>
