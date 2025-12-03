{
  description = "cc3-next dev env and commands";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/release-25.05";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      # Set up rust overlay
      overlays = [(import rust-overlay)];
      pkgs = import nixpkgs {
        inherit system overlays;
      };

      # Load the rust version in use
      rustToolchain =
        if builtins.pathExists ./rust-toolchain.toml
        then builtins.readFile ./rust-toolchain.toml
        else ''
          [toolchain]
          channel = "1.81.0"
        '';
      rustChannel = builtins.fromTOML rustToolchain;
      rustVersion = rustChannel.toolchain.channel;

      # Add some substrate-specific dependencies
      rust = pkgs.rust-bin.stable.${rustVersion}.default.override {
        # Needed to build the runtime
        extensions = ["rust-src"];
        targets = ["wasm32-unknown-unknown"];
      };
    in {
      # TODO: this could be reworked to generate actual binary packages instead
      # of just runner scripts
      packages = let
        experimental = "--extra-experimental-features nix-command --extra-experimental-features flakes";
        shebang = "#!/usr/bin/env -S nix develop ${experimental} .#default -c bash";
        cargo-run = "cargo run --features=fast-runtime --release --bin";
      in {
        node = pkgs.writeScriptBin "node" ''
          ${shebang}
          ${cargo-run} creditcoin3-node -- --dev --tmp --log=info,pallet_attestation_poc=debug
        '';

        anvil = pkgs.writeScriptBin "anvil" ''
          ${shebang}
          anvil --block-time 6
        '';

        zombienet = pkgs.writeScriptBin "zombienet" ''
          ${shebang}
          cargo build --release -p attestor_new
          cargo run --release -p attestor_new_zombienet -- \
            -n 3                                           \
            --bin=./target/release/attestor                \
            --eth-url=ws://localhost:8545                  \
            --cc3-url=ws://localhost:9944                  \
            --funding-address='//Alice'                    \
            --config=./attestor_new/config.yaml
        '';

        prover = pkgs.writeScriptBin "prover" ''
          ${shebang}
          docker compose -f prover/docker-compose.yaml down
          docker compose -f prover/docker-compose.yaml up -d
          if [ ! -d artifacts ]; then
            mkdir artifacts;
          else
            rm -rf artifacts/chain_deployment_artifacts.json
          fi
          # secretlint-disable
          ${cargo-run} prover --                                                                     \
            --cc3-key "//Alice"                                                                      \
            --cc3-evm-private-key "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133" \
            --postgres-uri "postgres://prover:prover@127.0.0.1:5433/attestations"                    \
            --name "devprover"
          # secretlint-enable
          docker compose -f prover/docker-compose.yaml down
        '';

        transfer = pkgs.writeScriptBin "transfer" ''
          ${shebang}
          cd attestor/scripts && node Transfer.js $@
        '';

        query = pkgs.writeScriptBin "query" ''
          ${shebang}
          ${cargo-run} query-cli --                                                                  \
            --cc3-rpc-url ws://localhost:9944                                                        \
            --cc3-evm-private-key "8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
            --prover-contract-address 0xc01ee7f10ea4af4673cfff62710e1d7792aba8f3
        '';
      };

      apps = {
        node = {
          type = "app";
          program = "${self.packages.${system}.node}/bin/node";
        };

        anvil = {
          type = "app";
          program = "${self.packages.${system}.anvil}/bin/anvil";
        };

        zombienet = {
          type = "app";
          program = "${self.packages.${system}.zombienet}/bin/zombienet";
        };

        prover = {
          type = "app";
          program = "${self.packages.${system}.prover}/bin/prover";
        };

        transfer = {
          type = "app";
          program = "${self.packages.${system}.transfer}/bin/transfer";
        };
      };

      devShells.default = pkgs.mkShell {
        # Packages used at build time
        nativeBuildInputs = with pkgs; [
          rust
          perl
          openssl
          pkg-config
          protobuf
          clang
          gcc
        ];

        # Packages used at runtime
        buildInputs = with pkgs; [
          jq
          rocksdb
          clang
          libpq
        ];

        # Packages used for development
        packages = with pkgs; [
          foundry
          python310
          nodejs
        ];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
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

          # Make prover/verifier available in the PATH
          PROVER_PATH=$(readlink -f ./cairo/stone-prover)
          SCRIPTS_PATH=$(readlink -f ./cairo/scripts)
          VERIFIER_PATH=$(readlink -f ./cairo/stone-verifier)
          export PATH="$PATH:$PROVER_PATH:$SCRIPTS_PATH:$VERIFIER_PATH"
        '';
      };
    });
}
