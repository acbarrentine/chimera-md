# Markdown example 1

I am making this tool to make it easy to index a variety of markdown documents I have
lying around, served through a web server with included style information and full
text search. It is also a project goal to learn about making a web app (probably with
[Axum](https://github.com/tokio-rs/axum)) and a [Docker container](https://www.docker.com/).

-----------------

Anyway, here are some `Markdown` features to test with.

##### Game table

| Month    | Game                     |
| -------- | ----------------------   |
| January  | Persona 5                |
| February | Final Fantasy: Rebirth   |
| March    | Return of the Obra Din   |
| April    | Curse of the Golden Idol |


##### Code block

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

##### Blockquote

> This is part of a blockquote.
> This continues on the same line.
>
> This does not.

##### Unordered list

* The New York Times
* The Time Picayune
* The Seattle Post-Intelligencer
* The Washington Post
* The London Times

##### A Picture

![sky-box](/assets/img-1.jpg)
