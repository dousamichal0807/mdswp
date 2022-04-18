# mdswp

Sliding window protocol implementation in Rust.

This crate consists of two public structs:

 - `MdswpListener`, which is used in a same way as `std::net::TcpListener`
 - and `MdswpStream`, which is used as `std::net::TcpStream`.

By same usage it is meant that both structs, from `mdswp` library and from Rust's standard library, share the same methods.

## Documentation

Documentation can be generated. Steps to generate it:

```shell
# 1. Download the GitHub repository:
git clone https://github.com/dousamichal0807/mdswp.git
# 2. Navigate into the repository
cd mdswp
# 3. Channge branch from development to a stable branch, for example:
git checkout v0.2.0
# 4. Generate documentation:
cargo doc --release --no-deps
```

Now, documentation is generated in `target/doc` directory. You can open `index.html` in your browser to see mdswp's documentation.

## Usage

It is recommended to use it in your `Cargo.toml` like this:

```toml
[dependencies]
mdswp = { git = "https://github.com/dousamichal0807/mdswp.git", branch = "v0.2.0" }
# your other dependencies here
```

You can use other branch, as well.