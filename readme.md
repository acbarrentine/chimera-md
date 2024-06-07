# Chimera

A Markdown documentation server. Modeled after my guess about how [MKDocs](https://github.com/mkdocs/mkdocs)
works, even though I've never used that tool

## Goals

1. Transparently serve Markdown files
2. Cooperate with an existing fast, efficient web server (a la [Nginx](https://nginx.org/en/))
3. Serve multiple origin folders
4. Nice-looking theme
5. Simple site navigation
6. Full text search

## Contributing

* I have been finding the cargo-watch tool useful to speed development. This command line enables a watcher to spin
a new server any time I save changes:
```
cargo watch -x run -w src/
```

## Acknowledgements

* I want to call out [this video by Rainer Stropek](https://www.youtube.com/watch?v=y5W6PErCc2c) for the very cogent guide to using Axum. It was a huge help.
* Chimera uses the [Skeleton CSS framework](http://getskeleton.com/) under the MIT license
* It also uses [Axum web framework](https://github.com/tokio-rs/axum), which is similarly under the MIT license
* [pulldown-cmark](https://crates.io/crates/pulldown-cmark) also uses the MIT license
* [tokio](https://tokio.rs/) also uses the MIT license
* [Handlebars](https://github.com/sunng87/handlebars-rust) also uses the MIT license

## License

Like all the components I'm using, this project is open sourced under the MIT license