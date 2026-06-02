{
  description = "Chores - a calendar/todo list system";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, ... }:
    {
      nixosModules.default = { config, lib, pkgs, ... }: {
        imports = [ ./nix/module.nix ];
        services.chores.package = lib.mkDefault self.packages.${pkgs.system}.chores-unwrapped;
      };
    } //
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Filter source to only include Rust/SQL/static files
        srcFilter = path: type:
          (craneLib.filterCargoSources path type)
          || (builtins.match ".*\\.sql$" path != null)
          || (builtins.match ".*/static/.*" path != null)
          || (builtins.match ".*/migrations/.*" path != null)
          || (builtins.match ".*/pictures/.*" path != null);

        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = srcFilter;
        };

        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];
        };

        # Build only the cargo dependencies for caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        chores-unwrapped = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = "chores-unwrapped";

          postInstall = ''
            mkdir -p $out/share/chores
            cp -r static $out/share/chores/static
            cp -r migrations $out/share/chores/migrations
          '';
        });

        # Wrapper that ensures static assets are accessible
        chores = pkgs.writeShellApplication {
          name = "chores";
          runtimeInputs = [ chores-unwrapped ];
          text = ''
            # Link static assets into the working directory if not already present
            if [ ! -d "static" ]; then
              ln -sfn "${chores-unwrapped}/share/chores/static" static
            fi
            if [ ! -d "migrations" ]; then
              ln -sfn "${chores-unwrapped}/share/chores/migrations" migrations
            fi
            exec chores-unwrapped "$@"
          '';
        };
      in
      {
        checks = {
          inherit chores-unwrapped;
          chores-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });
        };

        packages = {
          default = chores;
          inherit chores chores-unwrapped;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = chores;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = with pkgs; [
            sqlite
            pkg-config
            rust-analyzer
          ];
        };
      });
}
