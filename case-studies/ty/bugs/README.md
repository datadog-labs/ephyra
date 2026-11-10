# Bugs: ty

The bugs the case studies found in ty. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Accepting a call-argument-name inlay hint can produce a syntax error (`f((1))` → `f((x=1))`, `f(1, 2)` → `f(x=1, 2)`)](ty-argname-inlay-hint-edit-before-later-pos/) | fixed | [link](https://github.com/astral-sh/ty/issues/4855) |
| [Accepting a call-argument-name inlay hint can produce a syntax error (`f((1))` → `f((x=1))`, `f(1, 2)` → `f(x=1, 2)`)](ty-argname-inlay-hint-inside-parentheses/) | fixed | [link](https://github.com/astral-sh/ty/issues/4855) |
| [Call-argument-name inlay hint on a union callee uses the first element's parameter name](ty-argname-inlay-hint-on-union-callee/) | open | [link](https://github.com/astral-sh/ty/issues/4858) |
| [[panic] `--update-baseline` aborts: "user-provided comparison function does not correctly implement a total order"](ty-baseline-sort-total-order/) | fixed | [link](https://github.com/astral-sh/ty/issues/4839) |
| [Narrowing lost in a comprehension inside a nested function](ty-closure-narrowing-comprehension/) | open | [link](https://github.com/astral-sh/ty/issues/4825) |
| [[panic] Chained assignment with a missing value (debug assertions)](ty-crash-panic-type-expression-271/) | fixed | [link](https://github.com/astral-sh/ty/issues/4837) |
| [`--fix` for `unused-ignore-comment` after a backslash continuation introduces a syntax error, so no fixes in the file are applied](ty-fix-backslash-ignore-comment/) | fixed | [link](https://github.com/astral-sh/ty/issues/4840) |
| [Generic alias parameter `A[T]` (`type A[T] = list[T]`) gets no context from sibling arguments in a generic call](ty-generic-alias-param-solving/) | open | [link](https://github.com/astral-sh/ty/issues/4830) |
| [Conditional of list literals passed to `list[T]` is rejected when a sibling argument also binds `T`, depending on branch order](ty-generic-call-conditional-collection-literals/) | open | [link](https://github.com/astral-sh/ty/issues/4836) |
| [Lambda gets no type context when the expected type is a union of two or more `Callable`s](ty-lambda-union-of-callables/) | open | [link](https://github.com/astral-sh/ty/issues/4835) |
| [Literal string compared with LiteralString narrows to Never in the != branch](ty-literalstring-eq-negative-never/) | fixed | [link](https://github.com/astral-sh/ty/issues/4826) |
| [Mapping-pattern capture is `object` when the subject was narrowed to an intersection (e.g. after `if d:`)](ty-mapping-pattern-truthy-intersection/) | fixed | [link](https://github.com/astral-sh/ty/issues/4831) |
| [False positive when a generic call over `list[T]` is passed to a bounded TypeVar parameter](ty-max-generic-list-context/) | open | [link](https://github.com/astral-sh/ty/issues/4828) |
| [`unknown-argument` for extra keyword arguments to a pydantic-settings `BaseSettings` model with `extra="allow"`](ty-pydantic-basesettings-extra-allow-reject/) | open | [link](https://github.com/astral-sh/ty/issues/4865) |
| [Rename skips calls of a method of a `@final` generic class through a specialized instance](ty-rename-misses-method-of-final-generic-cl/) | open | [link](https://github.com/astral-sh/ty/issues/4860) |
| [Rename / find references of an attribute skip accesses in platform-dependent unreachable code](ty-rename-skips-attribute-access-in-unreach/) | open | [link](https://github.com/astral-sh/ty/issues/4856) |
| [`x == S.A` narrows a `str` to the StrEnum, although `"a" == S.A`](ty-strenum-eq-narrowing/) | fixed | [link](https://github.com/astral-sh/ty/issues/4827) |
| [`TypeAliasType("A", value=int)` is rejected: "Wrong number of arguments … expected 2, found 1"](ty-typealiastype-keyword-value-rejected/) | fixed | [link](https://github.com/astral-sh/ty/issues/4854) |
| [Dict literal is rejected for a `TypedDict \| dict[str, list[float]]` parameter: inner list inferred without context](ty-typeddict-union-arg-dict-literal/) | fixed | [link](https://github.com/astral-sh/ty/issues/4829) |
| [Accepting the variable-type inlay hint on a subclass override of a `ClassVar` gives `invalid-attribute-override`](ty-vartype-inlay-hint-edit-on-inherited-cla/) | open | [link](https://github.com/astral-sh/ty/issues/4866) |
| [Invalid edit offer on variable-type inlay hint](ty-vartype-inlay-hint-edit-on-non-receiver-/) | open | [link](https://github.com/astral-sh/ty/issues/4859) |
