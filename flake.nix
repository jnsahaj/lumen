{
  description = "A command-line tool that uses AI to streamline your git workflow - from generating commit messages to explaining complex changes, all without requiring an API key.";

  inputs = {
    nixpkgs.url = github:nixos/nixpkgs/nixos-unstable;
    flake-utils.url = github:numtide/flake-utils;
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages = {
        lumen = 
          let
            manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
          in
          pkgs.rustPlatform.buildRustPackage {
            pname = manifest.name;
            version = manifest.version;
          
            cargoLock.lockFile = ./Cargo.lock;
          
            src = pkgs.lib.cleanSource ./.;
          
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.openssl ];
          };
        default = self.packages.${system}.lumen;
      };
    })
    // {
      overlays.default = final: prev: {
        inherit (self.packages.${final.system}) lumen;
      };
    };
}
