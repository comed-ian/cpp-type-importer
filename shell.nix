let
  pkgs = import <nixpkgs> {};
  helix-flake = builtins.getFlake "github:helix-editor/helix/3b7aaddb1391a9e343877ceadb83a41c95ede33f";
in
pkgs.mkShell {
  packages = [
    helix-flake.packages.${builtins.currentSystem}.helix
    pkgs.clangStdenv
    pkgs.rustup
  ];
  shellHook = ''
    rustup component add rust-analyzer
  '';
}
