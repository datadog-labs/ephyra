DRAFT, not filed
Title: (comment on #8336) Same panic when the window call's argument is non-deterministic and the call appears twice, also from first_value/nth_value

Comment text for #8336:

Another way to reach this panic: the argument of `lag`/`lead` is a non-deterministic call (`random()`, `date(...)`, `strftime(...)`, `julianday(...)`, ...), the query has a second window, and the window call appears twice, either written twice or referenced again by the outer `ORDER BY`. `first_value`/`nth_value` hit the sibling `unreachable!` in the same file.

```sql
CREATE TABLE t(a, b);
SELECT count(*) OVER (), lag(random()) OVER (ORDER BY a) FROM t ORDER BY 2;
SELECT count(*) OVER (), lag(random()) OVER (ORDER BY a), lag(random()) OVER (ORDER BY a) FROM t;
SELECT count(*) OVER (), first_value(date(b)) OVER (ORDER BY a) FROM t ORDER BY 2;
```

Turso (each statement on its own; the table may be empty):
```
thread 'main' panicked at core/translate/window.rs:3394:22:
internal error: entered unreachable code: lag/lead arg[0] must be a buffer column reference after planner rewrite, got Some(FunctionCall { name: Name { quote: None, value: "random" }, ... })

thread 'main' panicked at core/translate/window.rs:3394:22:
internal error: entered unreachable code: lag/lead arg[0] must be a buffer column reference after planner rewrite, got Some(FunctionCall { name: Name { quote: None, value: "random" }, ... })

thread 'main' panicked at core/translate/window.rs:3530:22:
internal error: entered unreachable code: first_value/nth_value arg[0] must be a subquery column ref after planner rewrite, got Some(FunctionCall { name: Name { quote: None, value: "date" }, ... })
```

SQLite 3.50.4 (and trunk): no rows, no error.

Each of these changes avoids the panic: a deterministic argument (`abs(b)`, `b+1`, `b`); dropping `count(*) OVER ()`; dropping the `ORDER BY 2` or the repeated call. `group_concat(random())` in the same position does not panic.

Tested on Turso main cbd5ac4e.
