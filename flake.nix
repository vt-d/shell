{
  description = "A native Rust-based shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, home-manager }:
    (flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        workspace = (pkgs.lib.importTOML ./Cargo.toml).workspace.package;
        # Embedded into the binary by shell-m3e (`icons.rs`) at build time.
        materialSymbols = "${pkgs.material-symbols}/share/fonts/truetype/MaterialSymbolsRounded[FILL,GRAD,opsz,wght].ttf";
        # Embedded by shell-m3e (`typeface.rs`): Google Sans Flex, Material 3 Expressive's
        # typeface (OFL). Not yet in nixpkgs' google-fonts, so pinned from google/fonts.
        googleSansFlex = pkgs.fetchurl {
          name = "GoogleSansFlex.ttf";
          url = "https://raw.githubusercontent.com/google/fonts/bd8f81ddb5c74d5c8897b36ad88b440266245103/ofl/googlesansflex/GoogleSansFlex%5BROND,opsz,slnt,wdth,wght%5D.ttf";
          hash = "sha256-qax3B5iFfV6KxZPO1Csi5Ap8CM7Jgxn5Kw7Mwng6d7o=";
        };
        fonts = {
          MATERIAL_SYMBOLS_ROUNDED = materialSymbols;
          GOOGLE_SANS_FLEX = googleSansFlex;
        };
        runtimeLibraries = with pkgs; [ wayland libxkbcommon vulkan-loader libGL ];
        runtimePrograms = with pkgs; [
          bash
          coreutils
          gnugrep
          curl
          bluez
          networkmanager
          wireplumber
          pipewire
          systemd
          util-linux
          mako
        ];
      in
      {
        packages = rec {
          default = shell;
          shell = pkgs.rustPlatform.buildRustPackage (fonts // {
            pname = "shell";
            version = workspace.version;
            # Only build inputs: do not copy target/, .git/, or development caches.
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./crates ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            # The workspace's one binary, `shell`, comes from the shell-main crate.
            cargoBuildFlags = [ "--package" "shell-main" ];
            cargoTestFlags = [ "--workspace" ];
            # smithay-client-toolkit reads keymaps with libxkbcommon.
            nativeBuildInputs = [ pkgs.makeWrapper pkgs.pkg-config ];
            buildInputs = [ pkgs.libxkbcommon ];
            postFixup = ''
              wrapProgram "$out/bin/shell" \
                --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeLibraries}" \
                --prefix PATH : "${pkgs.lib.makeBinPath runtimePrograms}"
            '';

            meta = {
              description = "A native Rust-based shell";
              mainProgram = "shell";
              platforms = pkgs.lib.platforms.linux;
            };
          });
        };

        apps.default = (flake-utils.lib.mkApp { drv = self.packages.${system}.default; }) // {
          meta = self.packages.${system}.default.meta;
        };

        devShells.default = pkgs.mkShell (fonts // {
          inputsFrom = [ self.packages.${system}.default ];
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            cargo-watch
            cargo-nextest
          ];
          RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
          # winit/wgpu dlopen these at runtime.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibraries;
        });

        formatter = pkgs.nixpkgs-fmt;

        checks = {
          build = self.packages.${system}.default;
          home-manager = import ./nix/checks.nix {
            inherit pkgs self home-manager;
          };
        };
      })) // {
      homeManagerModules = rec {
        desktop-shell = import ./nix/home-manager.nix self;
        default = desktop-shell;
      };
    };
}
