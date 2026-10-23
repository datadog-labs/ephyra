# Pathname expansion: backslash from a variable is kept in a glob path component that has no glob characters

Posted as https://github.com/reubeno/brush/issues/1583 on 2026-10-23. The text below is the issue as posted.

---

### brush version

brush 0.4.0 (git:b562e058), built from main at b562e0586b366d0558b27019276e591856253947 (2026-10-22).

### bash version (for comparison)

GNU bash, version 5.3.20(1)-release

### Platform / OS

macOS

### Steps to reproduce

Run in an empty directory:

```bash
mkdir sub; touch sub/file
v='s\ub'
echo $v/*
echo $v*/*
```

### Actual behavior (brush)

```bash
echo $v/*
# brush: s\ub/*
echo $v*/*
# brush: sub/file
```

### Expected behavior

```bash
echo $v/*
# bash: sub/file
echo $v*/*
# bash: sub/file
```

The word `$v/*` has an unquoted `*`, so the whole word is a pattern. The bash manual (Pattern Matching) says: "A backslash escapes the following character; the escaping backslash is discarded when matching." So `s\ub` matches the directory `sub`.

brush already does this when the component has a glob character (the second line). It does not do it when the component has none (the first line). It looks for a directory named `s\ub` instead.

Other forms that show the same thing:

```bash
v='sub\'; echo ${v}/*
# bash: sub/file
# brush: sub\/*
```

### Additional context

Found by an automated differential tester (brush vs bash 5.3); this report was drafted with AI assistance.
