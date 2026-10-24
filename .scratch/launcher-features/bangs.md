# Bang list (draft)

Status: needs-info. Trigger, site, search URL with `{}` for the query. URLs are from memory and
have not been requested; check each before committing. The AI rows may only fill the input box.
Planned format: tracked TOML in the repo plus `~/.config/dojo/bangs.toml`, which wins on a clash.

Open: cuts and additions, triggers, per-bang home pages (a bare bang opens the site), default
web-search engine.

## Search engines
- `g` Google: google.com/search?q={}
- `ddg` DuckDuckGo: duckduckgo.com/?q={}
- `b` Bing: bing.com/search?q={}
- `brave` Brave Search: search.brave.com/search?q={}
- `kagi` Kagi: kagi.com/search?q={}
- `gi` Google Images: google.com/search?tbm=isch&q={}

## Dev
- `gh` GitHub: github.com/search?q={}
- `so` Stack Overflow: stackoverflow.com/search?q={}
- `mdn` MDN: developer.mozilla.org/en-US/search?q={}
- `crates` crates.io: crates.io/search?q={}
- `docsrs` docs.rs: docs.rs/releases/search?query={}
- `std` Rust std: doc.rust-lang.org/std/?search={}
- `npm`: npmjs.com/search?q={}
- `pypi`: pypi.org/search/?q={}
- `go` pkg.go.dev: pkg.go.dev/search?q={}
- `cpp` cppreference: en.cppreference.com/mwiki/index.php?search={}
- `hn` Hacker News: hn.algolia.com/?q={}

## Linux, Arch and Nix
- `aw` ArchWiki: wiki.archlinux.org/index.php?search={}
- `aur` AUR: aur.archlinux.org/packages?K={}
- `pkg` Arch packages: archlinux.org/packages/?q={}
- `man` Arch man pages: man.archlinux.org/search?q={}
- `nix` nixpkgs packages: search.nixos.org/packages?query={}
- `nixo` NixOS options: search.nixos.org/options?query={}
- `hm` Home Manager options: home-manager-options.extranix.com/?query={}
- `flat` Flathub: flathub.org/apps/search?q={}

## Reference
- `w` Wikipedia: en.wikipedia.org/w/index.php?search={}
- `wt` Wiktionary: en.wiktionary.org/w/index.php?search={}
- `wa` Wolfram Alpha: wolframalpha.com/input?i={}
- `tr` Google Translate: translate.google.com/?sl=auto&text={}
- `imdb`: imdb.com/find/?q={}

## Media and social
- `yt` YouTube: youtube.com/results?search_query={}
- `ytm` YouTube Music: music.youtube.com/search?q={}
- `sp` Spotify: open.spotify.com/search/{}
- `tw` Twitch: twitch.tv/search?term={}
- `r` Reddit: reddit.com/search/?q={}
- `x` X: x.com/search?q={}

## Maps and shopping
- `maps` Google Maps: google.com/maps/search/{}
- `osm` OpenStreetMap: openstreetmap.org/search?query={}
- `a` Amazon: amazon.com/s?k={}
- `ebay`: ebay.com/sch/i.html?_nkw={}

## AI
- `gpt` ChatGPT: chatgpt.com/?q={}
- `claude` Claude: claude.ai/new?q={}
- `pplx` Perplexity: perplexity.ai/search?q={}
