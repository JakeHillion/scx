{
  description = "Nix flake for the scx CI environment.";

  inputs = {
    nixpkgs.url = "github:JakeHillion/nixpkgs/virtme-ng";
    flake-utils.url = "github:numtide/flake-utils";

    "kernel_sched_ext_for-next".url = "git+https://git.kernel.org/pub/scm/linux/kernel/git/tj/sched_ext.git?ref=for-next";
    "kernel_bpf-next".url = "git+https://git.kernel.org/pub/scm/linux/kernel/git/bpf/bpf-next.git";
    "kernel_linux-rolling-stable".url = "git+https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git?ref=linux-rolling-stable";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ]
      (system: {
        packages = {
          kernels = let
            mkKernel = inp: {
              
            };
          in {
            "sched_ext/for-next" = mkKernel "sched_ext/for-next";
          };
        };

        devShells =
          let
            pkgs = import nixpkgs { inherit system; };
            common = with pkgs; [ gnutar zstd ];
          in
          {
            build-kernel = pkgs.mkShell {
              buildInputs = with pkgs; common ++ [
                bc
                bison
                cpio
                elfutils
                flex
                git
                openssl
                pahole
                perl
                virtme-ng
                zlib
              ];
            };
          };
      }) // flake-utils.lib.eachDefaultSystem (system: {
      formatter = nixpkgs.legacyPackages.${system}.nixpkgs-fmt;
    });
}

