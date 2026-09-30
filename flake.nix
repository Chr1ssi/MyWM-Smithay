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
    in
    {
      packages = forAllSystems (system:
        let package = packageFor nixpkgs.legacyPackages.${system};
        in {
          default = package;
          mywm = package;
        });

      overlays.default = final: _prev: { mywm = packageFor final; };

      nixosModules.default = import ./nix/module.nix { inherit self; };
      nixosModules.mywm = self.nixosModules.default;
    };
}
