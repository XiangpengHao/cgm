{
  description = "AiDEX X / GX-01S — Dioxus glucose web app, shared Rust core";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
      in
      {
        devShells.default =
          with pkgs;
          mkShell {
            packages = [
              # Dioxus / web toolchain
              dioxus-cli
              wasm-bindgen-cli_0_2_118
              binaryen
              tailwindcss_4
              nodejs

              # general
              pkg-config
              openssl
              just
              fd

              # Rust nightly with the wasm target.
              (rust-bin.selectLatestNightlyWith (
                toolchain:
                toolchain.default.override {
                  extensions = [
                    "rust-src"
                    "rust-analyzer"
                    "clippy"
                    "llvm-tools-preview"
                  ];
                  targets = [
                    "x86_64-unknown-linux-gnu"
                    "wasm32-unknown-unknown"
                  ];
                }
              ))
            ];

            # web-sys gates the Web Bluetooth bindings behind this cfg; it is also
            # set in .cargo/config.toml so plain `cargo`/`dx` builds pick it up.
            RUSTFLAGS = "--cfg=web_sys_unstable_apis";
          };
      }
    );
}
