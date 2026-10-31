# Contributing

Thanks for looking. Loki is a small project with a clear direction (see ROADMAP.md): self-hosted, local-first, nothing phones home.

- **Issues first.** For anything bigger than a typo, open an issue so we agree on the shape before you write code.
- **Build and test.** `npm install`, then `npm run build` for the front end; `cd src-tauri && cargo test --lib` for the Rust side. Both must pass.
- **Keep it personal-data free.** No real names, addresses, hosts or keys in code, tests, docs or screenshots. Use the placeholders you see in `config.example.toml` and the starter pack.
- **Profile over hard-coding.** Folder names, headings, labels and people belong in `profile.toml`, not in Rust or TypeScript. If you need a new one, add a profile field with a blank-vault default.
- **License.** By contributing you agree your work is released under the Apache License 2.0.
