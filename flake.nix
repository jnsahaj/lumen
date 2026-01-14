{
  description = "A command-line tool that uses AI to streamline your git workflow - from generating commit messages to explaining complex changes.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    fenix,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {inherit system;};
      rustToolchain = fenix.packages.${system}.stable.withComponents [
        "cargo"
        "clippy"
        "rustc"
        "rustfmt"
        "rust-src"
      ];
      rust-analyzer = fenix.packages.${system}.rust-analyzer;
    in {
      packages = {
        lumen = let
          manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
        in
          pkgs.rustPlatform.buildRustPackage {
            pname = manifest.name;
            version = manifest.version;

            cargoLock.lockFile = ./Cargo.lock;

            src = pkgs.lib.cleanSource ./.;

            nativeBuildInputs = [pkgs.pkg-config pkgs.perl];
            buildInputs = [pkgs.openssl];
            doCheck = false;
          };
        default = self.packages.${system}.lumen;
      };

      devShells.default = pkgs.mkShell {
        nativeBuildInputs = [
          rustToolchain
          rust-analyzer
          pkgs.pkg-config
          pkgs.perl
        ];
        buildInputs = [
          pkgs.openssl
        ];
      };
    })
    // {
      overlays.default = final: prev: {
        inherit (self.packages.${final.system}) lumen;
      };
    };
}
