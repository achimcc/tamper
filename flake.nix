{
  description = "Mutation-test your build-time assertions";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  # The RustSec advisory database, pinned like any other input. The `audit`
  # check reads it offline; `nix flake update advisory-db` brings news in.
  inputs.advisory-db = {
    url = "github:rustsec/advisory-db";
    flake = false;
  };

  outputs =
    {
      self,
      nixpkgs,
      advisory-db,
    }:
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
          # The test suite drives the real tools rather than mocking them:
          # a throwaway worktree is a `git worktree`, a lever is a real
          # `sed`. Without these three, `nix flake check` would run one test
          # file and stop.
          nativeCheckInputs = with pkgs; [
            git
            gnused
            perl
          ];
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
          # Known advisories against Cargo.lock, read offline from the pinned
          # database.
          audit = pkgs.runCommand "tamper-audit" { nativeBuildInputs = [ pkgs.cargo-audit ]; } ''
            HOME=$TMPDIR cargo-audit audit --no-fetch --db ${advisory-db} --file ${./Cargo.lock}
            touch $out
          '';
          # Bans, sources and licenses of the dependency tree (deny.toml).
          # Inside the package's build environment: the vendored crates are
          # what `cargo metadata` reads there, so nothing is fetched.
          deny = package.overrideAttrs (old: {
            pname = "tamper-deny";
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.cargo-deny ];
            buildPhase = "cargo deny --offline check bans sources licenses";
            doCheck = false;
            installPhase = "touch $out";
          });
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
