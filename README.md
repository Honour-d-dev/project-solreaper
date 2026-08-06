<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="logo-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="logo.svg">
    <img src="logo.svg" alt="SolRepr logo" width="320" />
  </picture>
</p>

<h1 align="center" style="margin-bottom: 0;">SolRepr</h1>

[![Rust](https://img.shields.io/badge/rust-000000?style=flat-square&logo=rust)](https://www.rust-lang.org/) [![Solidity](https://img.shields.io/badge/solidity-363636?style=flat-square&logo=solidity)](https://soliditylang.org/) [![Tree-sitter](https://img.shields.io/badge/tree--sitter-000000?style=flat-square&logo=tree-sitter)](https://tree-sitter.github.io/) [![License](https://img.shields.io/github/license/Honour-d-dev/SolReapr?style=flat-square)](LICENSE) [![Stars](https://img.shields.io/github/stars/Honour-d-dev/SolReapr?style=flat-square)](https://github.com/Honour-d-dev/SolReapr)
<p align="center" style="margin-top: -12px;">
  A semantic repr. analyzer and language server for Solidity, built in Rust.
</p>

<p align="center">
  A Rust-Analyzer-like language server for Solidity. Very early development stage.<br />
  Best of both worlds: Incremental reparsing + incremental recomputation leveraging tree-sitter + salsa.
</p>

---

## Getting Started

To run:

```bash
cd lsp_server
cargo build
cd ../extension
npm install
npm run compile
```

Then open the project (`extension/src/extension.ts`) in VS Code and press `F5` to start the extension in debug mode.

## Capabilities

- Hover 👍
- Goto definition 👍