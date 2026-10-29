# Kept license texts

Some dependencies are published without their license text: it lives only in their source
repository, or nowhere. The texts here fill those gaps in the third-party licenses dialog.
`scripts/licenses.mjs` (`KEPT_LICENSES`) attaches each file to the packages from the listed
repository that ship without a license file of their own.

| File | Packages | Source |
|---|---|---|
| `objc2-MIT.txt` | the objc2 crates (`objc2`, `block2`, `dispatch2`, `objc2-*`; MIT, or Zlib OR Apache-2.0 OR MIT where Kinetra uses MIT) | github.com/madsmtm/objc2 `LICENSE-MIT.txt`, main at 8a5abacd90d5 |
| `polka-MIT.txt` | `@polka/url` | github.com/lukeed/polka `license`, at c36cf8b5d1a5 |
| `sirv-MIT.txt` | `sirv` | github.com/lukeed/sirv `license`, at 1135207e92c4 |
| `rust-alloc-no-stdlib-BSD-3-Clause.txt` | `alloc-stdlib` | github.com/dropbox/rust-alloc-no-stdlib `LICENSE`, at 0a81fd6928ea |
| `defmt-MIT.txt` | `defmt-parser` (MIT OR Apache-2.0; Kinetra uses MIT) | github.com/knurling-rs/defmt `LICENSE-MIT`, at 6b791b84a974 |
| `webview2-rs-MIT.txt` | `webview2-com`, `webview2-com-macros`, `webview2-com-sys` | github.com/wravery/webview2-rs `LICENSE`, at edc2caf88617 |
| `MPL-2.0.txt` | `selectors` (MPL-2.0; its files point to mozilla.org/MPL/2.0, the repository has no license file) | mozilla.org/media/MPL/2.0/index.txt |
| `rich-harris-MIT.txt` | `is-reference`, `locate-character` | Neither the packages nor their repositories have a license text, only "MIT" and the author Rich Harris: SPDX's MIT text (github.com/spdx/license-list-data `text/MIT.txt`, at 31846481de7e) with "Copyright (c) Rich Harris". |

All texts fetched 2026-10-29. When a package starts shipping its own license file, that file is
used instead (the generator only fills gaps).
