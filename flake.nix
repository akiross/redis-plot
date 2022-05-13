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
        show_rle_snap = pkgs.writers.writePython3Bin
          "show_rle_snap"
          { libraries = with pkgs.python3Packages; [ matplotlib numpy ]; }
          ''
            # This is a quick visualizer for RLE-encoded images in JSON.
            # Run it passing the files in tests/snapshots

            import numpy as np
            from matplotlib import pyplot as plt
            import json
            import argparse
            from pathlib import Path


            def show(args):
                assert args.path.exists()
                with open(args.path) as fp:
                    # Skip header
                    c = 0
                    while c < 2:
                        if fp.readline().strip() == "---":
                            c += 1
                    # Read json data
                    data = json.load(fp)

                # RLE to list
                seq = []
                for e in data["rle_data"]:
                    seq.extend([e[1]] * e[0])
                w, h = data["width"], data["height"]
                img = np.asarray(seq).reshape((h, w, 3))
                plt.imshow(img)
                plt.show()


            def main():
                parser = argparse.ArgumentParser()
                parser.add_argument(
                  "path",
                  type=Path,
                  help="Path of the JSON file to load"
                )
                args = parser.parse_args()
                show(args)


            if __name__ == "__main__":
                main()
          '';

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
            glib
            gtk4
            libclang
            pkgconfig
            pre-commit
            redis
            rustPackages.clippy
            rustc
            rustfmt
            show_rle_snap
          ];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LIBCLANG_PATH = "${libclang.lib}/lib";
          APPEND_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [
            libGL
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
          ];
          shellHook = ''
            export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$APPEND_LIBRARY_PATH"
          '';
        };
      });
}
