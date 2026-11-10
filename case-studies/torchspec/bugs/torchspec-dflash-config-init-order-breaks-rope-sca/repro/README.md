# Repro: DFlashConfig crashes on a YaRN rope_scaling

Needs TorchSpec installed (tested at c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9) with transformers 5.20.2.
CPU only. No downloads, no tokenizer files, no GPU.

    python poc.py

Unfixed:  prints "LlamaConfig: ok", then AttributeError: 'DFlashConfig' object has no attribute 'max_position_embeddings' (exit 1).
Fixed:    prints "LlamaConfig: ok" and "DFlashConfig: ok" (exit 0).
