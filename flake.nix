{
  description = "development shell for flock";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    utils,
  }:
    utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages."${system}";
    in {
      devShell = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [rustup cargo-deny cargo-outdated sqlite rlwrap];
      };
    });
}
