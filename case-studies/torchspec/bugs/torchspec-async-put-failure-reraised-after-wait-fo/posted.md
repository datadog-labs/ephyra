**Description**

`AsyncPutManager.wait_for_buffer` in `torchspec/transfer/mooncake/buffers.py` raises the exception of a failed background `batch_put_from` to its caller. Before re-raising, it also stores the exception in `self._last_error`. The next `check_last_error()` then raises the same exception again, even though it has already been surfaced and nothing new has failed.

`EagleMooncakeStore._put_raw_tensors` (async host-buffer path, `eagle_store.py` lines 105-107) calls `check_last_error()` and then `wait_for_buffer(buf.ptr)`. `flush()` calls `check_last_error()` (lines 307-309). So when sample A's put fails, the failure is raised from the `put()` of a later sample B that reuses A's host buffer. The caller has now seen A's error. The next `put()` or `flush()`, for some other sample C whose puts are fine, raises A's error a second time. That aborts C's publication too.

Expected: each async failure is raised once. The `check_last_error` docstring says:

> Re-raise the first async failure that hasn't been surfaced yet.

The `wait_for_buffer` docstring says it "Raises the original exception if the transfer failed", so a failure raised there has been surfaced. `check_errors()` (added in #188) already follows this rule: it pops the failed future, sets `_last_error` and clears it immediately in the same call. `tests/test_mooncake_async_publish.py::test_async_error_check_surfaces_completed_failure` checks that it surfaces the failure and retires the future.

**Steps to reproduce**

No GPU, Mooncake or tokenizer files are needed. A stub store stands in for `MooncakeDistributedStore`. Save as `poc.py` in a TorchSpec checkout:

```python
from torchspec.transfer.mooncake.buffers import AsyncPutManager

class FailingStore:  # stands in for MooncakeDistributedStore; every put fails
    def batch_put_from(self, keys, ptrs, sizes):
        return [-200] * len(keys)
    def batch_remove(self, keys, force=False):
        pass

m = AsyncPutManager(FailingStore())
m.submit(["sample0_hs"], [0x1000], [8], owner_buffer_ptr=0x1000)

# put() of the next sample reuses the buffer: sample0's failure is raised here (correct).
try:
    m.wait_for_buffer(0x1000)
except RuntimeError as e:
    print("wait_for_buffer raised:", e)

# Nothing else was submitted, so there is no unsurfaced failure left.
try:
    m.check_last_error()
    print("check_last_error: nothing raised")
except RuntimeError as e:
    print("check_last_error raised:", e)
```

```
python poc.py
# expected (docstring): check_last_error: nothing raised
# torchspec:            check_last_error raised: async batch_put_from failed: sample0_hs (code=-200)
```

**Observed output**

```
wait_for_buffer raised: async batch_put_from failed: sample0_hs (code=-200)
check_last_error raised: async batch_put_from failed: sample0_hs (code=-200)
```

**Expected output**

```
wait_for_buffer raised: async batch_put_from failed: sample0_hs (code=-200)
check_last_error: nothing raised
```

The same thing happens end to end through `EagleMooncakeStore.put()` with a stubbed Mooncake store. With a pool of host buffers, a failed put of sample `s0` is raised by the `put()` that reuses its buffer. The following call, publishing sample `s2`, then fails with `async batch_put_from failed: s0_hs (code=-200)`. It also happens with `put_usp_shards`, e.g. `s1_usp0_hs` is raised again while publishing sample 2.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.10, Ubuntu 24.04.5 LTS, Linux 6.8.0 x86_64
- CPU only, no GPU

**Latest release**

The repository has no tagged releases (version.txt says 0.1.0). The code came in with #20 (803c1b5, "[Feature] add async put support for mooncake eagle store"). It has not changed since, so every commit since then has it.

**Related issues and PRs**

- #20 added `AsyncPutManager` and this `wait_for_buffer` code.
- #188 added `check_errors()`, which surfaces each completed failure only once.
- I found no open or closed issue or PR that reports this. I searched the issue and PR titles. GitHub search was not available.

A fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
