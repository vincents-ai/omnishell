{
  description = "OmniShell: shrs + vincents-ai/llm + vincents-ai/gitoxide";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            (rust-bin.stable."1.88.0".default.override {
              extensions = [ "rust-src" "rust-analyzer" ];
            })
            pkg-config
            openssl
            cmake
            fontconfig
            zlib
          ];

          shellHook = ''
            echo "🦀 Entering OmniShell Development Environment"
            echo "Available Profiles:"
            echo "  cargo run -- --mode kids"
            echo "  cargo run -- --mode agent"
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "omnishell";
          version = "0.1.0";

          src = ./.;

          cargoHash = "sha256-/eUDxoH6YEfzl5lUWxxO5WWvHEFHlTCUf0BWuMIvm/E=";

          nativeBuildInputs = with pkgs; [ pkg-config cmake ];
          buildInputs = with pkgs; [ openssl fontconfig zlib ];

          env = {
            ZLIB_NO_PKG_CONFIG = "1";
          };

          # Integration tests need the built binary.
          # In nix checkPhase, the binary is at target/<profile>/omnishell.
          preCheck = ''
            export OMNISHELL_BIN="$(find target -name omnishell -type f | head -1)"
          '';
        };
      }
    );
}
