Run from a TorchSpec checkout (or with it on PYTHONPATH), CPU only, no network or tokenizer needed:

    python poc.py

On c4fc3e0 the two cos rows differ (torchspec ignores the linear factor 2).
With the fix they are equal: [-0.9365, 0.9394, 0.9994, 1.0].
