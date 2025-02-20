{
  description = "Nix flake for the scx CI environment.";

  inputs = {
    nixpkgs.url = "github:JakeHillion/nixpkgs/virtme-ng";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ]
      (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          packages = {
            kernels = builtins.mapAttrs
              (name: details: (pkgs.callPackage ./build-kernel.nix {
                inherit name;
                inherit (details) repo branch hash narHash;
                version = details.kernelVersion;
              }))
              (builtins.fromJSON (builtins.readFile ./kernel-versions.json));
          };
        }) // flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        formatter = pkgs.nixpkgs-fmt;

        apps = {
          update-kernels =
            let
              script = pkgs.python3Packages.buildPythonApplication {
                pname = "update-kernels";
                version = "git";

                pyproject = false;
                dontUnpack = true;

                buildDependencies = with pkgs; [ gnumake ];

                installPhase = "install -Dm755 ${./update-kernels.py} $out/bin/update-kernels";
              };
            in
            {
              type = "app";
              program = "${script}/bin/update-kernels";
            };
        };
      });
}

