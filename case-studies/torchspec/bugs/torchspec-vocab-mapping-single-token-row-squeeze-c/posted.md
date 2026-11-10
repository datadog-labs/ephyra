**Description**

`generate_vocab_mapping` raises a `ValueError` from `np.concatenate` if any prompt's `input_ids` is a 1-D tensor with a single token.

The cause is `_count_token_frequencies` (`torchspec/data/preprocessing.py:583`), which runs `input_ids.squeeze().numpy()`. When a 1-D tensor has length 1, `squeeze()` turns it into a 0-d array. The `np.concatenate(all_ids)` call at line 598 then fails.

These rows are valid input. `_load_pretokenized_dataset` (`torchspec/data/dataset.py:106`) stores `input_ids` as 1-D tensors. It rejects only empty rows ("Pretokenized sample {idx} has empty input_ids") and rows with no supervised tokens. A one-token supervised row passes every check.

The `generate_vocab_mapping` docstring says it accepts:

> prompts: List of dicts with input_ids and packed_loss_mask (as returned by load_conversation_dataset).

`TrainingController.compute_vocab_mapping` (`torchspec/controller/training_controller.py:352`) passes the stored dataset straight to this function. As a result, one short row in a dataset stops vocab-mapping generation.

Expected: a one-token row is counted like any other row, and the function returns `(d2t, t2d)`.

**Steps to reproduce**

No tokenizer, model or GPU is needed. Save this as `poc.py`:

```python
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
```

```
PYTHONPATH=/path/to/TorchSpec python poc.py
# expected: loaded 2 rows ... ok torch.Size([8]) 8
# torchspec: loaded 2 rows, then ValueError (traceback below)
```

A smaller variant, with a single row:

```
python -c "import torch; from torchspec.data.preprocessing import generate_vocab_mapping; generate_vocab_mapping([{'input_ids': torch.tensor([3])}], 32, 8)"
# expected: returns (d2t, t2d)
# torchspec: ValueError: zero-dimensional arrays cannot be concatenated
```

**Traceback**

```
loaded 2 rows
Traceback (most recent call last):
  File "poc.py", line 13, in <module>
    d2t, t2d = generate_vocab_mapping(rows, target_vocab_size=32, draft_vocab_size=8)
  File "torchspec/data/preprocessing.py", line 624, in generate_vocab_mapping
    token_dict = _count_token_frequencies(prompts)
  File "torchspec/data/preprocessing.py", line 598, in _count_token_frequencies
    flat_ids = np.concatenate(all_ids)
ValueError: all the input arrays must have same number of dimensions, but the array at index 0 has 1 dimension(s) and the array at index 1 has 0 dimension(s)
```

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (main, 2026-10-29)
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3, numpy 2.3.5, numba 0.68.1
- Python 3.12.3, Linux 6.8 x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

TorchSpec has no tagged release. The `squeeze()` line has been unchanged since the initial commit (4643a09, "Initial release of TorchSpec"), so every version of main has this bug.

**Related issues and PRs**

I found no matching open or closed issue or PR. #163 added `_load_pretokenized_dataset`, which accepts one-token rows, but it did not touch this line.

A one-line fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
