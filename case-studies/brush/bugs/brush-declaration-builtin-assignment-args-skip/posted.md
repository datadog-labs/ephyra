# Brace expansion is not applied to assignment arguments of `declare`/`local`/`export`

Posted as https://github.com/reubeno/brush/issues/1569 on 2026-10-21. The text below is the issue as posted.

---

### brush version

`main` at fc401d784d93aea0e956fff7d9755199a8bf834b (2026-10-20, `brush 0.4.0 (git:fc401d78)`), release build.

### bash version (for comparison)

GNU bash 5.3.20(1)-release

### Platform / OS

Linux

### Steps to reproduce

```sh
declare v={a,b}; declare -p v
```

### Actual behavior (brush)

```sh
declare v={a,b}; declare -p v
# brush (main):  declare -- v="{a,b}"
# brush (0.4.0): declare -- v="a b"
```

### Expected behavior

```sh
declare v={a,b}; declare -p v
# bash 5.3: declare -- v="b"
```

bash brace-expands the word to two arguments, `v=a v=b`, so `v` ends up as `b`. The same happens with `local`, `export`, `readonly` and `typeset`, and with subscripts: `declare a[{1,2}]=x` sets both `a[1]` and `a[2]` in bash. On main, a plain assignment without a builtin, `v={a,b}`, stores the literal text `{a,b}`, as bash does.

### Additional context

`basic_expand_assignment_word` (`brush-core/src/expansion.rs`) turns off brace expansion for every assignment-shaped word, including arguments to declaration builtins. Open PR #1280 reworks this code but does not seem to cover this case.

Potential fix (applies to main (0574ed13) with `git am`):

<details>
<summary>fix.patch</summary>

```diff
From e59ea4deeb94bc5e53212dc9b78bd1795546a4a7 Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Wed, 21 Oct 2026 09:16:14 -0400
Subject: [PATCH] fix(core): brace-expand assignment arguments of declaration
 builtins
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

bash applies brace expansion to an assignment-shaped argument of `declare`,
`local`, `export`, `readonly` and `typeset`: `declare v={a,b}` declares
`v=a` and `v=b`, leaving `v` as `b`. brush turned brace expansion off for
every assignment-shaped word, so it stored the literal `{a,b}`.

Keep brace expansion for arguments of declaration builtins; plain
assignment words still skip it.

Signed-off-by: Alperen Keleş <alpkeles99@gmail.com>
Assisted-by: Claude Code:claude-opus-5-5
---
 brush-core/src/expansion.rs                   | 75 ++++++++++++-------
 brush-core/src/interp.rs                      | 60 ++++++++++++++-
 .../tests/cases/compat/builtins/declare.yaml  | 12 +++
 3 files changed, 120 insertions(+), 27 deletions(-)

diff --git a/brush-core/src/expansion.rs b/brush-core/src/expansion.rs
index 7dce5d3f..0268798e 100644
--- a/brush-core/src/expansion.rs
+++ b/brush-core/src/expansion.rs
@@ -626,6 +626,53 @@ pub(crate) async fn basic_expand_assignment_word(
     expander.basic_expand_to_str(word_str.as_ref()).await
 }
 
+/// Returns whether brace expansion would change the given word.
+///
+/// # Arguments
+///
+/// * `shell` - The shell in which expansion would be performed.
+/// * `word_str` - The word to check, as a string.
+pub(crate) fn brace_expansion_changes_word(
+    shell: &Shell<impl extensions::ShellExtensions>,
+    word_str: impl AsRef<str>,
+) -> bool {
+    let word = word_str.as_ref();
+    shell.options().perform_brace_expansion
+        && brace_expand(word, &shell.parser_options())
+            .is_some_and(|words| words.as_slice() != [word])
+}
+
+/// Performs brace expansion on the word, yielding the resulting words, or `None` if
+/// the word contains no brace expression.
+fn brace_expand(word: &str, parser_options: &brush_parser::ParserOptions) -> Option<Vec<String>> {
+    // We perform a non-authoritative check to see if the string *may* contain braces
+    // to expand. There may be false positives, but must be no false negatives.
+    if !may_contain_braces_to_expand(word) {
+        return None;
+    }
+
+    let brace_expansion_pieces =
+        match brush_parser::word::parse_brace_expansions(word, parser_options) {
+            Ok(Some(pieces)) => pieces,
+            Ok(None) => return None,
+            Err(err) => {
+                tracing::error!("failed to parse for brace expansion: {err:?}");
+                return None;
+            }
+        };
+
+    tracing::debug!(target: trace_categories::EXPANSION, "Brace expansion pieces: {brace_expansion_pieces:?}");
+
+    // An alternative that expands to nothing yields no word at all (like any other
+    // unquoted empty expansion); `{a,}` is just `a`.
+    let words = braceexpansion::generate_and_combine_brace_expansions(brace_expansion_pieces)
+        .into_iter()
+        .filter(|s| !s.is_empty())
+        .collect();
+
+    Some(words)
+}
+
 /// Assigns a value to a named parameter.
 ///
 /// # Arguments
@@ -922,35 +969,11 @@ impl<'a, SE: extensions::ShellExtensions> WordExpander<'a, SE> {
     /// Performs brace expansion on the word, yielding the resulting words, or `None` if
     /// the word contains no brace expression.
     fn brace_expand_if_needed(&self, word: &str) -> Option<Vec<String>> {
-        // We perform a non-authoritative check to see if the string *may* contain braces
-        // to expand. There may be false positives, but must be no false negatives.
-        if self.disable_brace_expansion
-            || !self.shell.options().perform_brace_expansion
-            || !may_contain_braces_to_expand(word)
-        {
+        if self.disable_brace_expansion || !self.shell.options().perform_brace_expansion {
             return None;
         }
 
-        let brace_expansion_pieces =
-            match brush_parser::word::parse_brace_expansions(word, &self.parser_options) {
-                Ok(Some(pieces)) => pieces,
-                Ok(None) => return None,
-                Err(err) => {
-                    tracing::error!("failed to parse for brace expansion: {err:?}");
-                    return None;
-                }
-            };
-
-        tracing::debug!(target: trace_categories::EXPANSION, "Brace expansion pieces: {brace_expansion_pieces:?}");
-
-        // An alternative that expands to nothing yields no word at all (like any other
-        // unquoted empty expansion); `{a,}` is just `a`.
-        let words = braceexpansion::generate_and_combine_brace_expansions(brace_expansion_pieces)
-            .into_iter()
-            .filter(|s| !s.is_empty())
-            .collect();
-
-        Some(words)
+        brace_expand(word, &self.parser_options)
     }
 
     /// Apply tilde-expansion, parameter expansion, command substitution, and arithmetic expansion;
diff --git a/brush-core/src/interp.rs b/brush-core/src/interp.rs
index 22ab6d3c..3c969342 100644
--- a/brush-core/src/interp.rs
+++ b/brush-core/src/interp.rs
@@ -1227,7 +1227,24 @@ impl<SE: extensions::ShellExtensions> ExecuteInPipeline<SE> for ast::SimpleComma
                         // scoped assignment. Add it to the list we're accumulating.
                         assignments.push(assignment);
                     } else {
-                        if command_takes_assignments {
+                        if command_takes_assignments
+                            && matches!(assignment.value, ast::AssignmentValue::Scalar(_))
+                            && expansion::brace_expansion_changes_word(&context.shell, word)
+                        {
+                            // Like bash, brace expansion turns this into plain words: each is
+                            // expanded like a regular argument, and only then handed to the
+                            // builtin as an assignment if it still looks like one.
+                            let mut next_args = expansion::full_expand_and_split_word(
+                                &mut context.shell,
+                                &params,
+                                word,
+                            )
+                            .await?
+                            .into_iter()
+                            .map(|arg| expanded_declaration_arg(arg, assignment))
+                            .collect();
+                            args.append(&mut next_args);
+                        } else if command_takes_assignments {
                             // This looks like an assignment, and the command being invoked is a
                             // well-known builtin that takes arguments that need to function like
                             // assignments (but which are processed by the builtin).
@@ -1476,6 +1493,47 @@ async fn expand_words(
     Ok(fields)
 }
 
+/// Interprets an already-expanded argument to a declaration builtin, treating it as an
+/// assignment (located at `template`'s location) if it has the form `name=value`,
+/// `name+=value`, or `name[index]=value`.
+fn expanded_declaration_arg(arg: String, template: &ast::Assignment) -> CommandArg {
+    // Find the first '=' that is not inside a subscript.
+    let mut depth = 0usize;
+    let equals = arg.char_indices().find_map(|(i, c)| {
+        match c {
+            '[' => depth += 1,
+            ']' => depth = depth.saturating_sub(1),
+            '=' if depth == 0 => return Some(i),
+            _ => (),
+        }
+        None
+    });
+
+    if let Some((lhs, value)) = equals.and_then(|i| Some((arg.get(..i)?, arg.get(i + 1..)?))) {
+        let (lhs, append) = lhs
+            .strip_suffix('+')
+            .map_or((lhs, false), |lhs| (lhs, true));
+        let (var_name, name) = match lhs.strip_suffix(']').and_then(|l| l.split_once('[')) {
+            Some((var_name, index)) => (
+                var_name,
+                ast::AssignmentName::ArrayElementName(var_name.to_owned(), index.to_owned()),
+            ),
+            None => (lhs, ast::AssignmentName::VariableName(lhs.to_owned())),
+        };
+
+        if valid_variable_name(var_name) {
+            return CommandArg::Assignment(ast::Assignment {
+                name,
+                value: ast::AssignmentValue::Scalar(ast::Word::from(value.to_owned())),
+                append,
+                loc: template.loc.clone(),
+            });
+        }
+    }
+
+    CommandArg::String(arg)
+}
+
 async fn expand_assignment(
     shell: &mut Shell<impl extensions::ShellExtensions>,
     params: &ExecutionParameters,
diff --git a/brush-shell/tests/cases/compat/builtins/declare.yaml b/brush-shell/tests/cases/compat/builtins/declare.yaml
index e0d484ba..da4b62c8 100644
--- a/brush-shell/tests/cases/compat/builtins/declare.yaml
+++ b/brush-shell/tests/cases/compat/builtins/declare.yaml
@@ -1293,3 +1293,15 @@ cases:
       declare -p c
       f() { local x=1 -r 2>/dev/null; echo "local: $?"; x=2; declare -p x; }
       f
+
+  - name: "Brace expansion of a declaration's assignment word yields regular words"
+    stdin: |
+      declare v={c,d}; declare -p v
+      declare v+={e,f}; declare -p v
+      declare a[{1,2}]=x; declare -p a
+      x="a 1-"
+      f() { local w={c,d}$x 2>/dev/null; declare -p w; }
+      f
+      declare q={c,d}"$x"; declare -p q
+      declare r=\{c,d}; declare -p r
+      declare -a arr=({1,2}); declare -p arr
```

</details>
