{
  description = "Redlib: Private front-end for Reddit";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        rustToolchain = pkgs.rust-bin.stable.latest.default;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        src = lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (lib.hasInfix "/templates/" path) ||
            (lib.hasInfix "/static/" path) ||
            (craneLib.filterCargoSources path type);
        };

        redlib = with pkgs; craneLib.buildPackage {
          inherit src;
          strictDeps = true;
          doCheck = false;

          nativeBuildInputs = [
            git
            cmake
            clang
          ];

          LIBCLANG_PATH = "${libclang.lib}/lib";
        };
      in
      {
        checks = {
          my-crate = redlib;
        };

        packages.default = redlib;
        packages.docker = pkgs.dockerTools.buildImage {
          name = "quay.io/redlib/redlib";
          tag = "latest";
          created = "now";
          copyToRoot = with pkgs.dockerTools; [ caCertificates fakeNss ];
          config.Cmd = "${redlib}/bin/redlib";
        };
      });
}
