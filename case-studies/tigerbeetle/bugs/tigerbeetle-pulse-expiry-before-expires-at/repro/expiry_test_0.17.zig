
test "expire_pending_transfers: expiry not before expires_at" {
    // The testing state machine expires up to 30 transfers per pulse, so a pulse
    // prepared at timestamp P is committed at T = P + 1 + 30 (this harness adds 1 ns per prepare) and its expiry events get
    // timestamps T-n+1..T. T2 and T3 both have expires_at == T (T3 is created exactly
    // 1 s after T2 (offsets include the harness's 1 ns per prepare), with a 1 s shorter timeout).
    try check(
        \\ account A1  0  0  0  0  _  _  _ _ L1 C1   _   _   _ _ _ _ _ _ created
        \\ account A2  0  0  0  0  _  _  _ _ L1 C1   _   _   _ _ _ _ _ _ created
        \\ commit create_accounts
        \\ transfer   T1 A1 A2   10   _  _  _  _    2 L1 C1   _ PEN   _   _   _   _  _ _ _ _ _ created
        \\ commit create_transfers
        \\ tick 29 nanoseconds
        \\ transfer   T2 A1 A2   10   _  _  _  _    2 L1 C1   _ PEN   _   _   _   _  _ _ _ _ _ created
        \\ commit create_transfers
        \\ tick 999999998 nanoseconds
        \\ transfer   T3 A1 A2   10   _  _  _  _    1 L1 C1   _ PEN   _   _   _   _  _ _ _ _ _ created
        \\ commit create_transfers
        // T1 is now due: the pulse scans everything with expires_at <= T, i.e. T1, T2, T3,
        // and commits their expiries at T-2, T-1, T. T2 expires at T-1, before its
        // expires_at, and assert(expires_at <= timestamp_event) fails.
        \\ tick 999999969 nanoseconds
        \\ tick 1 seconds
        \\ lookup_account A1   0  0   0  0  _
        \\ lookup_account A2   0  0   0  0  _
        \\ commit lookup_accounts
    );
}
