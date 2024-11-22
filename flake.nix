{
  description = "Sched_ext Schedulers and Tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , ...
    }@inputs:
    flake-utils.lib.eachSystem (with flake-utils.lib.system; [ x86_64-linux aarch64-linux ])
      (
        system:
        let
          defaultLlvmVersion = 18;
          pkgs = import nixpkgs { inherit system; };

          mkScxPackage =
            llvmVersion:
            let
              llvmPackages = pkgs."llvmPackages_${toString llvmVersion}";
            in
            llvmPackages.stdenv.mkDerivation rec {
              name = "scx";

              src = self;

              nativeBuildInputs =
                (with llvmPackages; [
                  clang
                ]) ++ (with pkgs; [
                  bash
                  gnused

                  meson
                  ninja
                  pkg-config
                ]);
              buildInputs = with pkgs; [
                elfutils
                zlib
              ];

              enableParallelBuilding = true;
            };

          # Define lambda that returns a devShell derivation with extra test-required packages
          # given the scx package derivation as input
          mkScxDevShell =
            pkg:
              with pkgs;
              pkgs.mkShell {
                buildInputs = [
                ] ++ pkg.nativeBuildInputs ++ pkg.buildInputs;
              };
        in
        {
          # Define package set
          packages = rec {
            default = self.packages.${system}."scx-llvm${toString defaultLlvmVersion}";

            scx-llvm18 = mkScxPackage 18;
          };

          # devShells = rec {
          #   default = self.devShells.${system}."scx-llvm${toString defaultLlvmVersion}";

          #   scx-llvm18 = mkScxDevShell self.packages.${system}.scx-llvm18;
          # };
        }
      )
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
