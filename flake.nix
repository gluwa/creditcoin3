{
  description = "cc3-next dev env and commands";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    crane,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      overlays = [(import rust-overlay)];
      pkgs = import nixpkgs {
        inherit system overlays;
      };

      # ====================================================================== #
      #                               RUST SETUP                               #
      # ====================================================================== #

      rustToolchain = builtins.readFile ./rust-toolchain.toml;
      rustChannel = builtins.fromTOML rustToolchain;
      rustVersion = rustChannel.toolchain.channel;

      rust = pkgs.rust-bin.stable.${rustVersion}.default.override {
        extensions = ["rust-src"];
        targets = [
          "x86_64-unknown-linux-musl"
          "wasm32-unknown-unknown"
        ];
      };

      # Use musl for static compilation
      pkgsMusl = pkgs.pkgsCross.musl64;
      craneLib = (crane.mkLib pkgsMusl).overrideToolchain (p: rust);

      # Source filtering - include all files needed for the build
      # We use cleanCargoSource with additional filters
      src = let
        # Filter to include Cargo files, Rust source, and other necessary files
        cargoFilter = path: _type: builtins.match ".*Cargo\\.toml$|.*Cargo\\.lock$" path != null;
        rustFilter = path: _type: builtins.match ".*\\.rs$" path != null;
        # Include YAML configs that might be needed
        yamlFilter = path: _type: builtins.match ".*\\.yaml$|.*\\.yml$" path != null;
        # Include .scale files for subxt metadata (needed by cc-client)
        scaleFilter = path: _type: builtins.match ".*\\.scale$" path != null;
        # Include .json files for ABI definitions (needed by eth crate's sol! macro)
        jsonFilter = path: _type: builtins.match ".*\\.json$" path != null;
        # Include .html files for doc enhancements (needed by the attestor crate)
        htmlFilter = path: _type: builtins.match ".*\\.html$" path != null;
        # Include symlinks (needed for contracts/block_prover.json -> precompiles/metadata/abi/)
        symlinkFilter = _path: type: type == "symlink";
        # Combine filters
        combinedFilter = path: type:
          (cargoFilter path type) || (rustFilter path type) || (yamlFilter path type) || (scaleFilter path type) || (jsonFilter path type) || (htmlFilter path type) || (symlinkFilter path type) || (craneLib.filterCargoSources path type);
      in
        pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = combinedFilter;
        };

      # ====================================================================== #
      #                               BUILD SETUP                              #
      # ====================================================================== #

      nativeBuildInputs = with pkgs; [
        perl
        pkg-config
        protobuf
        clang
      ];

      sccacheEnv = {
        RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
        SCCACHE_DIR = "/tmp/sccache"; # Shared cache directory
        SCCACHE_CACHE_SIZE = "50G";
        SCCACHE_SERVER_UDS = "/tmp/sccache-server.sock";
      };

      buildEnv = {
        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
        OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
        OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
      };

      commonArgs = {
        inherit src;
        strictDeps = true;
        nativeBuildInputs = nativeBuildInputs ++ [pkgsMusl.stdenv.cc];

        buildInputs = [];

        CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
        CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static -C linker=${pkgsMusl.stdenv.cc.targetPrefix}cc";

        SKIP_WASM_BUILD = "1";
        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        PROTOC = "${pkgs.protobuf}/bin/protoc";

        # Let openssl-sys build OpenSSL from source (vendored)
        OPENSSL_STATIC = "1";
      };

      # ====================================================================== #
      #                               COMPILATION                              #
      # ====================================================================== #

      # Static dependencies (cached between builds)
      cargoArtifacts = craneLib.buildDepsOnly (commonArgs
        // {
          pname = "cargo-deps";
          cargoExtraArgs = "--package attestor --package attestor_zombienet --locked";
        });

      # attestor
      attestor = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          pname = "attestor";
          cargoExtraArgs = "--package attestor --locked";
          doCheck = false;
          meta.mainProgram = "attestor";
        });

      # zombienet
      zombienet = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          pname = "attestor-zombienet";
          cargoExtraArgs = "--package attestor_zombienet --locked";
          doCheck = false;
          # Binary name uses underscore, not hyphen
          meta.mainProgram = "attestor_zombienet";
        });

      # ====================================================================== #
      #                                 SCRIPTS                                #
      # ====================================================================== #

      experimental = "--extra-experimental-features nix-command --extra-experimental-features flakes";
      shebang = "#!/usr/bin/env -S nix develop ${experimental} .#default -c bash";

      script_help = pkgs.writeShellScriptBin "help" ''
        exec ${pkgs.busybox}/bin/cat <<EOF
        cc3-next dev env and commands
        =============================

        USAGE
            nix run .#<app> [-- <args>]

        APPS
          Info:
            help                Show this help message (default)

          Runtime:
            attestor            Run the attestor binary (via cargo, in devshell)
                                Passes additional arguments to the attestor.

            zombienet           Launch a local attestor zombienet with 3 nodes
                                Preconfigured with:
                                  • eth-url:  ws://localhost:8545
                                  • cc3-url:  ws://localhost:9944
                                  • funding:  //Alice
                                  • config:   ./attestor/config.yaml

            node                Run creditcoin3-node in dev mode (fast-runtime)
                                Starts a temporary local chain with debug logging.

            anvil               Run Anvil (local Ethereum testnet)
                                Configured with 6-second block time.

          Development:
            check               Run 'cargo check --tests --release'
            clippy              Run 'cargo clippy --tests --release'
            test                Run 'cargo test --release'

        EXAMPLES
            nix run                         # Show this help
            nix run .#node                  # Start local creditcoin3 node
            nix run .#anvil                 # Start local Ethereum testnet
            nix run .#zombienet             # Start 3-node attestor network
            nix run .#attestor -- --help    # Show attestor CLI help
            nix run .#check                 # Quick compilation check
            nix run .#test -- -p attestor   # Run tests for attestor package

        PACKAGES
            nix build .#attestor            # Build static attestor binary
            nix build .#zombienet           # Build static zombienet binary

        DEVSHELL
            nix develop                     # Enter development shell with all tools
                                            # Includes: rust, protobuf, foundry, nodejs,
                                            # rocksdb, openssl, sccache, and more.
        EOF
      '';

      script_anvil = pkgs.writeShellScriptBin "anvil" ''
        exec ${pkgs.foundry}/bin/anvil --block-time 1 "$@"
      '';

      script_attestor = pkgs.writeScriptBin "attestor" ''
        ${shebang}
        cargo run --release --bin attestor -- "$@"
      '';

      script_zombienet = pkgs.writeScriptBin "zombienet" ''
        ${shebang}
        cargo run --release --bin attestor_zombienet -- \
          -n 3                                          \
          --bin=${script_attestor}/bin/attestor         \
          --eth-url=ws://localhost:8545                 \
          --cc3-url=ws://localhost:9944                 \
          --funding-address='//Alice'                   \
          --config=./attestor/config.yaml               \
          "$@"
      '';

      script_node = pkgs.writeScriptBin "node" ''
        ${shebang}
        cargo run --features=fast-runtime --release --bin creditcoin3-node -- \
          --dev                                                               \
          --tmp                                                               \
          --log=info,pallet_attestation_poc=debug                             \
          "$@"
      '';

      script_check = pkgs.writeScriptBin "cargo_check" ''
        ${shebang}
        cargo check --tests --release "$@"
      '';

      script_clippy = pkgs.writeScriptBin "cargo_clippy" ''
        ${shebang}
        cargo clippy --tests --release "$@"
      '';

      script_test = pkgs.writeScriptBin "cargo_test" ''
        ${shebang}
        cargo test --release "$@"
      '';
    in {
      # ====================================================================== #
      #                                PROGRAMS                                #
      # ====================================================================== #

      packages = {
        inherit attestor zombienet;
      };

      apps = rec {
        default = help;
        help = {
          type = "app";
          program = "${script_help}/bin/help";
        };
        attestor = {
          type = "app";
          program = "${script_attestor}/bin/attestor";
        };
        zombienet = {
          type = "app";
          program = "${script_zombienet}/bin/zombienet";
        };
        node = {
          type = "app";
          program = "${script_node}/bin/node";
        };
        anvil = {
          type = "app";
          program = "${script_anvil}/bin/anvil";
        };
        check = {
          type = "app";
          program = "${script_check}/bin/cargo_check";
        };
        clippy = {
          type = "app";
          program = "${script_clippy}/bin/cargo_clippy";
        };
        test = {
          type = "app";
          program = "${script_test}/bin/cargo_test";
        };
      };

      # ====================================================================== #
      #                                DEVSHELL                                #
      # ====================================================================== #

      devShells.default = pkgs.mkShell {
        nativeBuildInputs = nativeBuildInputs ++ [rust pkgs.sccache pkgs.gcc];
        buildInputs = with pkgs; [
          openssl
          rocksdb
          libpq
        ];

        # Additional packages for development
        packages = with pkgs; [
          jq
          foundry
          nodejs
        ];

        LIBCLANG_PATH = buildEnv.LIBCLANG_PATH;
        ROCKSDB_LIB_DIR = buildEnv.ROCKSDB_LIB_DIR;
        OPENSSL_LIB_DIR = buildEnv.OPENSSL_LIB_DIR;
        OPENSSL_INCLUDE_DIR = buildEnv.OPENSSL_INCLUDE_DIR;

        # sccache for shared compilation cache
        RUSTC_WRAPPER = sccacheEnv.RUSTC_WRAPPER;
        SCCACHE_DIR = sccacheEnv.SCCACHE_DIR;
        SCCACHE_CACHE_SIZE = sccacheEnv.SCCACHE_CACHE_SIZE;
        SCCACHE_SERVER_UDS = sccacheEnv.SCCACHE_SERVER_UDS;

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
          pkgs.gcc.cc.lib
          pkgs.openssl
          pkgs.libpq
        ];

        shellHook = ''
          # See https://nixos.wiki/wiki/Node.js
          export NPM_GLOBAL_PREFIX="$PWD/npm-global"
          export NODE_PATH="$NPM_GLOBAL_PREFIX/lib/node_modules";
          export PATH="$NPM_GLOBAL_PREFIX/bin:$PATH"
          npm config set prefix "$NPM_GLOBAL_PREFIX"

          # Start sccache server if not running
          ${pkgs.sccache}/bin/sccache --start-server 2>/dev/null || true
          echo "sccache stats:"
          ${pkgs.sccache}/bin/sccache --show-stats
        '';
      };
    });
}
