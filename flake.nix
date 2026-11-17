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

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "x86_64-unknown-linux-musl" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;


        src = lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (lib.hasInfix "/templates/" path) ||
            (lib.hasInfix "/static/" path) ||
            (craneLib.filterCargoSources path type);
        };

        muslCc = pkgs.pkgsMusl.stdenv.cc;

        buildInputs = with pkgs; [
          git
          cmake
          gnumake
          perl
          go
        ];

        redlib = craneLib.buildPackage {
          inherit src;
          strictDeps = true;
          doCheck = false;

          nativeBuildInputs = buildInputs;

          # btls-sys compiles BoringSSL via cmake for the musl target; the
          # default (glibc) gcc cannot link `-static`, so hand it musl gcc.
          CC_x86_64_unknown_linux_musl = "${muslCc}/bin/cc";
          CXX_x86_64_unknown_linux_musl = "${muslCc}/bin/c++";
          AS_x86_64_unknown_linux_musl = "${muslCc}/bin/cc";

          # bindgen (btls-sys) needs libclang at build-script time, and must
          # parse musl target headers, not the glibc host's.
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.pkgsMusl.musl.dev}/include";

          # final link must use musl gcc so libstdc++/libc resolve to musl
          # symbols (glibc libstdc++.a references __isoc23_strtoul, which musl
          # does not have).
          CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "${muslCc}/bin/cc";

          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static -L ${pkgs.pkgsMusl.gcc.cc}/lib";
        };
      in
      {
        checks = {
          my-crate = redlib;
        };

        devShells.default = craneLib.devShell {
          packages = buildInputs;
        };

        packages.default = redlib;
        packages.docker = pkgs.dockerTools.buildImage {
          name = "ghcr.io/nachtalb/redlib";
          tag = "latest";
          created = "now";
          copyToRoot = with pkgs.dockerTools; [ caCertificates fakeNss ];
          config.Cmd = "${redlib}/bin/redlib";
        };
      });
}
