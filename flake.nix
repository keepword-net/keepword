{
  description = "Rust dev environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # The version CI uses, the same one rustup picks
        toolchain = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain;
        rustToolchain = pkgs.rust-bin.stable.${toolchain.channel}.default.override {
          extensions = [ "rust-src" "clippy" "rustfmt" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            pkg-config
            openssl
          ];

          # Helps build scripts find things
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "Rust dev shell ready: $(rustc --version)"
          '';
        };
      });
}
