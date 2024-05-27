# Chimera

I am making this tool to make it easy to index a variety of markdown documents I have
lying around, served through a web server with included style information and full
text search. It is also a project goal to learn about making a web app (probably with
[Nginx](https://nginx.org/en/)) and a [Docker container](https://www.docker.com/).

I keep going back and forth on whether the output should be static HTML or those file
should be cached intermediates. I should figure out how [MkDocs](https://github.com/mkdocs)
manages assets referenced by the files.

-----------------

Anyway, here's some `Markdown` features to test with.

```rust
    let blocks = markdown::to_mdast(md_content.as_str(), &ParseOptions::default());
    match blocks {
        Ok(node) => {
            stdout().write_all(node.to_string().as_bytes()).await.unwrap();
        },
        Err(e) => {
            eprintln!("Failed converting to markdown?: {e}");
        }
    }
```

![sky-box](./assets/documentation-img-1.jpg)
