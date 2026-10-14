# External navigation integration

The retained MIT CUA browser read adapter now exposes
`BrowserReadEpisode::take_navigation_binding`. The adapter rechecks the exact
native process, endpoint, selected tab and access class. It retires its element
refs and closes read connections before returning the page target to Dojo's
separate compositor broker. This operation grants no mutation permission.

The method rejects consumer profiles, title-only correlation and changed active
page selection. `BoundNavigationEndpoint` contains the engine-produced native
identity, endpoint URL and page target. The tool caller supplies none of these.

The Niri final writer and Rust broker are new Dojo implementations informed by
CUA's CDP transport contract and the prior socket feasibility proof. They permit
one bounded cleartext loopback `Page.navigate` frame and report accepted effects
as unknown. Source notices remain in the retained CUA directory. Recompute the
machine-readable ledger with `vendor/cua/refresh-ledger.mjs` after integration.
