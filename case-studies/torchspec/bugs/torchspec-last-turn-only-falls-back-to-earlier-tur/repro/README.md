# PoC: compute_assistant_loss_mask(last_turn_only=True) with an empty last assistant turn

Run with a Python environment where TorchSpec (main) is installed, plus transformers and huggingface_hub:

    python poc.py

It downloads only the Qwen/Qwen3-0.6B tokenizer files at revision c1899de2 (the revision pinned in
tests/tokenizer_contract/fixtures/manifest.json). CPU only.

Exit 1 = bug present (prints `trains on: 'OLD ANSWER'`), exit 0 = fixed (prints `trains on: ''`).

The same thing without a tokenizer:

    python -c "from torchspec.models.ops.loss_mask import compute_assistant_loss_mask as f; print(f([1,2,4,1,4],[1],[4],last_turn_only=True).tolist())"
    # expected: [0, 0, 0, 0, 0]   actual on main: [0, 1, 0, 0, 0]
