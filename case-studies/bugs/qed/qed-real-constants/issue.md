# Issue for qed-solver/prover: filed as #13

Filed 2026-10-14 04:48 UTC by alpaylan: https://github.com/qed-solver/prover/issues/13

Work tag `qed/bug/real-constants`. Reproduced on upstream main 9e9c262 with qed-parser 684daf2
(both Nix builds); raw outputs in `verify/`, patch in `fix.patch` (`git apply --check` clean on
9e9c262). Found by ephyra-sqleq (one-step run 0-100). Duplicate search: prover issues/PRs #1-#12,
GitHub search (real, float, decimal, precision, f32, NaN, infinity, panic, normal.rs, from_float,
to_i32, ratio, constant), parser issues #21-#40: nothing related; the lines are unchanged since
2022 (2337fb6).

---

**Title:** Unsound: real constants are parsed as `f32`, so distinct literals are proved equal (and wide ones panic)

**Body:**

QED parses every real constant as an `f32` and then squeezes it into an `i32/i32` fraction. This has two effects:
- Different SQL literals that round to the same `f32` become the same Z3 value, so non-equivalent queries are proved equivalent.
- Literals outside the `f32` or `i32/i32` range panic.

## Reproduction

Each case goes through the shipped pipeline (`qed-parser repro.sql && qed-prover repro.json`); no IR is edited by hand. Every file starts with:

```sql
CREATE TABLE t (c0 DOUBLE, c1 DECIMAL(20,10), c2 INTEGER);
```

**A. Unsound (`.result` is `"provable":true`)**

| | query 1 | query 2 |
|---|---|---|
| A1 | `SELECT c2 FROM t WHERE c0 = 16777217.0;` | `SELECT c2 FROM t WHERE c0 = 16777216.0;` |
| A2 | `SELECT c2 FROM t WHERE c1 = 16777217.0;` | `SELECT c2 FROM t WHERE c1 = 16777216.0;` |
| A3 | `SELECT c2 FROM t WHERE c0 = 0.1;` | `SELECT c2 FROM t WHERE c0 = 0.100000001;` |

The parser passes the literal through unchanged, e.g. `{"type":"DOUBLE","operand":[],"operator":"16777217.0"}`.

```
$ qed-parser A1.sql && qed-prover A1.json   # same for A2, A3
A1.json  Provable(...)        # .result: {"provable":true,"panicked":false,...}
```

Expected: not provable. On PostgreSQL, put the rows `(16777217.0, 16777217.0, 1)` and `(0.1, 0.1, 1)` into `t (c0 float8, c1 numeric, c2 int4)`. Query 1 then returns 1 row and query 2 returns 0, for each of A1, A2 and A3.

As a control, the integer version (`c2 = 16777217` vs `c2 = 16777216`) is correctly not provable.

**B. Panics.** Each pair is `... WHERE c0 > X AND c2 = 1` vs `... WHERE c2 = 1 AND c0 > X`, which is equivalent:

| `X` | parser emits | result |
|---|---|---|
| `1e39` | `"1E+39"` (DOUBLE) | `panicked at src/pipeline/normal.rs:655:65` |
| `10000000000.0` | `"10000000000.0"` (DECIMAL) | `panicked at src/pipeline/normal.rs:658:44` |
| `8.5e26` | `"8.5E+26"` (DOUBLE) | `panicked at src/pipeline/normal.rs:658:44` |
| `9.7e-28` | `"9.7E-28"` (DOUBLE) | `panicked at src/pipeline/normal.rs:659:44` |

All four are ordinary `float8` values (PostgreSQL: `1e+39|8.5e+26|9.7e-28|10000000000`). Other frontends that emit `NaN` or `Infinity` as a Real constant, or as a `CAST` of one, hit line 655 too.

## Root cause

`src/pipeline/normal.rs` at 9e9c262, the constant parser inside `impl Eval<&Expr, Dynamic> for &Z3Env`:

```rust
653				Real => {
654					let r: f32 = input.parse()?;
655					let r = num::rational::Ratio::from_float(r).unwrap();
656					ctx.real_some(Re::from_real(
657						z3_ctx,
658						r.numer().to_i32().unwrap(),
659						r.denom().to_i32().unwrap(),
660					))
661				},
```

It is reached for every nullary `Op` of type Real (`FLOAT`, `DOUBLE` and `DECIMAL` all alias to `Real` in `shared.rs:323`), and for `CAST(<constant>)` to Real.

- **Line 654:** `f32` has a 24-bit significand. `16777217` becomes `16777216`, and `0.100000001` becomes the same float as `0.1`. Values above about 3.4e38 become `inf`, and values below about 1e-45 become `0`.
- **Line 655:** `from_float` returns `None` for `inf` and `NaN`.
- **Lines 658-659:** the exact fraction of the float does not fit in `i32`. Large values overflow the numerator; small ones (`9.7e-28` is about `k / 2^112`) overflow the denominator.

## Suggested fix

Parse the decimal string exactly into a `BigRational` and build the Z3 numeral from its decimal numerator and denominator strings (`Real::from_real_str`, which is already in the z3 binding). Anything that is not a decimal literal, such as `NaN` or `Infinity`, which have no real counterpart, returns `Err`. Through the existing `Ok(exp) = parse(..)` guard, that keeps the constant uninterpreted instead of panicking.

```diff
 				Real => {
-					let r: f32 = input.parse()?;
-					let r = num::rational::Ratio::from_float(r).unwrap();
-					ctx.real_some(Re::from_real(
-						z3_ctx,
-						r.numer().to_i32().unwrap(),
-						r.denom().to_i32().unwrap(),
-					))
+					let r = parse_decimal(input)?;
+					let (num, den) = (r.numer().to_string(), r.denom().to_string());
+					let r = Re::from_real_str(z3_ctx, &num, &den);
+					ctx.real_some(r.ok_or_else(|| anyhow::anyhow!("invalid real constant {}", input))?)
 				},
```

<details><summary><code>parse_decimal</code> (new, about 30 lines)</summary>

```rust
/// Parse a decimal literal such as `-12.50`, `1E+39` or `9.7e-28` into the exact rational it
/// denotes. Anything else (e.g. `NaN` or `Infinity`, which have no counterpart among the reals)
/// is rejected with an error, so the constant is kept uninterpreted instead of approximated.
fn parse_decimal(input: &str) -> anyhow::Result<num::BigRational> {
	use num::{BigInt, BigRational};
	let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
	let (mantissa, exp) = match input.find(['e', 'E']) {
		Some(i) => (&input[..i], input[i + 1..].parse::<i64>()?),
		None => (input, 0),
	};
	let (neg, mantissa) = match mantissa.strip_prefix('-') {
		Some(m) => (true, m),
		None => (false, mantissa.strip_prefix('+').unwrap_or(mantissa)),
	};
	let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
	if (int.is_empty() && frac.is_empty()) || !digits(int) || !digits(frac) {
		bail!("not a decimal literal: {}", input);
	}
	// Bound the exponent so a hostile literal cannot make us build an enormous numeral.
	let exp = exp - frac.len() as i64;
	if exp.abs() > 100_000 {
		bail!("decimal exponent out of range: {}", input);
	}
	let mut num: BigInt = format!("{}{}", int, frac).parse()?;
	if neg {
		num = -num;
	}
	let scale = num::pow(BigInt::from(10), exp.unsigned_abs() as usize);
	Ok(if exp >= 0 {
		BigRational::from_integer(num * scale)
	} else {
		BigRational::new(num, scale)
	})
}
```

</details>

With the patch applied to 9e9c262:
- A1, A2 and A3 become **not provable**.
- All four B pairs become **provable**, with no panic.
- These controls stay as before:
  - Provable: `c0 = 16777216.0` vs `16777216.0 = c0`, `c0 > 0.5 AND c2 = 1` vs swapped, `c0 > -0.5 AND c2 = 1` vs swapped, `c0 = 0.5` vs `c0 = 0.50`, `c0 = 2.5e-1` vs `c0 = 0.25`, `c0 > -0.5` vs `c0 > -0.50`.
  - Not provable: `c0 > -0.5` vs `c0 > 0.5`.

### Effect on `tests/`

Over all 1731 cases, exactly one verdict changes: `cockroach/memo/431` goes from provable to **not provable**.

- One side is `f > 0.0`. The other is `f > 0.0` plus an index constraint `f >= 5e-324`.
- The two are equivalent only for IEEE doubles, where `5e-324` is the smallest positive subnormal.
- Over the reals they differ for `0 < f < 5e-324`.
- It was proved only because `"5e-324".parse::<f32>()` underflows to `0.0`.

Totals are 1235 vs 1234 provable. No other case changes verdict, panic status or timeout status.

Happy to open a PR with the patch.

## Environment

`qed-prover` at `9e9c262` and `qed-parser` at `684daf2`, both built with Nix. Found by differential testing; the expected results were checked on PostgreSQL.

---

## Verification (not part of the issue)

| Claim | Command | Output |
|---|---|---|
| A1/A2/A3 proved on main | `run_cases.sh <main> verify/sql` | `Provable`, `"provable":true` (verify/main-9e9c262.txt: u1_double, u2_decimal, u3_frac) |
| Integer control | same | `ctl_int NotProvable` |
| B panics | same | `normal.rs:655:65`, `658:44` (x2), `659:44` |
| PostgreSQL evidence | `psql -f verify/pg_check.sql` | `1/0`, `1/0`, `1/0` (verify/pg_check.out) |
| Fix | `run_cases.sh <fixed> verify/sql` | A: NotProvable; B and controls: Provable (verify/fixed.txt) |
| sqleq IR, incl. `'NaN'` | `run_cases.sh` on `verify/sqleq-ir` | main: NaN panics at `655:65`; fixed: Provable |
| tests/ | full dir on both builds | one diff: `cockroach/memo/431` (verify/tests-verdicts.diff) |
| Patch applies to 9e9c262 | `git apply --check fix.patch` | clean |

Open notes: an unused `ToPrimitive` import warning from the patch was not checked; NaN/Infinity
become uninterpreted constants (no model of PostgreSQL's NaN ordering, as before).
