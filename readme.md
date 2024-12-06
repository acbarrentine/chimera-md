# Chimera-md

![screenshot](examples/assets/screenshot.jpg)

Chimera-md is a [Markdown](https://www.markdownguide.org/)-aware documentation server.

I have spent years developing a hard drive full of notes and documents written in the
Markdown text processing language, and while it is comparatively easy to view them as
intended in a special editor, most often I would wind up seeing them in plain text form.
I went looking for a server I could host that would serve up HTML-ified versions of
those documents transparently. Most of the tools I found were static site generators,
or had strong opinions about how the document should be formatted.

Chimera-md is my attempt to make a very simple web server that understands and can serve
a library of Markdown files (and supporting assets). It is a full web server
and can handle ordinary files but has special processing for Markdown files.

It is named after the mythical [chimera](https://en.wikipedia.org/wiki/Chimera_(mythology)),
a beast with at least two heads (and sometimes many more!). It represents the multiple
presentations of these files. Looked at one way, they are plain text. Looked at another,
they are fancy HTML.

## Goals

1. Transparently serve Markdown files
2. Built on top of an existing speedy [web server framework](https://docs.rs/axum/latest/axum/)
3. Nice-looking theme
4. Simple site navigation
5. Full text search
6. Syntax highlighting for code blocks
7. Updates automatically when files change

## Installation

The intended way to install is via [Docker](https://hub.docker.com/repository/docker/acbarrentine/chimera-md/general).
The Docker Compose file is probably the easiest way to get going. The version below should
get you going, but the [one on Github](https://github.com/acbarrentine/chimera-md/blob/main/compose.yaml)
will probably be more up to date.

```yaml
version: '3.8'
services:
  chimera-md:
    container_name: chimera-md
    platform: linux/amd64
    image: acbarrentine/chimera-md:latest
    ports:
      - "8080:8080"

    volumes:
      # /data/www is the web root -- Point this to your main documents folder
      - /usr/data/user1/documents:/data/www:ro

      # /data/search is where the full text indexer writes its work files
      # These are persisted to optimize server startup times
      - /docker/chimera-md/search:/data/search

      # Set a log dir
      - /docker/chimera-md/log:/data/log

      # You may want to map an images folder
      - /usr/data/user1/media:/data/www/images:ro

      # You can map in a favicon, if you'd like
      # - /usr/data/user1/images/logo.png:/data/www/favicon.ico:ro

      # Optional extra document directories can be added to the web root
      # - /usr/data/user1/notes:/data/www/notes:ro

      # Similarly, you can map additional image directories
      # - /usr/data/user1/icons:/data/www/images/icons:ro

      # If you use plug-ins, scripts for them will be loaded from /data/www/script
      # - /volume1/docker/chimera-md/script:/data/www/script:ro

    environment:
      # HEADS UP!
      #
      # Breaking change as of v 0.2.0!
      #
      # Configuration data has been moved to a site-based config file
      # Default is "chimera.toml"
      - CHIMERA_CONFIG_FILE=chimera.toml

    restart: unless-stopped
```

The config file in [TOML](https://toml.io/en/) format. Every field has a default, and the
Docker image provides a stub file, so the server should be able to start up provided you
have mapped reasonable values above. But there's a good chance you'll want to override
some of this content. Example with defaults included here:

```toml
# When used in Docker, these are probably the only fields you will want to change
site_title = "Chimera-md"
site_lang = "en"
generate_index = false

# But the rest of these are available if you want to tune things
document_root = "/data/www"
template_root = "/data/templates"
style_root = "/data/style"
icon_root = "/data/icon"
search_index_dir = "/data/search"
log_dir = "/data/log"
index_file = "index.md"
highlight_style = "an-old-hope"
log_level = "Info"
max_cache_size = 52428800
port = 8080

[menu]
# Items to appear in the navigation menu drop-down
# "label" = "URL"
"Home" = "/"

[redirects]
# You can list as many redirects here as you'd like
# "original URL" = "new URI"
# 
# URIs should be absolute, except the left one should not have a preceding /
#
# Example:
# "original-uri/" = "/home/path/to/new/uri.md"
```

Note that while Chimera-md is a web server, it is not trying to solve all problems a web server
can be asked. There is no CGI plug-in model. It doesn't handle SSL (TLS) certificates. If you
want authenticated traffic (and you probably do!), you should run it behind a reverse proxy like
[nginx](https://nginx.org/en/) or [Traefik](https://traefik.io/). 

## Managing documents

If you, like me, haven't been putting files in any semblance of order as they accumulate, you
might feel at a loss how to make a single folder that indexes them all. And that's what the web
root is — the starting point for serving up all your documents.

The basic organizational structure you'll want is an `index.md` file in each interesting directory
that points to the files you want to show and to the other folders you want to have reachable from
there. This doesn't have to be a tree structure, but that's a good way to start. If you don't have
an index file, but CHIMERA_GENERATE_INDEX is set to `true`, the server will make a best guess from
the contents of a folder (showing anything with a .md extension). That setting will also populate
a sidebar panel on markdown documents linking to discovered peers. Consider dressing up your index
documents with pictures to make them look sharp!

If you don't have a tree-like structure, you can use Docker volume mappings to invent one. As long
as mappings don't target the same exact Docker directory, they can overlap however you'd like. I
have a bunch of Synology "shared folder" mount points mapped into the one document folder, which
presents as a very serve-able tree.

![index](examples/assets/index.png)

In Docker, the left side of the mapping is the real, physical path; the right side is how it
presents in the container. Supposing you have a home directory that contains your main documents
folder, you could map it this way:

```yaml
    volumes:
      - /users/fancy/documents:/data/www
```

Then add additional folders in by creating sub-folder mappings:

```yaml
      - /users/wifey/budget:/data/www/budget
      - /users/kiddo/minecraft:/data/www/minecraft
```

You can also target specific files

```yaml
      - /users/fancy/automation/readme.md:/data/www/automation.md
```

Note that while the focus of Chimera-md is serving Markdown files, it is a fully capable web
server. Feel free to add other kinds of documents or assets to your pages. Only Markdown files
will be discovered by the full-text indexer, however.

A note about URLs: content inside your document root will be routed on the server through
a common root path called `/home` — this keeps them from potentially colliding with
internal route names controlled by the server. Supposing you had a folder structure like this:

`/users/me/taxes` → `/data/www/taxes`

The `index.md` file inside that folder would have an absolute URL of `/home/taxes/index.md`.
In general, though, relative URLs are sufficient. The root `index.md`, for example, would
refer to that document by `[Taxes](taxes/index.md)`. I find it comes up most often for image
references. A file such as `/data/www/images/logo.png` would route as `/home/images/logo.png`

## Customizing the server

There's a few different possible levels of customization points for the Chimera-md server. In
increasing complexity, these are:

1.  `/home/style/site.css`

    The Chimera-md header template references three CSS files. Two are termed internal
    use, and serve from the `/style` route. But the third, `/home/style/site.css` is intended
    for end-user customization. In there, you can adapt the site style as you see fit. There is
    a set of CSS color variables to make broad color changes easy. An example of this file is
    provided on [Github](https://github.com/acbarrentine/chimera-md/blob/main/examples/style/site.css).
    You can make your own `site.css` file within your document root without having to do any
    Docker mapping hijinks, and browsers will just immediately start using it. It does expect
    to be in a `style` subfolder, though. So if your document root was `/users/me/documents`,
    you would place it in `/users/me/documents/style/site.css`.

    You can also put a `favicon.ico` file in the web root and browsers will discover it on
    their own.

2.  Make your own additions to the header and footer. The base Chimera-md image header and
    footer include empty templates that are intended for site replacement. These are called
    `site-header.html` and `site-footer.html`. While these are looked file in the image's
    `/data/templates` directory, you can use volume mapping to overwrite them with your
    own. Content in here is echoed into the final output by a templating tool called
    [Tera](https://keats.github.io/tera/). Variables provided by the Chimera-md back-end
    are available.

```
        volumes:
        - /users/me/templates/site-header.html:/data/templates/site-header.html
        - /users/me/templates/site-footer.html:/data/templates/site-footer.html
```

3.  Override the built-in CSS files

    Using Docker mapping, you can paper over the built-in CSS files. These are located
    at `/data/style/skeleton.css` and `/data/style/chimera.css` (in Docker parlance).
    `skeleton.css` is an open source [CSS framework](http://getskeleton.com/), and I have
    only lightly customized it. `chimera.css` contains most of what you would consider
    the look of the site. You could start with my copies from Github and map them over
    with your adapted copies via Docker volume commands, like so:

    ```
        volumes:
        - /users/me/style/my-fancy.css:/data/style/chimera.css
    ```

    Note, however, that I probably won't be hands-off with these files as I make updates
    to the app, so grabbing new versions could cause instabilities with your local changes.

4.  Override the html template files

    The Markdown files Chimera-md serves are assembled from your content merged into
    html template files using a tool called [Tera](https://keats.github.io/tera/). While
    the template directory is inside the Docker container, you can use volume mapping to
    add templates of your own. You'll probably want to start by making copies of mine,
    [found here](https://github.com/acbarrentine/chimera-md/tree/main/templates), and
    map them to `/data/templates`. By default, a Markdown file renders with the
    markdown.html template file. But you can use the override mechanism to direct it to
    your own.
    
    ```markdown
    ---
    template: dialog.html
    ---

    # My Cool Index
    
    ## Generated index with customization
    ```

    I made an index.html template to render directories without index files, but I have
    found it useful as an override on an index.md file to let me customize and decorate
    the page without having to manually list out all the contents.

    ```markdown
    ---
    template: index.html
    ---

    # Index of /

    ![Logo](/home/media/logo.png)
    <!-- Files/Folders will show up below this point -->
    ```
    
    If you are _really_ intent on hacking up the server, you can use Docker volume mappings
    to replace my copies with your own. As with the style files, though, changes here have
    a decent chance of conflicting with changes I might make as I release new versions. I
    suggest using the partials system to isolate your changes, if possible. Look for
    `{% include %}` in the templates for examples.

## Release notes

### v0.3.8

* Trying out a couple things with this release. I have added a `cache-control` header to
  the responses to tell caching layers (like CloudFlare) that it's ok to remember things.
  The other item is that the back-end is providing a new variable `uri` to the Tera template
  engine. My intention with that is to combine it with a server name in the site-header.html
  to give a full canonical URL to each page. There's nothing using it in the default install,
  though, so you don't need to worry about it if you don't need. `uri` will start with a
  slash if you intend to use it.

```html
<link rel="canonical" href="https://canonical.server.name{{uri}}" />
```

### v0.3.8

* Fixing an ambiguous routing issue. /path/file.md should not be equivalent to
  /home/path/file.md

### v0.3.7

* Another attempt to get the requester's IP address, this time using the
  [X-Forwarded-For](https://en.wikipedia.org/wiki/X-Forwarded-For) header if you are behind
  a proxy

### v0.3.6

* Push the z-index on the drop-down menu; that seemed to fix the click-through problem
  I was getting. Some fine-tuning to the readme.

### v0.3.5

* Changes menu item HTML from `<a>` to `<button>`. The tap targets were too small for the
  links and on mobile, you'd often wind up clicking through to something underneath

### v0.3.4

* Adds a drop-down navigation menu on the right side of the breadcrumb bar. Entries in the
  menu are specified in the `chimera.toml` config file. Example:

```toml
[menu]
# Items to appear in the navigation menu drop-down
# "label" = "URL"
"Home" = "/home/index.md"
"Tests" = "/home/tests/index.md"
"Subfolder" = "/home/subfolder/index.md"
```

### v0.3.3

* Adds the client IP address to log lines

### v0.3.2

* Making the redirect mappings use permanent redirection. Fingers crossed they're working!

### v0.3.1

* The server now logs output both to the TTY and a log file. Referer and user-agent added
  to log statements

### v0.3.0 **BREAKING CHANGE!**

* Adding support for log files. This adds a new field to the config `log_dir`, set to
  `/data/log` by default. Log files rotate daily. You'll need to establish this path
  in the Docker container before it will work

### v0.2.2

* I hit an instance of a thread deadlocking which I believe to be a result of the cache
  failsafe operation I added. This changes the implementation to a message passed to the
  existing compaction thread, which should prevent the lock-in-lock situation I got it
  into before

### v0.2.1

* This contains an attempt to work around a sporadic fault in the directory watcher I'm
  using. I keep having instances where the notifies just seem to drop and the server no
  longer spots file changes. This change adds a backup system the looks at file modtimes
  before serving a file out of the cache. That doesn't cover all cases, but it should be
  a pretty good equivalent

### v0.2.0 **BREAKING CHANGE!**

* Added support for a table of redirects to the router. This is motivated by the needs
  of my [main website](https://www.dismal.ink/), which I am porting from Wordpress to
  Chimera-ma. I wanted to make some of the old, popular URLs map to their current
  locations
* Because environment variables aren't conducive to making a mapping and the configuration
  was already getting pretty hairy, I wound up moving the config settings over to toml.
  Going forward, you will need to make a site-based toml config file, called chimera.toml
  by default

### v0.1.25

* I am now promoting frontmatter variables through to Tera, the HTML template engine,
  which makes them available to the templates. The current use case is for opengraph
  tags. So, for example, if a post had frontmatter like this:

```yaml
---
Date: 2024-11-14
Title: Exmaple Index
Image: /media/fancy.jpg
Alt: A fancy piece of chocolate art
---
```

  That data would be available in the `__tera_context`, like so:

```html
{% if Image %}<meta property="og:image" content="{{Image}}" {% if Alt %}alt="{{Alt}}"{%endif%}>{% endif %}
{% if Excerpt -%}
<meta property="og:description" content="{{Excerpt}}">
<meta property="og:type" content="article">
{% else -%}
<meta property="og:type" content="website">
{% endif -%}
{% if site_lang %}<meta property="og:locale" content="{{site_lang}}">{% endif %}
{% if site_title %}<meta property="og:site_name" content="{{site_title}}">{% endif %}
{% if Title -%}
<meta property="og:title" content="{{Title}}">
{% else -%}
<meta property="og:title" content="{{title}}">
{% endif %}
```

### v0.1.24

* Fixed a bug in the frontmatter parser. Turns out, that's Yaml in there! I integrated
  the yaml-rust2 crate to parse it and feed it into the Tera vars as name-value pairs.
  Current implementation is a bit of a hack as it doesn't take into account the
  potentially nested nature of Yaml data. I'll come back around on that soon

### v0.1.23

* Ok, now I think I _have_ fixed the footer thing. When going through the array of server
  timing objects, actually use the one you're iterating on, not the end of the array
* Added a site-header.html template that's basically empty to be used by implementers
  who want to add additional `<head>` content, such as `<meta>`, `<script>`, or `<link>`
  tags

### v0.1.22

* Maybe fixed the footer thing?

### v0.1.21

* Added a customization point to the footer called site-footer.html. The included
  file is mostly empty and the intention is for people to override it with their
  docker mappings
* Updated various support crates
* Troubleshooting the timing bars Javascript widget, so there's an extra console.log()
  I am getting strange results from the site over the public internet, and I need to
  see what the data looks like

### v0.1.20

* Added gzip compression support

### v0.1.19

* Got rid of the plugins notion in favor of using overridden template files
* Added hook for signals (like ctrl-C) to shut the server down cleanly
* Two-column index layout

### v0.1.18

* Tried out a version of making a directory tree, rather than a single, flat folder
  for the sidebar and generated indices. Ultimately I decided I didn't like it and
  backed it out, but along the way I got to clean up a fair bit of the code
* Fixed a couple rendering issues with the sidebar

### v0.1.17

* Used `server-timing` http header, which is the only response header web clients allow
  Javascript to read, to render the response timing directly in the page.
* Figured out how to do my dialog plugin without jquery, using straight Javascript. As
  a result, I removed jquery from the html generator

### v0.1.16

* Fixed the search results template (it was closing the `<ol>` prematurely)
* Added a performance timing component
* Added the ability to specify an alternate Tera template to render against. The only
  use for this at the moment is letting you make an index.md file that doesn't need to
  list out the file contents. index.html uses the same file discovery component
  as the sidebar. To use the feature, in a Markdown document's header, use:

```markdown
---
template: index.html
---
```

### v0.1.15

* Fixed bug with heading jump-lists (one side was getting slugified and the other was not)
* Fixed the html generator's file watcher so it would dump the cache if an html file (Tera
  template) changed, instead of the old .hbs (Handlebars template) files

### v0.1.14

* Swapped out the Handlebars templating system for [Tera](https://keats.github.io/tera/).
  Tera is both faster and has more functionality
* Made the file names in the sidebar use stems (no extensions)

### v0.1.13

* Added little .svg icons to the sidebars
* Cleaned up the full text indexer's file discovery

## Non-Docker installation

You rebel! Well, I don't have precompiled binaries, so the easiest way to go
about getting a non-Docker install is to [clone the depot](https://github.com/acbarrentine/chimera-md)
and build it from source. It is a standard [Rust](https://www.rust-lang.org/) project
and should compile on just about any platform.

One interesting thing I noted while developing this. While running it locally prevents
the use of Docker directory mapping, I was able to use soft links to present a unified
folder structure under the document root. The server was able to navigate them just fine.
That does not appear to be the case in the Docker version, however.

`cargo run` will compile and run the project. As noted above, there are environment
variables for controlling the behavior, and more of them will be applicable here. You
can either use the environment, or specify them on the command line.

`bacon example` will compile and run the `examples` web content and keep watch on the
source directories for changes. However, at the moment, the paths in the config file
are required to be absolute, so the values you'll find in git will only work for me.
You'll need to either change the `example.toml` file or add your own
[bacon job](https://dystroy.org/bacon/#configuration).

## Arguments

The only command line argument is the path to the config TOML file. Call it with

```bash
chimera-md --config-file=example.toml
```

## Contributing

I'm pretty new to this whole open source world, but I welcome contributors and
feedback! Feel free to file [pull requests](https://github.com/acbarrentine/chimera-md/pulls) for new features
and [flag bugs](https://github.com/acbarrentine/chimera-md/issues). It takes a village!

## Roadmap

What does the future hold? It's hard to say. Some possible future directions include:

1. Progressive search results
2. User authentication and restricting content to particular users
3. Additional navigation customization options
4. Editing or uploading of Markdown files

It all depends on you. What do you want to see?

## Acknowledgements

* I want to call out [this video by Rainer Stropek](https://www.youtube.com/watch?v=y5W6PErCc2c) for the
very cogent guide to using Axum. It was a huge help.
* Icons come from the [Ionicons](https://ionic.io/ionicons) SVG library

## Libraries

Chimera-md uses the following open source libraries:

* [Skeleton CSS framework](http://getskeleton.com/)
* [Axum web framework](https://github.com/tokio-rs/axum)
* [Tera](https://keats.github.io/tera/)
* [pulldown-cmark](https://crates.io/crates/pulldown-cmark)
* [Tantivy](https://crates.io/crates/tantivy)
* [tokio](https://tokio.rs/)
* [tower-http](https://crates.io/crates/tower-http)
* [tracing](https://crates.io/crates/tracing)
* [serde](https://crates.io/crates/serde)
* [clap](https://crates.io/crates/clap)
* [regex](https://crates.io/crates/regex)
* [urlencoding](https://crates.io/crates/urlencoding)
* [walkdir](https://crates.io/crates/walkdir)
* [async-watcher](https://crates.io/crates/async-watcher)
* [toml](https://crates.io/crates/toml)
* [indexmap](https://crates.io/crates/indexmap)
* [slugify](https://crates.io/crates/slugify)
* [yaml-rust2](https://crates.io/crates/yaml-rust2)
* [toml](https://crates.io/crates/toml)

## License

This project is open sourced under the MIT [License](License.txt)
