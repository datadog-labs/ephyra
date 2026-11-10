# Posted: https://github.com/tigerbeetle/tigerbeetle/issues/3960

Filed by alpaylan, 2026-11-10. Text as posted:

**State machine bug when two transfers share the same expiry nanosecond**

**Running the following test:**

```zig
test "expire_pending_transfers: expiry not before expires_at" {
    try check(
        \\ account A1  0  0  0  0  _  _  _ _ L1 C1   _   _   _ _ _ _ _ _ created
        \\ account A2  0  0  0  0  _  _  _ _ L1 C1   _   _   _ _ _ _ _ _ created
        \\ commit create_accounts
        \\ transfer   T1 A1 A2   10   _  _  _  _    2 L1 C1   _ PEN   _   _   _   _  _ _ _ _ _ created
        \\ commit create_transfers
        \\ tick 29 nanoseconds
        \\ transfer   T2 A1 A2   10   _  _  _  _    2 L1 C1   _ PEN   _   _   _   _  _ _ _ _ _ created
        \\ commit create_transfers
        \\ tick 999999999 nanoseconds
        \\ transfer   T3 A1 A2   10   _  _  _  _    1 L1 C1   _ PEN   _   _   _   _  _ _ _ _ _ created
        \\ commit create_transfers
        \\ tick 999999970 nanoseconds
        \\ tick 1 seconds
        \\ lookup_account A1   0  0   0  0  _
        \\ lookup_account A2   0  0   0  0  _
        \\ commit lookup_accounts
    );
}
```

**Aborts with the following error:**

```
error: 'state_machine_tests.test.expire_pending_transfers: expiry not before expires_at' terminated with signal ABRT
thread 204 panic: reached unreachable code
src/state_machine.zig:5281:23: in execute
                assert(expires_at <= timestamp_event);
src/state_machine.zig:4514:78: in execute_pulse
src/state_machine.zig:2562:45: in commit
```
