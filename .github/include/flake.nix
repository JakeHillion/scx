{
  description = "Nix flake for the scx CI environment.";

  inputs = {
    nixpkgs.url = "github:JakeHillion/nixpkgs/virtme-ng";
    flake-utils.url = "github:numtide/flake-utils";

    nix-develop-gha.url = "github:nicknovitski/nix-develop";
    nix-develop-gha.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, nix-develop-gha, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ]
      (system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
        in
        {
          devShells =
            let
              common = with pkgs; [ git gnutar zstd ];
            in
            {
              update-kernels = pkgs.mkShell {
                buildInputs = with pkgs; common ++ [
                  gh
                  git
                  jq
                ];
              };

              build-kernel = pkgs.mkShell {
                buildInputs = with pkgs; common ++ [
                  bc
                  bison
                  cpio
                  elfutils
                  flex
                  git
                  jq
                  openssl
                  pahole
                  perl
                  virtme-ng
                  zlib
                ];
              };

              rust-tests = pkgs.mkShellNoCC {
                buildInputs = with pkgs; common ++ [
                  cargo
                  clang
                  clippy
                  elfutils
                  jq
                  llvmPackages.libclang
                  llvmPackages.libllvm
                  pkg-config
                  rustc
                  rustfmt
                  virtme-ng
                  zlib
                ];

                LIBCLANG_PATH = "${lib.getLib pkgs.llvmPackages.libclang}/lib";

                hardeningDisable = [
                  "stackprotector"
                  "zerocallusedregs"
                ];
              };
            };

          packages = {
            nix-develop-gha = nix-develop-gha.packages."${system}".default;

            kernels = builtins.mapAttrs
              (name: details: (pkgs.callPackage ./build-kernel.nix {
                inherit name;
                inherit (details) repo branch commitHash narHash;
                version = details.kernelVersion;
              }))
              (builtins.fromJSON (builtins.readFile ./../../kernel-versions.json));

            ci =
              pkgs.python3Packages.buildPythonApplication rec {
                pname = "ci";
                version = "git";

                pyproject = false;
                dontUnpack = true;

                propagatedBuildInputs = with pkgs; [
                  bash
                  binutils
                  cargo
                  clang
                  clippy
                  coreutils
                  git
                  gcc
                  gnugrep
                  gnumake
                  gnused
                  jq
                  llvmPackages.libclang
                  llvmPackages.libllvm
                  pkg-config
                  rustc
                  rustfmt
                  virtme-ng

                  elfutils.dev
                  zlib.dev
                  zstd.dev
                ];

                makeWrapperArgs = lib.lists.flatten [
                  [ "--set" "CC" "gcc" ]
                  [ "--set" "LD" "ld" ]

                  [ "--set" "BPF_CLANG" (lib.getExe pkgs.llvmPackages.clang) ]
                  [ "--set" "LIBCLANG_PATH" "${lib.getLib pkgs.llvmPackages.libclang}/lib" ]

                  [ "--set" "PKG_CONFIG_PATH" "${lib.makeSearchPath "lib/pkgconfig" propagatedBuildInputs}" ]

                  [ "--set" "RUSTFLAGS" "'-C relocation-model=pic -C link-args=-lelf -C link-args=-lz -C link-args=-lzstd -L /nix/store/7s00dr6z6qgm2hdn4jfmxryk59wz93jp-scx_cscheds-1.0.8-dev/libbpf/src'" ]

                  [ "--set" "NIX_BINTOOLS" pkgs.binutils ]
                  [ "--set" "NIX_BINTOOLS_WRAPPER_TARGET_HOST_x86_64_unknown_linux_gnu" "1" ]
                  [ "--set" "NIX_CC" pkgs.gcc ]
                  [ "--set" "NIX_CC_WRAPPER_TARGET_HOST_x86_64_unknown_linux_gnu" "1" ]
                  [ "--set" "NIX_PKG_CONFIG_WRAPPER_TARGET_HOST_x86_64_unknown_linux_gnu" "1" ]

                  [ "--set" "NIX_LDFLAGS" "'-rpath /data/users/jake/repos/scx-git/outputs/out/lib  -L/nix/store/awbfciyq3cjvw6x8wd8wdjy8z2qxm98n-elfutils-0.191/lib -L/nix/store/awbfciyq3cjvw6x8wd8wdjy8z2qxm98n-elfutils-0.191/lib -L/nix/store/vpg96mfr1jw5arlqg831i69g29v0sdb3-zlib-1.3.1/lib -L/nix/store/vpg96mfr1jw5arlqg831i69g29v0sdb3-zlib-1.3.1/lib -L/nix/store/6cf2yj12gf51jn5vdbdw01gmgvyj431s-zstd-1.5.6/lib -L/nix/store/6cf2yj12gf51jn5vdbdw01gmgvyj431s-zstd-1.5.6/lib'" ]
                ];

                installPhase = "install -Dm755 ${../include/ci.py} $out/bin/ci";
              };
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

                dependencies = with pkgs; [
                  bash
                  coreutils
                  git
                  gnumake
                  gnused
                  nix
                ];

                installPhase = "install -Dm755 ${../include/update-kernels.py} $out/bin/update-kernels";
              };
            in
            {
              type = "app";
              program = "${script}/bin/update-kernels";
            };
        };
      });
}

