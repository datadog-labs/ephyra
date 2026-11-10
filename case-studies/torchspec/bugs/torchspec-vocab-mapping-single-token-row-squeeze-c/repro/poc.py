# generate_vocab_mapping crashes when any loaded row has exactly one token.
from torchspec.data.dataset import _load_pretokenized_dataset
from torchspec.data.preprocessing import generate_vocab_mapping

rows = _load_pretokenized_dataset(
    [
        {"input_ids": [5, 6, 7], "loss_mask": [0, 1, 1]},
        {"input_ids": [3], "loss_mask": [1]},  # one token, accepted by the loader
    ],
    max_length=16,
)
print("loaded", len(rows), "rows")
d2t, t2d = generate_vocab_mapping(rows, target_vocab_size=32, draft_vocab_size=8)
print("ok", d2t.shape, int(t2d.sum()))
