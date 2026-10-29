# tyz: generated typed Python against ty

tyz tests [ty](https://github.com/astral-sh/ty) (Astral's type checker, in the ruff repo) with Python
programs that are typed by construction and run under CPython. Each program's runtime behaviour is the
reference ty is checked against. ty's own fuzzer (`ruff/python/py-fuzzer`) only checks for crashes on
random syntax, and its quickcheck property tests (`ty_python_semantic/src/types/property_tests.rs`) cover
the type algebra. tyz aims at inference and narrowing on whole programs.

## Programs (`src/pygen.rs`)

A program has enums (`Enum`, `IntEnum`, `StrEnum`), protocols (some `@runtime_checkable`), TypedDicts,
classes (plain, with inheritance and covariant overrides; `@dataclass` with `frozen`/`slots`/`kw_only`;
`NamedTuple`; PEP 695 generic classes), functions (some PEP 695 generic, bounded, `TypeIs`/`TypeGuard`
guards), and `main()`, which calls every function one to three times with generated arguments. Union
parameters therefore see several runtime types. Function bodies use narrowing (`is None`, `isinstance`,
`type(x) is`, enum identity, truthiness, walrus, tuple elements, TypedDict keys, user guards), `match`
(class, value, sequence, mapping and capture patterns), loops whose accumulator the body widens and
narrows (ty's fixpoint iteration), `try`/`except`/`else`/`finally`, `with`, closures, comprehensions and
unpacking.

The model (`Ty`, `Model::sub`) is coarser than the typing spec. A judgement in the model must hold in
ty, never the other way round. Each place where ty's semantics forced the model to be more careful has a
comment: `float` is `int | float`; non-final classes may have `Sequence`/`Mapping` subclasses; value
patterns use `__eq__`; unannotated bindings take the inferred type; and so on.

### Pydantic models

Some classes are pydantic `BaseModel`s (`ClassKind::Pydantic`, settings in `Pyd`), aimed at ty's
dedicated support (`types/dedicated/pydantic.rs`): subclasses of other models (which may thaw a frozen
model, never freeze a mutable one), config as `model_config = ConfigDict(...)`, as a dict literal or
as class keywords (`frozen`, `extra`, `validate_by_name`/`validate_by_alias`/`populate_by_name`,
`strict`, `arbitrary_types_allowed` for plain-class fields), fields with `Field(alias=, frozen=,
strict=, default=/default_factory=)` or the same inside `Annotated[T, Field(...), Strict()]` (a default
that names a class not defined yet becomes `default_factory=lambda: ...`), `ClassVar`s, private
attributes (`_pv<k>: T = v`, assigned through instances even when frozen), `BaseSettings` roots
(`SettingsConfigDict`), PEP 695 / `Generic[T]` models (never specialized at the call: unspecialized,
pydantic validates a type variable's field as `Any`, so it gets no lax values), and
`@computed_field` properties. Constructor calls are keyword-only, name an aliased field by its alias or its name as the
config allows, leave out fields with defaults, and in lax mode sometimes pass a value pydantic's
conversion table turns into the field's type (`"7"`, `b"7"`, `7.0` or an int enum member for `int`;
bytes or an enum member for `str`; tuples for lists, ...): ty widens those parameters to
`LaxInt` and the like, and probes see the converted value. Field types are never protocols (pydantic
validates with `isinstance`); per-field strictness only on scalar fields (pydantic raises otherwise).
build.sh installs pydantic into `<build>/venv` (public PyPI; `TYZ_PIP_INDEX` overrides), `bin/py` runs
that venv's python, and tyz sets `VIRTUAL_ENV` to it so every ty and tyinc run resolves `pydantic`.
Programs import pydantic only when they have a model.

The output is a tree of lines (`src/tree.rs`). The shrinker drops runs of siblings, drops nodes,
hoists blocks and empties blocks, without knowing Python. A candidate that fails differently is
rejected.

## Oracles (`src/main.rs`)

`eval_batch` runs a batch of programs under CPython 3.13 (`py/runner.py`, each in a forked child, with
a 3 s limit). `_probe(k, e)` (`py/_rt.py`) records the runtime type of `e` as a type expression:
literals for ints, strs, bytes and enum members; `TypeOf[0.5]` for floats; tuples element-wise; class
instances as `Intersection[C, Not[X], ...]` over every program and builtin class `X` the value is not an
instance of, plus `Not[AlwaysFalsy]` or `Not[AlwaysTruthy]`; and `?` (no assertion) for dicts, which
could be TypedDicts. Programs that raise are `Invalid` (generator misses, counted). For the others, a
check file gets, after each observed probe (union of the observed types `U`):

```python
static_assert(not is_disjoint_from(U, TypeOf[e]))   # sound__disjoint: ty rules out the value itself
static_assert(is_assignable_to(U, TypeOf[e]))       # sound__assign: weaker; fails when ty cannot prove a coarse U fits
```

ty checks all the check files of a batch in one `ty check --project .` run. Verdicts, first match wins:
`crash__panic__<file>_<line>` (a `fatal[panic]` diagnostic), `crash__exit__*` / `crash__hang__ty` (the
whole batch), `fp__<rule>` (an error on a program line: the program ran, and the model says it is
well-typed), `harness__assert__<rule>` (an error on an inserted line other than a failed assertion),
`sound__disjoint`, `sound__assign`, `truth__<rule>__<construct>`.

Truthiness (`truth__`): `py/runner.py` runs each program with every condition (`if`/`elif`/`while`/
`assert` test, conditional expression, comprehension `if`, match guard, and the operands of `and`/`or`/
`not` inside them) wrapped in a recorder of its truth value, keyed by its source range. The check
directory's `ty.toml` turns on `redundant-condition-strict`. A `redundant-condition(-strict)` warning
("always truthy/true" or "always falsy/false") on a condition CPython saw take the other truth value
is a wrong result; `<construct>` is `if`, `while`, `ifexp`, `assert`, `comp`, `guard`, with
`-boolop`/`-not` for operands. When several conditions start at the warning's position (`a and b`
and `a`), each of them that ran must contradict the claim.

### Incremental consistency (`src/incr.rs`, `tyinc/`)

`bin/tyinc` is built in-process against the checkout's `ty_project` (its own workspace with ruff's
`Cargo.lock`, same toolchain and flags as `bin/ty`). For the programs of a batch that pass the oracles
above, step 0 is their check files with a `reveal_type` after every probe plus, per program, `u_<i>.py`,
which imports every top-level name of `c_<i>.py` (`from c_i import ...` and `c_i.<name>`) and reveals it.
Steps 1 to 3 edit each program again, one to three edits each (a statement or definition becomes
`pass`, a builtin type or a literal changes, a top-level definition is renamed, a return or variable
annotation goes, a line is cut short as if being typed, a line is inserted at the top); step 2 also makes
`u_<i>` reveal every name through `m_<i>`, which step 3 creates (`from c_<i> import *`). tyinc checks
step 0 in one database (LRU on, inputs mutable, as in watch mode and the server), syncs the files to
each next step (unique mtimes: ty keys file revisions on them), applies the change events and re-checks,
checks the step in a fresh database too (configured like one-shot `ty check`), and at the end reverts to
step 0 and re-checks.

- `incr__edit__<rule>[-import]`: an incremental step differs from the fresh check of that step (`<rule>` of the first
  differing diagnostic; `-import` when it is in `u_<i>.py`).
- `incr__revert__<rule>[-import]`: after reverting, the diagnostics differ from step 0.
- `crash__panic__*` from a panic in any of the checks, in an IDE request, or one that kills tyinc;
  `crash__exit__tyinc_*`, `crash__hang__tyinc` when tyinc dies otherwise.

After the first and the last edit, tyinc also runs IDE requests (hover, goto definition and type
definition, highlights, signature help, references, completions, inlay hints, semantic tokens, symbols,
hints) on a sample of the files, at identifier offsets and right after `.`/`(`/`[`, as a language
server would between keystrokes (`TYINC_IDE=0` turns them off). Edits include cutting a line short
with an unfinished token, so these files are often syntactically broken on purpose.

The finding keeps `incr/s0` .. `incr/s3` (the program alone when that reproduces, otherwise the whole
batch with `incr/only.txt` naming the files compared); `repro.sh` runs tyinc on them up to three times.
`tyz minimize` does not apply (the edit depends on the program text): shrink the step files by hand, or
`tyz incr-edit <file.py>` to see an edit. `TYZ_INCR=0` turns the oracle off.

### Rename round trips (`src/ren.rs`, `tyinc --rename`)

For the programs of a batch that pass the oracles above (TYZ_REN_N, 16, in one batch of TYZ_REN_EVERY, 2), a few of the
program's own names (TYZ_REN_K, 3 each: classes, functions, methods, attributes and fields, parameters, locals) are renamed
the way an editor does on F2: `tyinc --rename` opens the programs in one database and calls `ty_ide::can_rename` and
`ty_ide::rename` at a random occurrence of the name. The new name is as long as the old one (programs compute
`len(C.__name__)`) and occurs nowhere in the program. Names that also occur in a string literal (`hasattr`, TypedDict keys,
functional enums, `__all__`), names declared in more than one place (overrides, protocol members and their
implementations) and module names (rename.rs: "TODO(submodule-imports)") are not picked. Then:

- `ren__edit__<kind>` / `ren__crossfile__<kind>`: an edit does not replace the old name, or edits a file the program does
  not import (the batch's programs share one project but never import each other);
- `ren__self__<kind>`: no edit covers the occurrence the rename started from;
- `ren__asym__<kind>`: renaming again from another occurrence the first rename edited gives other edits;
- `ren__runtime__<kind>`: CPython raises on the renamed program, or its probe values differ (old name put back, string
  contents blanked: `__name__`, `.name` and reprs change with the name);
- `ren__check-<rule>__<kind>`: ty reports an error on it (except `invalid-method-override`/`invalid-overload` after
  renaming a parameter: what the user asked for).

`<kind>` is the declaration's kind (`class`, `func`, `method`, `attr`, `param`, `var`); `-x` when the program was split
over modules first, `-q` when its annotations were quoted first (`py/meta.py`'s `q` variant, TYZ_REN_Q, 2 programs per
batch: ty renames inside string annotations): TYZ_REN_M (2) programs per batch go through `py/meta.py`'s `m` variant (not its mutual-`*` shape,
`meta__module-tailstar__star-cycle`) and are renamed across modules (definitions and imports in `md_<i>`/`mp_<i>/`, uses
in `m_<i>`); that program is run through `e_<i>.py` (`import m_<i>`) and compared with the unrenamed split. A finding keeps
`ren/in/` (or `ren/unit/` and `ren/entry.txt`) and `ren/req.json`; `tyz ren-reqs <file.py>` lists the requests for a
program. `TYZ_REN=0` turns the oracle off.

### Inlay hint edits (`src/hint.rs`, `tyinc --hints`)

For the first TYZ_HINT_N (16) programs of every batch that pass the oracles above, `tyinc --hints` asks
`ty_ide::inlay_hints` (default settings, whole file) for each program's hints, as the language server does,
and applies the edits an editor inserts when the user double-clicks a hint, one variant per kind:

- `argname`: the `name=` edit ty offers on each call's last non-starred positional argument;
- `vartype`: the `: T` annotation edits (and the imports they add) on assignment targets bound only once in
  the program (a second binding of another type would fail the inserted declaration, which is what the
  user asked for); `vartype-class` for targets in a class body, `vartype-attr` for `self.<attr>` targets.

CPython must run the variant cleanly with the same probe values (`hint__runtime__<kind>`; a
`typing_extensions.py` shim re-exporting `typing` sits next to it, since ty imports `LiteralString` and
others from typing_extensions, which is not in the stdlib) and ty must report no error on it
(`hint__check-<rule>__<kind>`). `argname` and `vartype` leave out four shapes ty's edits are known to break;
1 in TYZ_HINT_BAD_EVERY (250) programs gets one of them in a variant of its own: `argname-parens` (a
parenthesized argument: the edit goes inside the parentheses, a SyntaxError), `argname-varargs` (positional
arguments follow it into `*args`: "positional argument follows keyword argument"), `argname-typealiastype`
(ty rejects the `TypeAliasType("A", value=...)` it creates), and `vartype-attrobj` (an annotation on another
object's attribute, `r.a: T = ...`, which ty then rejects as invalid-type-form), and `vartype-classvar`
(`model_config: ConfigDict = {...}` in a pydantic model: an instance variable overriding `BaseModel`'s class variable). A finding keeps `hint/in/<file>` and
`hint/obs.json`. `TYZ_HINT=0` turns the oracle off.

### Language server sessions (`src/lsp.rs`, `py/lsp.py`)

For the check files of the first TYZ_LSP_N (6) programs that pass the oracles above, in one search batch of
TYZ_LSP_EVERY (2), `py/lsp.py` runs one `ty server` session over stdio, as an editor does (pull diagnostics,
notebook sync, a position encoding negotiated per session: UTF-16, UTF-8 or UTF-32; diagnosticMode
`workspace` in half the sessions). F is the check file with non-ASCII comments (`# é😀日本`) appended to some
lines, CRLF line ends in 1 program of 5; N is F split into 2 to 8 notebook cells at top-level statements (some
without a trailing newline), plus in half the notebooks one or two markdown, IPython magic or shell-escape
cells. F (`l_<k>.py`) and N (`n_<k>.ipynb`) are on disk. Each is opened broken (1 to 3 edits: lines deleted,
cut short, junk lines, a builtin type, identifier, literal or operator changed; the notebook also misses a
cell, has an extra one or two cells swapped) and restored with the inverse edits as incremental `didChange`s
(some typed in chunks with hover/completion/signature help in between, also past line ends; some whole-text
changes), the cell structure by a notebook structure change. Every program has its own random choices (seed
and text), so a program shows the same session alone.

- `lsp__cli-<rule>__<text|nb>`: `ty check` reports an error on F or on N (CLI notebook support vs the `.py`);
- `lsp__diff-<rule>__<text|nb>`: just after opening the broken documents and at one state in the middle of
  the edits, the pulled diagnostics (line, column in code points, severity, rule, first line of the message;
  `previousResultId` reuse) differ from `ty check` of the same texts;
- `lsp__final-<rule>__<text|nb>`: the restored documents' pulled diagnostics have an error;
- workspace mode only: `lsp__closed-<rule>__text` (after `didClose`, F from disk has an error),
  `lsp__diff-<rule>__disk` / `lsp__closed-<rule>__disk` (F on disk replaced by the broken text and back, with
  `didChangeWatchedFiles`: diagnostics must be `ty check`'s, then none), `lsp__wsdiag-<rule>__text`
  (`workspace/diagnostic` reports an error);
- `lsp__xpos-<request>__nb`: hover, definition, type definition, declaration, references, inlay hints (label
  locations mapped), folding ranges (not import/comment blocks) or semantic tokens in one of N's cells differ
  from the same request at the same position or lines of F;
- `crash__panic__*` (stderr or an error response), `crash__exit__tyserver`, `crash__hang__tyserver`.

Known bugs on cells, each compared in 1 session of 40 only, on its first program (TYZ_LSP_FORCE=<request> compares that one always):
`documentHighlight` and `documentSymbol` return every cell's results with cell-local ranges
(`doc_highlights.rs`, `document_symbols.rs` use `local_range()` on targets in other cells), and
`prepareCallHierarchy` returns the notebook's file URI with cell-local ranges. A finding keeps
`lsp/in/c_<i>.py` (the program alone when it shows alone, else the session's files) and `lsp/seed.txt`;
`repro.sh` reruns the session. `TYZ_LSP=0` turns the oracle off.

### Metamorphic rewrites (`py/meta.py`)

For the programs of a batch that pass the oracles above, `meta.py` rewrites each check file (with its
assertions) into equivalent programs, which ty checks in one more run (`<batch>/meta/<v>_<i>.py`). Python
evaluates each rewrite exactly like the original, so ty must again report no error and fail no assertion:

- `n` negate: `if c: A else: B` becomes `if <not c>: B else: A` (elif chains nest), `a if c else b`
  becomes `b if <not c> else a`, comprehension filters and `while` conditions become `not <not c>`;
  `<not c>` is `not (c)` or pushed inwards (De Morgan, `is`/`is not`, `in`/`not in`, `==`/`!=`).
- `a` alias: annotations or their parts go through type aliases (`type _Ak[T] = ...` generic over the
  PEP 695 parameters they use, `TypeAlias`, implicit, `TypeAliasType`), defined before the top-level
  statement that uses them.
- `s` spell: union members reordered, `X | Y` / `Union[...]` / `Optional[...]`, `typing.List` etc.,
  `Literal[a, b]` split, `Annotated[X, n]` wrappers.
- `q` quote: whole annotations or subscript arguments as strings.
- `m` module: the top-level definitions split at a point into two modules that import each other.
  `head`: the definitions before the point move to `md_<i>` (`headcycle` when one of them reads a
  later name inside a function body, spelled `m_<i>.X` through an import cycle); `tail`: the ones
  after it move, `md_<i>` imports what it needs back from `m_<i>` (a cycle Python resolves, the
  import of `md_<i>` comes after `m_<i>`'s own definitions), the trailing `main()` stays. Styles
  for importing `md_<i>` (hashes of the program's top-level names and the split point):
  `explicit` (`from md_<i> import a, b`), `star` (`*`, with `__all__` in one of its idioms or
  without), `package` (`mp_<i>/__init__.py` re-exports `mp_<i>/mpd_<i>.py` by `*` + `__all__ +=`,
  by `import X as X`, or by importing `__all__`), `qualified` (`import md_<i>` / `import
  mp_<i>.mpd_<i>` / `import md_<i> as _mq`, every use qualified), `rename` (`import X as X_r`).
  Errors in either file count; the signature is `meta__module-<mode>-<style>__<rule>`
  (`divergent` when any message has `Divergent`). One split in 256 (`TYZ_META_STAR_CYCLE`) is
  `tailstar`: `*` imports both ways, each module with its own `__all__`; ty leaks `Divergent`
  into the first module's public types there (a.py `x = 1` ... `from b import *`, b.py
  `__all__ = ["g"]; from a import *`: `x` is `Literal[1] | Divergent`), all under
  `meta__module-tailstar__star-cycle`. `*` imports without `__all__` of a module that imports
  back are left to that shape too.

- `p` stub: the `m` split again (its own split point and style, never the mutual-`*` shape; files `p_<i>`,
  `pd_<i>`, `pp_<i>/ppd_<i>`), and the moved module gets a `.pyi` stub next to it (`Stubber`), written the
  way stub generators and typeshed do: function bodies `...` (an overload's implementation dropped, async
  generators as plain `def`s), instance attributes assigned through `self` declared in the class body (type
  from the annotation, the annotated parameter or the constant; `Any` otherwise; programs where that would
  add a field to a dataclass-like class or a member to a protocol are skipped), and by hash some annotated
  defaults, annotated module / plain-class / `@dataclass` / NamedTuple values and plain `Enum` member values
  `...` (not `IntEnum`/`StrEnum`/`Flag`: their `==` uses the values, which ty then cannot know), some
  module declarations without a value, and one top-level definition in six under a `sys.version_info`
  check with a decoy of the same name in the branch 3.13 does not take. One module in three has only the stub (`.py`
  removed); half the packages get their `__init__` as `__init__.pyi` too (sometimes only that). Typing spec
  ("Distributing type information"): a stub takes precedence over the `.py` and declares its interface, so
  the importing `p_<i>.py` must keep the original's verdicts: `meta__stub__<rule>` for an error there,
  `meta__stub-pyi__<rule>` for an error in a generated stub. The moved `.py` itself is not judged when it
  has a stub: when it imports names back, its classes are not the stub's for ty (two definitions).

A search batch checks `n`, `m`, `p` and one of `a`/`s`/`q` per program (by position); a single program (`tyz eval`,
`repro`, `minimize`) gets all six. Choices are hashes of the rewritten node's own text, so they survive
shrinking. `meta__<negate|alias|spell|quote>__<rule>`, where `<rule>` is `assign`/`disjoint` for an
assertion that held in the original; a panic is `crash__panic__*` as usual. To find the rewrite that
matters: `TYZ_META_ONLY=<k>` makes `meta.py` rewrite only its k-th annotation (variants a/s/q).
`TYZ_META=0` turns the oracle off.

`known.json` lists triaged findings: a signature prefix plus a substring of the message or of an error.
They are counted as `known__<name>` and not saved again.

### Return type inference and check order (`py/ord.py`)

ty infers the return types of unannotated functions only under `[analysis] return-type-inference =
"always"` (off by default; "Enabling return type inference will never override explicit return type
annotations"; methods of non-final classes get `<inferred> | Unknown`, generators and async functions
`Unknown`). For the programs of a batch that pass the oracles above, `ord.py` writes `<batch>/ord/`
(with that `ty.toml`): `o_<i>.py`, the check file with most return annotations dropped (kept:
overloads, stub-like bodies, `TypeIs`/`TypeGuard`, one function in four by a hash of its text), then
`reveal_type` of each top-level name; `oi_<i>.py`, which imports them (`from o_<i> import ...`,
`import o_<i>`) and reveals them both ways; and meta.py's module split of the program (`m_<i>` and its
other files, not the mutual-`*` shape) with the same changes. The probes' assertions stay:

- `ret__disjoint__<kind>[-x]`: an inferred type excludes the runtime value (a `:d` assertion fails;
  `<kind>`: `zipenum` when the value is a target of `for ... in enumerate|zip(<conditional>)` (one known
  bug), else `func`, `literal`, `union`, `tuple`, `float`, `none`, a builtin class name or `instance`;
  `-x` when only the module split fails). Other errors are not findings: inferred returns are
  narrower than the annotations they replace (a dict literal for a TypedDict, invariant containers);
  in a unit with such an error no assertion counts (declarations then narrow what probes see).
  Two shapes keep their annotations in 7 functions of 8, each hitting one bug found here: generator,
  async and iterator-protocol methods (their `Unknown` / `X | Unknown` iterables in a union get dropped
  when `enumerate`/`zip` solve their type variable, `ret__disjoint__union`/`list`/...) and functions
  returning a program function by name (`TypeOf[f0]` disjoint from the inferred `TypeOf[f1()]`,
  `ret__disjoint__func`).
- `ord__<entry|defs|seq>-<rule>__<single|module-head|module-tail|module-headcycle>`: in one batch of
  TYZ_ORD_EVERY (2), `ty check` of only the importing files (`oi_*`, `m_*`), of only the imported
  ones, and of all on one thread (`TY_MAX_PARALLELISM=1`) must give every file it checks the same
  diagnostics, reveals included, as the parallel check of all: which query enters a cycle first
  must not change a result. `<rule>` is the first differing diagnostic's (`divergent` when one
  names `Divergent`).

The finding keeps `ord/`. `TYZ_ORD=0` turns the oracle off.

### Suppression round trips (`src/rt.rs`)

For the programs of a batch that pass the oracles above, `rt` makes an ill-typed variant (two
`incr` edits, without cutting lines short except in one program in eight), appends `ty: ignore` /
`type: ignore` comments to some lines, and reflows some lines over several physical lines (a break
after a comma inside brackets, a backslash continuation, rarely right before a trailing comment
(only in one file in eight: that shape hits known `--fix`/`--add-ignore` bugs in most files), a
`"..."` literal turned into a triple-quoted one with a line break). Then each file independently
gets the text editors leave (`texture`): non-ASCII text in some `"..."` literals and comments, CRLF
line endings, a UTF-8 BOM, tab indentation, no final newline. The `--fix` variant also has
`cast(T, e)` around some expressions (`py/casts.py`; `T` guessed from the expression's shape, in
several spellings: keywords, `typing.cast`, a string `T`, over several lines, nested). On all of them
together:

- `ty check --add-ignore` ("Adds `ty: ignore` comments to suppress all rule diagnostics"), then a
  fresh check: `rt__ignore__<rule>` when a lint diagnostic survives in a file without syntax errors
  (or an unused ignore comment that was not unused before), `rt__ignore__internal-converge` /
  `internal-syntax` for ty's own "Fixes failed to converge" / "introduced a syntax error",
  `rt__ignore-ast__changed`/`syntax` when the program's AST changed, `rt__ignore-report__<rule>`
  when the remaining diagnostics `--add-ignore` reports differ from the fresh check's,
  `rt__ignore-idem__changed` when a second `--add-ignore` changes a file again.
- `--update-baseline --baseline base.json`, then `--baseline base.json`: `rt__baseline__<rule>` when
  a lint diagnostic of that same check is not suppressed; `rt__baseline-shift__<rule>` the same after
  two lines are added at the top of every file (baseline.rs: line numbers are not recorded);
  three batches in four start from an empty baseline file (writing one where none exists yet panics
  now and then: `crash__panic__smallsort_854_update-baseline`);
  `rt__baseline-update__changed` when updating the unchanged project's baseline rewrites it.
- `--fix` on another variant with more ignore comments: `rt__fix__internal-*`,
  `rt__fix__unused-ignore-comment` (an unused ignore left after `--fix`), `rt__fix-ast__syntax`,
  `rt__fix-ast__changed` when the fixed file's AST is not the input's with some casts removed
  (`astcmp.py --uncast`: the only safe fixes remove unused ignore comments and redundant casts,
  "Remove the redundant `cast`"), `rt__fix-report__<rule>`.

`ty.toml` turns on some rules that are off by default. The finding keeps `rt/in/` (`r_<i>.py`,
`f_<i>.py`; `rt/only.txt` names the file when only the whole batch reproduces); `repro.sh` reruns
the oracle on them. `tyz rt-inputs <file.py>` prints a program's inputs. A search runs `rt` on the first
`TYZ_RT_N` (8) passing programs of one batch in `TYZ_RT_EVERY` (2). `TYZ_RT=0` turns it off.

## Commands

```sh
. ./env.sh                       # TYZ_TY (ty binary), TYZ_PY (python3.13)
cargo build --release
./target/release/tyz search <seed> <secs> <out>   # TYZ_WORKERS (4), TYZ_BATCH (48), TYZ_MAX_CASES
./target/release/tyz minimize <out>/findings/<sig>/<n> [secs]   # min.py, min_check.py, finding.json
./target/release/tyz eval prog.py                 # one program: check file and verdict
./target/release/tyz gen <seed> <n> <dir>         # samples
./show.sh <finding-dir>...                        # minimized program and messages
./cov.sh <seed> <secs> <out>                      # search against a coverage build; <out>/cov-files.txt
```

`search` saves at most three originals per signature (`findings/<sig>/<n>/{original.py,tree.json,
finding.json}`) and rewrites `stats.json` after every batch: cases, invalid (by exception), passes,
probes, asserts, and failures by signature. Minimizing runs separately, so the search doesn't stall.

ty builds used locally (`ruff/` checkout at the monorepo root, gitignored):
`target/release/ty` (release), `target-da/release/ty` (`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`,
overflow checks), `target-cov/aarch64-apple-darwin/release/ty` (`-C instrument-coverage`, no LTO;
`cov.sh`).

## ephyra-agent contract

`build.sh <src> <build> [coverage]` builds ty from a ruff checkout `<src>` into `<build>/bin/ty` (release,
no LTO, debug assertions and overflow checks; with `coverage`, `-C instrument-coverage` and
`<build>/coverage.json` instead), tyz into `<build>/bin/tyz` (against ephyra at `$EPHYRA`, default
`/opt/ephyra`), and links a CPython 3.13+ as `<build>/bin/py` (`$TYZ_PY`, or a uv install under
`/agent-run/work/.python`). `run.sh <build> <seed> <secs> <out>` runs `tyz search` with
`TYZ_WORKERS` (6) workers. `stats.json` has `failures` (non-known failures). Each finding has
`finding.json` (`signature`, `oracle`, `message`, `seed`), `original.py`, `tree.json` and `repro.sh`
(`repro.sh <build>` runs `tyz repro`: exit 1 when `original.py` still fails with the signature,
0 when it passes, 2 when it no longer runs under CPython). `known.json` is compiled in (`TYZ_KNOWN`
overrides it).

## Results

See `../bugs/STATUS.md`. Generator rounds are in `../rounds/<n>/` (ROUND-LOG.md), scored by covered
lines of `ty_python_semantic` over a fixed number of programs (`TYZ_MAX_CASES=4800 ./cov.sh 11 ...`).

## Loop notes (2026-10-26)

- build.sh: when ruff's pinned toolchain (`rust-toolchain.toml`) is not usable (RUSTUP_HOME read-only,
  partial install), it falls back to `RUSTUP_TOOLCHAIN=stable`. The coverage map then points at that
  toolchain's llvm-tools.
- Measured on this machine (6 workers, batch 40): ~62 cases/s alone, ~24 cases/s each with 4 runs in
  parallel, ~15 cases/s on the coverage build. 0 failures in ~38k clean cases; a few `known__*` hits.
