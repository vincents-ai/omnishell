{
  description = "OmniShell: shrs + vincents-ai/llm + vincents-ai/gitoxide";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
            openssl
            cmake
            fontconfig
            zlib # Required by gitoxide
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
          
          # IMPORTANT: Run `nix run nixpkgs#nix-prefetch-git` or let it fail once
          # to acquire the combined cargoHash of your local code AND the github patches.
          cargoHash = pkgs.lib.fakeHash; 

          nativeBuildInputs = with pkgs; [ pkg-config cmake ];
          buildInputs = with pkgs; [ openssl fontconfig zlib ];
          
          # Fixes zlib linkage issues for pure rust git implementations
          env = {
            ZLIB_NO_PKG_CONFIG = "1";
          };
        };
      }
    );
}
