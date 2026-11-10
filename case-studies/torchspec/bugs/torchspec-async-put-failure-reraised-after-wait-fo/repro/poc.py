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
