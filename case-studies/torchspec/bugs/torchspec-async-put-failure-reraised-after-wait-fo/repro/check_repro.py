# AsyncPutManager: a batch_put_from failure raised once by wait_for_buffer (put() on the
# same host buffer) is raised again by the next check_last_error() (next put()/flush()).
# Docstring, torchspec/transfer/mooncake/buffers.py: "Re-raise the first async failure
# that hasn't been surfaced yet."
import sys
from torchspec.transfer.mooncake.buffers import AsyncPutManager

class Store:
    def batch_put_from(self, keys, ptrs, sizes):
        return [-200 for _ in keys]
    def batch_remove(self, keys, force=False):
        pass

m = AsyncPutManager(Store())
m.submit(["s0_hs"], [1], [1], owner_buffer_ptr=1)
try:
    m.wait_for_buffer(1)          # first surfacing: raises the failure of s0
    print("wait_for_buffer raised nothing"); sys.exit(3)
except RuntimeError as e:
    print("first raise:", e)
try:
    m.check_last_error()          # nothing new failed; must not raise
except RuntimeError as e:
    print("BUG: same failure raised again:", e); sys.exit(1)
print("ok"); sys.exit(0)
