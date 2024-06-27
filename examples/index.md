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

## What's in a name?

A chimera is a mythical creature with two heads from different beasts, most commonly a
lion and a goat. It was the differing perspectives that suggested the name for this
project. It is a webserver that presents an alternative view depending on the kind of
document you are looking at.

![Chimera plate, Louvre](/home/assets/Chimera_Apulia_Louvre_K362.jpg)

## Example documents

* [Index (this document)](index.md)
* [Example 1](example1.md)
* [Example 2](example2.md)
* [Something](no-index/something.md)

## Other folders

* [Subfolder](subfolder/)
* [Gruber test suite](tests/)
