# Todo

## Nginx notes

> Docroot is: /opt/homebrew/var/www
>
> The default port has been set in /opt/homebrew/etc/nginx/nginx.conf to 8080 so that 
nginx can run without sudo.
>
> nginx will load all files in /opt/homebrew/etc/nginx/servers/.
>
> To start nginx now and restart at login:
>  brew services start nginx
> Or, if you don't want/need a background service you can just run:
>  /opt/homebrew/opt/nginx/bin/nginx -g daemon\ off\;

**Start server in the background (add sudo for port 80):**

`nginx`

**Stop server:**

`nginx -s quit`

**Reload config**

`nginx -s reload`

**Conf files**

* /opt/homebrew/etc/nginx/nginx.conf
* /opt/homebrew/etc/nginx/servers/mkscript.conf

There can be any number of conf files in `/opt/homebrew/etc/nginx/servers/` directory. Changes and
additions require restarting nginx

## Markdown conversion

* Transformed document has no doctype, html, head, or body tags - opportunity for template system
* Also has no title

## Handlebars

* https://crates.io/crates/handlebars

## Ope, maybe not!

I took a bunch of notes last night on future work. But as I was working through these things,
I began to have doubts about the architectural direction of the static conversion flow. If files
link to each other, such as a document linking to an image, that relationship becomes difficult
to maintain if the destination folder is not the same as the source. It could be maintained if
we are cloning assets, but I don't want to do that.

I'm thinking instead now of making my own server where requests for .md files get routed through
a separate handler that does conversion on the fly (with potential caching). This way I don't
need to rewrite the contents of links on their way through or consider cloning resources. I serve
the source folders directly.

Joakim has pointed me to [axum](https://docs.rs/axum/latest/axum/), a Rust web server with
configurable content handlers that looks perfect. Checking this stuff in now so I have a record
of the notes and all before scrapping the existing code and starting over.

## Serving ordinary files

* https://docs.rs/tower-http/latest/tower_http/services/struct.ServeFile.html
* https://github.com/tokio-rs/axum/discussions/608
* https://users.rust-lang.org/t/stream-media-with-axum/108465/3

## Full text search

Joakim:

> for that, I would probably look into meillisearch or if you're feeling really cool: tantivy