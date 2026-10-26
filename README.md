<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/images/kinetra-logo-for-dark-bg.svg"><img alt="Kinetra" src="docs/images/kinetra-logo-for-light-bg.svg" width="320"></picture></p>

Kinetra is a free, open-source desktop app for broadcast graphics: lower thirds, tickers,
scoreboards, media and live data. Design graphics without code (or in HTML and CSS),
preview them, and take them to air from one window.

Graphics are output as browser sources for OBS, vMix, CasparCG and other tools, or
fullscreen to any connected screen. Kinetra runs on Windows 10 or later and macOS 11 or later
([what works on macOS](docs/MACOS.md)). Data can come from Google Sheets, JSON and CSV APIs,
or anything that can send an HTTP request.

Projects are plain folders of JSON, HTML and CSS, so they are portable and git-friendly.

Website: [kinetra.live](https://kinetra.live)

## Development

Requirements: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) (LTS) and
[pnpm](https://pnpm.io).

```
pnpm install
pnpm tauri dev
```

Checks to run before opening a pull request:

```
cargo fmt    --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml
pnpm check
pnpm test
```

More for contributors:

- [docs/MACOS.md](docs/MACOS.md): what works on macOS, and what doesn't yet
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): how the app, the output server and the renderer fit together
- [docs/TEMPLATE_FORMAT.md](docs/TEMPLATE_FORMAT.md): the template file format
- [docs/API.md](docs/API.md): the Kinetra cloud API the app talks to
- [docs/RELEASING.md](docs/RELEASING.md): how releases are built and published

## Contributing

Bug reports, ideas and pull requests are welcome. Please read
[CONTRIBUTING.md](CONTRIBUTING.md) first; contributors sign a short
[Contributor License Agreement](CLA.md) on their first pull request.

## License

Copyright © 2026 Rørvik Medieteknikk. Kinetra is licensed under the
[GNU Affero General Public License v3.0](LICENSE), with an
[additional permission](LICENSE-EXCEPTION.md) for combining it with the NDI® Runtime, which
Kinetra uses but doesn't include ([NDI® terms](legal/ndi/NOTICE.md)). NDI® is a registered
trademark of Vizrt NDI AB.

Kinetra and the Kinetra logo are trademarks of Rørvik Medieteknikk and are not covered by
the license.
