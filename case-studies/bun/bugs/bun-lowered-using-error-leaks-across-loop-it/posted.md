# Posted: https://github.com/oven-sh/bun/issues/48361

Filed by alpaylan through the GitHub API, 2026-11-09, at the owner's request. Text as posted:

---

## `bun build` lowering of `using` in a loop rethrows an earlier iteration's error

### What version of Bun is running?

1.4.4+38afee6c8 (also main 2596ac5d3 and 1.3.14+0d9b296af)

### What platform is your computer?

Linux 7.0.0-1014-aws x86_64 x86_64

### What steps can reproduce the bug?

`using-loop.js`:

```js
for (let i = 0; i < 2; i++) {
  try {
    using r = null;
    if (i == 0) throw new Error("boom");
  } catch (e) {
    console.log("caught", i, e.message);
  }
}
```

```sh
bun build using-loop.js --outfile=out.js && bun out.js
```

### What is the expected behavior?

```
caught 0 boom
```

Iteration 1 throws nothing. `bun using-loop.js` and Node.js 24.22.2 print this.

### What do you see instead?

```
caught 0 boom
caught 1 boom
```

### Additional information

The lowered catch is `var _err = _catch, _hasErr = 1;`, so both values carry into the next iteration. `--target=node` and `Bun.Transpiler` lower `using` the same way; `--target=bun` keeps `using` and prints the expected output.

