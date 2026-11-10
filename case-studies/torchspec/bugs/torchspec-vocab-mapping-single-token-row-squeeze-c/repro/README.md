Run against an upstream TorchSpec checkout (CPU is enough, no tokenizer or network needed):

    PYTHONPATH=/path/to/TorchSpec python poc.py
    ./run.sh /path/to/python     # exit 1 = bug reproduces, 0 = fixed

Needs torch, numpy, numba and tqdm (TorchSpec's own dependencies).
