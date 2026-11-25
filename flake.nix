{
  description = "Review diffs in the terminal and discuss them with your coding agent.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages = {
          lumiere =
            let
              manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
            in
            pkgs.rustPlatform.buildRustPackage {
              pname = manifest.name;
              version = manifest.version;

              cargoLock.lockFile = ./Cargo.lock;

              src = pkgs.lib.cleanSource ./.;

              nativeBuildInputs = [
                pkgs.pkg-config
              ];
              doCheck = false;
            };
          default = self.packages.${system}.lumiere;
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [
            pkgs.cargo
            pkgs.clippy
            pkgs.rust-analyzer
            pkgs.rustc
            pkgs.rustfmt
            pkgs.pkg-config
          ];
        };
      }
    )
    // {
      overlays.default = final: prev: {
        inherit (self.packages.${final.system}) lumiere;
      };
    };
}
