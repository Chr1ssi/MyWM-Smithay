{
  description = "mywm – a Smithay-based tiling Wayland compositor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    mywm-shell = {
      url = "github:Chr1ssi/mywm-shell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, mywm-shell, ... }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      packageFor = pkgs: pkgs.callPackage ./nix/package.nix { shellSrc = mywm-shell; };
      settingsFor = pkgs: pkgs.callPackage ./nix/settings.nix { };
    in
    {
      packages = forAllSystems (system:
        let package = packageFor nixpkgs.legacyPackages.${system};
        in {
          default = package;
          mywm = package;
          mywm-settings = settingsFor nixpkgs.legacyPackages.${system};
        });

      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in {
          # Everything `cargo build`, `cargo test` and `cargo clippy` need, plus the runtime
          # libraries so that the nested compositor (`cargo run`) finds EGL, xkbcommon and libinput.
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default self.packages.${system}.mywm-settings ];
            packages = with pkgs; [ clippy rustfmt rust-analyzer ];
            # bindgen (pipewire-sys) needs libclang.
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
              wayland
              libxkbcommon
              libinput
              libglvnd
              libgbm
              libdrm
              pipewire
              libx11
              libxcursor
              libxrandr
              libxi
            ]);
          };
        });

      overlays.default = final: _prev: {
        mywm = packageFor final;
        mywm-settings = settingsFor final;
      };

      nixosModules.default = import ./nix/module.nix { inherit self; };
      nixosModules.mywm = self.nixosModules.default;
    };
}
