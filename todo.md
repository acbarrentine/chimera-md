# Todo

**Watch and build executable**

* `cargo watch -x run -w src/`

## Cross-compile?

`uname -a` on Synology:
```
Linux dismal-nas 4.4.302+ #69057 SMP Fri Jan 12 17:02:28 CST 2024 x86_64 GNU/Linux synology_geminilake_423+
```
Homebrew installer:
* https://formulae.brew.sh/formula/x86_64-elf-gcc

GCC cross-compiler:
* https://wiki.osdev.org/GCC_Cross-Compiler


Rust user targeting x86_64 linus
* https://github.com/briansmith/ring/issues/1605

Old piece on the subject
* https://roscopeco.com/2018/11/25/using-gcc-as-cross-compiler-with-x86_64-target/

Rust user group
* https://users.rust-lang.org/t/cross-compiling-linking-with-gcc/67021

The book
* https://doc.rust-lang.org/cargo/reference/config.html?highlight=linker

Someone targeting linux
* https://kyle.buzby.dev/notes/compiling-rust-for-ds418j/

## Page design

There are some interesting design elements on the [Tantivy tutorial](https://tantivy-search.github.io/examples/basic_search.html)

* Note the popup menu
* 2 tone background
* Vertial alignment of elements across the two columns

I also admire [this look](https://ryhl.io/blog/actors-with-tokio/). Nice and simple two panel design
