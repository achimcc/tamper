{
  description = "Mutation-test your build-time assertions";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in
    {
      packages = forAll (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "tamper";
          # Read out of Cargo.toml so the store path and the crate cannot disagree.
          version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            description = "Mutation-test your build-time assertions";
            homepage = "https://github.com/achimcc/tamper";
            license = pkgs.lib.licenses.agpl3Only;
            mainProgram = "tamper";
          };
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
          ];
        };
      });

      checks = forAll (
        pkgs:
        let
          package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        in
        {
          inherit package;
          clippy = package.overrideAttrs (old: {
            pname = "tamper-clippy";
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
            buildPhase = "cargo clippy --all-targets -- -D warnings";
            doCheck = false;
            installPhase = "touch $out";
          });
          fmt = package.overrideAttrs (old: {
            pname = "tamper-fmt";
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.rustfmt ];
            buildPhase = "cargo fmt --check";
            doCheck = false;
            installPhase = "touch $out";
          });
        }
      );
    };
}
