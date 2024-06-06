# Chimera example markdown

I am making this tool to make it easy to index a variety of markdown documents I have
lying around, served through a web server with included style information and full
text search. It is also a project goal to learn about making a web app (probably with
[Axum](https://github.com/tokio-rs/axum)) and a [Docker container](https://www.docker.com/).

-----------------

Anyway, here are some `Markdown` features to test with.

In document links:
1. [Game table](#gametable)
2. [Code block](#codeblock)
3. [Block quote](#blockquote)
4. [Unordered list](#unorderedlist)
5. [Picture](#picture)

<a name="gametable"></a>
| Month    | Game                     |
| -------- | ----------------------   |
| January  | Persona 5                |
| February | Final Fantasy: Rebirth   |
| March    | Return of the Obra Din   |
| April    | Curse of the Golden Idol |

<a name="codeblock"></a>
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

## A subhead

<a name="blockquote"></a>
> This is part of a blockquote.
> This continues on the same line.
>
> This does not.

<a name="unorderedlist"></a>
* Unordered
* List
* of
* Length
* 5

<a name="picture"></a>
![sky-box](documentation-img-1.jpg)
