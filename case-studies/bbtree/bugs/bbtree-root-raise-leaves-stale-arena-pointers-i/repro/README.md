Standalone reproducer. Edit the `bbtree-mmap` path in Cargo.toml to point at an upstream
checkout (or use `bbtree-mmap = "0.1.6"` from crates.io), then:

    cargo run --release

Bug present: "160 bytes differ" plus two different pointer-like words.
Fixed: "0 bytes differ".
