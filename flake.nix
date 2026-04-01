{
  description = "Flake for the evs crate";

  inputs = {
    nixpkgs.url = "github:NixOs/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } rec {
      systems = [ "x86_64-linux" ];
      perSystem =
        { self', pkgs, lib, ... }:
        {
          packages.evs =
            let
              manifest = fromTOML (builtins.readFile ./Cargo.toml);
            in
            pkgs.rustPlatform.buildRustPackage {
              pname = manifest.package.name;
              version = manifest.package.version;
              src = ./.;
              nativeBuildInputs = [ pkgs.installShellFiles ];

              cargoLock.lockFile = ./Cargo.lock;

              meta = {
                description = manifest.package.description;
                homepage = manifest.package.repository;
                license = lib.licenses.mit;
                mainProgram = manifest.package.name;
                platforms = systems;
              };

              postInstall = 
                let
                  mangen = ''
                    mkdir man_pages
                    $out/bin/evs mangen man_pages
                    installManPage man_pages/*
                  '';
                  genShellCompletion = shell: "installShellCompletion --${shell} --name evs.${shell} <(COMPLETE=${shell} $out/bin/evs)";
                  shells = [ "bash" "fish" "zsh" ];
                in
                ''
                  ${mangen}

                  ${lib.concatStringsSep "\n" (lib.forEach shells genShellCompletion)}
                '';
            };

          packages.default = self'.packages.evs;
        };
    };
}
