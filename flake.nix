{
  description = "LLZK interpreter";

  # Keep this revision aligned with llzk and llzk-sys in Cargo.lock. The
  # upstream overlays supply LLZK, LLVM/MLIR, and libclang as one set.
  inputs = {
    llzk-rs.url = "git+https://github.com/project-llzk/llzk-rs";

    llzk-pkgs.follows = "llzk-rs/llzk-pkgs";
    nixpkgs.follows = "llzk-pkgs/nixpkgs";
    flake-utils.follows = "llzk-pkgs/flake-utils";
    llzk-lib.follows = "llzk-rs/llzk-lib";
    release-helpers.follows = "llzk-rs/llzk-lib/release-helpers";
    rust-overlay.follows = "llzk-rs/rust-overlay";
  };

  # Custom colored bash prompt
  nixConfig.bash-prompt = "\\[\\e[0;32m\\][LLZK interpreter]\\[\\e[m\\] \\[\\e[38;5;244m\\]\\w\\[\\e[m\\] % ";

  outputs =
    {
      nixpkgs,
      flake-utils,
      llzk-pkgs,
      llzk-lib,
      llzk-rs,
      release-helpers,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (import rust-overlay)
            llzk-pkgs.overlays.default
            llzk-lib.overlays.default
            llzk-rs.overlays.default
            release-helpers.overlays.default
          ];
        };
        rust = pkgs.rust-bin.stable.latest.default;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rust;
          rustc = rust;
        };
        environment = pkgs.llzkSharedEnvironment;
      in
      {
        packages.default = rustPlatform.buildRustPackage (
          {
            pname = "llzk-interpreter";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = pkgs.lib.cleanSource ./.;
            nativeBuildInputs = environment.nativeBuildInputs;
            buildInputs = environment.buildInputs;
            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };
          }
          // environment.env
          // environment.pkgSettings
        );

        devShells.default = pkgs.mkShell (
          {
            nativeBuildInputs = environment.nativeBuildInputs;
            buildInputs = environment.devBuildInputs ++ [ rust ];
          }
          // environment.env
          // environment.devSettings
        );
      }
    );
}
