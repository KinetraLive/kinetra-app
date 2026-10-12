# Contributing to Kinetra

Thanks for wanting to help! Kinetra is maintained by a small team (right now, mostly one person), so issues and pull requests are reviewed about once a week. Please be patient, and thanks for understanding.

## Ways to contribute

- Report a bug: open an issue using the bug report form.
- Suggest a feature: open an issue using the feature request form, and describe the problem you're trying to solve, not only the solution.
- Improve the docs: typos, unclear explanations and missing steps are all welcome fixes.
- Contribute code: see below.
- Ask a question or share your setup: use Discord (https://discord.gg/nK4JaEh3) rather than issues.

## Reporting bugs

A good bug report includes:

- the Kinetra version and channel (stable or Canary), and your operating system;
- what you did, what you expected, and what happened instead;
- steps to reproduce it, if you can;
- logs or screenshots, if relevant (remove anything private first).

Please search existing issues before opening a new one.

## Security issues

Don't report security problems in public issues. Use GitHub's private vulnerability reporting (the "Report a vulnerability" button on the Security tab) or email support@kinetra.live. See SECURITY.md.

## Contributing code

1. Open an issue first for anything bigger than a small fix, so we can agree on the approach before you spend time on it. Pull requests for features that weren't discussed may be declined.
2. Fork the repo and create a branch from main.
3. Keep pull requests small and focused: one fix or feature per pull request.
4. Add or update tests for what you change.
5. Run the checks before opening the pull request:

   ```
   cargo fmt    --manifest-path src-tauri/Cargo.toml
   cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
   cargo test   --manifest-path src-tauri/Cargo.toml
   pnpm check
   pnpm test
   ```

6. Describe what and why in the pull request, and link the related issue.

### Guidelines

- Graphics on air must never stop or stutter because of a change. Anything that touches the renderer, outputs or Control needs extra care and testing.
- Keep the renderer small: it has a strict size budget.
- Follow the existing code style; cargo fmt and the checks above enforce most of it.
- Use short, plain commit messages that say what changed.

## Contributor License Agreement

Before your first pull request can be merged, you need to sign our Contributor License Agreement (CLA.md). A bot will comment on your pull request with instructions; signing takes a few seconds and covers all your future contributions.

Why we ask for it: you keep the copyright to your work, but the CLA lets Rørvik Medieteknikk offer Kinetra under other licenses too, for example commercial licenses for companies that can't use AGPL software. That helps fund Kinetra's development. The open-source version of Kinetra always stays available under the AGPL-3.0.

## License

By contributing, you agree that your contributions are covered by the CLA (CLA.md) and are distributed as part of Kinetra under the GNU Affero General Public License v3.0 (LICENSE).

Kinetra and the Kinetra logo are trademarks of Rørvik Medieteknikk and are not covered by the license.

## Be kind

Be respectful and constructive with everyone in issues, pull requests and on Discord. Harassment or abuse isn't tolerated.
