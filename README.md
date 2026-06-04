## Project  Sol-Reaper
An Ambitious attempt at a Rust-Analyzer like language server for Solidity. Very early development stage.
Best of both worlds: Incremental reparsing + Incremental computation enabled by tree-sitter + salsa.


To run:
```
cd lsp_server
cargo build
cd ../extension
npm install
npm run compile
```

Then open the project(extension/src/extension.ts) in VS Code and press `F5` to start the extension in debug mode.

Implemented Capabilities:
Hover 👍


