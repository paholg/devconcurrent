{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    nix2container = {
      url = "github:nlewo/nix2container";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
      nix2container,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-analyzer"
            "rust-src"
          ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain (p: rust);

        crateName = craneLib.crateNameFromCargoToml {
          cargoToml = ./crates/devconcurrent/Cargo.toml;
        };
        serviceCrateName = craneLib.crateNameFromCargoToml {
          cargoToml = ./crates/devconcurrent-service/Cargo.toml;
        };

        commonArgs = {
          inherit (crateName) pname version;
          src = ./.;
          strictDeps = true;
          nativeBuildInputs = [ ];
        };

        artifacts = commonArgs // {
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        };

        package = craneLib.buildPackage (
          artifacts
          // {
            meta.mainProgram = crateName.pname;
            doCheck = false;
          }
        );

        servicePackage = craneLib.buildPackage (
          artifacts
          // {
            pname = serviceCrateName.pname;
            version = serviceCrateName.version;
            cargoToml = ./crates/devconcurrent-service/Cargo.toml;
            cargoExtraArgs = "--package ${serviceCrateName.pname}";
            meta.mainProgram = serviceCrateName.pname;
            doCheck = true;
          }
        );

        # OCI image for the service.
        dockerImage = nix2container.packages.${system}.nix2container.buildImage {
          name = "devconcurrent-service";
          tag = serviceCrateName.version;
          copyToRoot = [ pkgs.cacert ];
          maxLayers = 100;
          config = {
            Entrypoint = [ "${servicePackage}/bin/${serviceCrateName.pname}" ];
            ExposedPorts = {
              "53/udp" = { };
              "53/tcp" = { };
              "80/tcp" = { };
              "443/tcp" = { };
            };
          };
        };

      in
      {
        checks = {
          clippy = craneLib.cargoClippy (
            artifacts
            // {
              cargoClippyExtraArgs = "-- --deny warnings";
            }
          );
          fmt = craneLib.cargoFmt artifacts;
          test = craneLib.cargoNextest artifacts;
        };
        packages = {
          default = package;
          service = servicePackage;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          docker-service-image = dockerImage;
        };
        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              cargo-dist
              cargo-edit
              cargo-nextest
              fd
              just
              nodejs
              pandoc
            ]
            ++ [ rust ];
          env = {
            RUST_BACKTRACE = 1;
          };
        };
      }
    );
}
