{
  inputs = {
    naersk.url = "github:nix-community/naersk/master";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils, naersk }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
      in
      {
        defaultPackage = naersk-lib.buildPackage ./.;

        defaultApp = utils.lib.mkApp {
          drv = self.defaultPackage."${system}";
        };

        devShell = with pkgs; mkShell {
          buildInputs = [
            cargo
            clang
            fontconfig
            #freetype
            glib
            gtk4
            libclang
            pkgconfig
            pre-commit
            redis
            rustPackages.clippy
            rustc
            rustfmt
          ];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LIBCLANG_PATH = "${libclang.lib}/lib";
        };
      });
}
