{
  description = "sched-ext schedulers for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }@inputs: (
    flake-utils.lib.eachSystem
      (with flake-utils.lib.system; [
        x86_64-linux
        aarch64-darwin
      ])
      (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = let
          llvmPackages = pkgs.llvmPackages_18;
        in pkgs.mkShell {
          buildInputs = with pkgs; [
            llvmPackages.clang

            cargo
            rustfmt

            elfutils
            gnumake
            pkg-config
            zlib
          ];
          LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

          # needed to compile BPF
          hardeningDisable = [
            "stackprotector"
            "zerocallusedregs"
          ];
        };
      })
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        formatter = pkgs.nixpkgs-fmt;
      }
    )
  );
}
