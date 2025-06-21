let
  helix-flake = builtins.getFlake "github:helix-editor/helix/3b7aaddb1391a9e343877ceadb83a41c95ede33f";
  pkgs = import (fetchTarball "https://github.com/NixOS/nixpkgs/archive/36ab78dab7da2e4e27911007033713bab534187b.tar.gz") {
    overlays = [
      (import (builtins.fetchTarball {
        url = "https://github.com/oxalica/rust-overlay/archive/master.tar.gz";
      }))
    ];
  };
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
  rustToolchainToml = builtins.fromTOML (builtins.readFile ./rust-toolchain.toml);
in
pkgs.mkShell {
  packages = [
    helix-flake.packages.${builtins.currentSystem}.helix
    pkgs.clangStdenv
    rustToolchain
  ];
  shellHook = ''
    echo "Using Rust: version"
    rustc --version
    rustup component add rust-analyzer
  '';
}
