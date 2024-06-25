# Chimera-md

Chimera-md is a Markdown-aware documentation server.

I have spent years developing a hard drive full of notes and documents written in the
[Markdown](https://www.markdownguide.org/) text processing language, and while it is
comparatively easy to view them as intended in a special editor, most often I would wind
up seeing them in plain text form. I went looking for a server I could host that would
serve up HTML-ified versions of those documents transparently. Most of the tools I
found were static site generators, or had strong opinions about how the document should
be formatted.

Chimera-md is my attempt to make a very simple web server that understands and can serve
a library of markdown files (and supporting assets) transparently. It is a full web server
and can handle ordinary files, but with special processing for markdown files.

## Goals

1. Transparently serve Markdown files
2. Cooperate with an existing fast, efficient web server (a la [Axum](https://docs.rs/axum/latest/axum/))
3. Serve multiple origin folders
4. Nice-looking theme
5. Simple site navigation
6. Full text search
7. Live updating when files change

## Contributing

I have been finding the [cargo-watch](https://crates.io/crates/cargo-watch) tool useful to speed development.
This command line enables a watcher to spin a new server any time I save changes:
```
cargo watch -x run -w src/
```

Build for Linux:
```
cargo build --release --target=x86_64-unknown-linux-gnu
```

## Arguments

There are currently 5 arguments that can be set either via environment or the command line.

```bash
    --document-root <DOCUMENT_ROOT>  [env: CHIMERA_DOCUMENT_ROOT=/var/chimera-md/www]
    --template-root <TEMPLATE_ROOT>  [env: CHIMERA_TEMPLATE_ROOT=/var/chimera-md/template]
    --site-title <SITE_TITLE>        [env: CHIMERA_SITE_TITLE=Chimera-md]
    --index-file <INDEX_FILE>        [env: CHIMERA_INDEX_FILE=index.md]
    --log-level <LOG_LEVEL>          [env: CHIMERA_LOG_LEVEL=INFO]
    --port <PORT>                    [env: CHIMERA_HTTP_PORT=8080]
```

## Acknowledgements

* I want to call out [this video by Rainer Stropek](https://www.youtube.com/watch?v=y5W6PErCc2c) for the very cogent guide to using Axum. It was a huge help.

## Libraries

Chimera-md uses the following open source libraries:

* [Skeleton CSS framework](http://getskeleton.com/)
* [Axum web framework](https://github.com/tokio-rs/axum)
* [Handlebars](https://github.com/sunng87/handlebars-rust)
* [pulldown-cmark](https://crates.io/crates/pulldown-cmark)
* [Tantivy](https://crates.io/crates/tantivy)
* [tokio](https://tokio.rs/)
* [tower-http](https://crates.io/crates/tower-http)
* [tracing](https://crates.io/crates/tracing)
* [serde](https://crates.io/crates/serde)
* [clap](https://crates.io/crates/clap)
* [regex](https://crates.io/crates/regex)
* [urlencoding](https://crates.io/crates/urlencoding)
* [tempfile](https://crates.io/crates/tempfile)
* [walkdir](https://crates.io/crates/walkdir)
* [async-watcher](https://crates.io/crates/async-watcher)

## License

This project is open sourced under the MIT [License](License.txt)
