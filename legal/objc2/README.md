# objc2 license text

The crates from the objc2 repository (<https://github.com/madsmtm/objc2>), which Kinetra's macOS
build uses through Tauri and directly (`objc2`, `block2`, `dispatch2`, the `objc2-*` framework
crates), are MIT-licensed, or licensed as Zlib OR Apache-2.0 OR MIT, where Kinetra uses MIT.
The published crates don't include the license text; it lives at the repository's root.

`LICENSE-MIT.txt` is that repository's `LICENSE-MIT.txt` (main at 8a5abacd90d5, 2026-10-29).
`scripts/licenses.mjs` attaches it to the repository's crates that ship without a license file.
