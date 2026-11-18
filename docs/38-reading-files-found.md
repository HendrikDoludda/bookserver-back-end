# Reading files — found

- `bookserver-front-end/lib/data/epub_book.dart` `chapterBody` · parses a flowing EPUB's XHTML with the HTML parser, which ignores `/>` on non-void tags: `<a id="page12"/>` swallows the rest of its paragraph and reopens around the next one, `<div class="x"/>` swallows the paragraph after it · page-marker anchors like that are common in EPUBs, so text can land inside the wrong element and take its styling
