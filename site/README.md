# weep.cc

The informational site and the manual for WEEP, built to static files.

```
npm install     # one dependency: marked
npm run build   # -> dist/weep
npm run serve   # build, then serve it on http://localhost:4321/
npm run watch   # the same, rebuilding on every change
```

From the repository root, `just site` and `just site-serve` do the same. On a Mac
without node, `nix-shell -p nodejs --run 'just site'`.

The build exits 1 if any internal link or in-page anchor does not resolve, so a
cross-reference into the manual cannot rot unnoticed.

## Why the builder is vendored

`build.mjs`, `lib/` and `shared/` are a copy of the builder in the `ragtux.com`
repository, by way of Scoria's `site/`, which also builds `ragtux.com`,
`lapstack.ragtux.com` and `scoria.cc`. They are kept **byte-identical on
purpose**, so a fix made on any side can be copied across whole rather than
merged, with two deliberate exceptions:

- `package.json` names this copy and defaults its scripts to the one site.
- `lib/highlight.mjs` highlights `weep` as a shell command, where Scoria's copy
  knows `scoria`.

Hence the slightly redundant `site/sites/weep/`.

## Layout

```
build.mjs              routes, assets, search index, sitemap, link check
lib/markdown.mjs       front matter, marked renderer (anchors, callouts, TOC)
lib/highlight.mjs      dependency-free highlighter (shell, rust, json, lua, toml)
lib/template.mjs       the page shell: top bar, sidebar, on-this-page, pager, footer
shared/                copied into /assets — site.css, site.js, Fira fonts
sites/weep/site.json   host, nav, sidebar order, footer, the CTA button
sites/weep/content/    the pages; .md is rendered, .html is passed through
sites/weep/assets/     logo.svg, favicon.svg, weep.css and Inter, over shared/
sites/weep/assets/img/app/  the README's screenshots, re-cut by `just site-media`
dist/weep/             the output to deploy (gitignored)
```

A page at `content/docs/tools.md` is served at `/docs/tools/`. Front matter
takes `title`, `description`, `layout` (`doc` or `page`) and `noindex`; pages
under `/docs` default to the `doc` layout. The sidebar order lives in
`site.json`, and the prev/next pager follows it, so a new page has to be added
there to be reachable.

## The look

`shared/site.css` is the vendored stylesheet and is not edited here. Everything
that makes weep.cc look like WEEP is in `sites/weep/assets/weep.css`, which
began as scoria.cc's and keeps its shape — Inter, hairlines instead of panels,
mono micro-labels, one loud action per page — with Fleet's ink: Fleet Dark's
`background.primary` and `text.primary` as the page, and Fleet's focus blue
(`border.focused`, `#2A7DEB`) as the accent. The comment at the top of the file
gives the contrast of each text colour.

The Inter faces in `assets/fonts/` are the subset Scoria's site cut from the
same Inter release the app bundles; `LICENSE-Inter.txt` ships beside them.

## The marks

`assets/logo.svg` is the boxed block wordmark of the original WEEP.CC logo
(`weep.cc/` in the old bouncing-logo page), redrawn as plain polygons so it
needs no font: black ink on a white plate, which reads on both themes.
`assets/favicon.svg` is its W alone on the same plate, because the whole word
is unreadable at 16 px.

## Deploying

`dist/weep` is plain static files. `404.html`, `robots.txt`, `sitemap.xml` and
`search-index.json` are generated; search is client-side against that index,
and there is no server. Serve it from the web root of weep.cc with the host
answering unknown paths with `404.html`.
