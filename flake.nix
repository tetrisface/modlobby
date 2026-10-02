{
  description = "modlobby development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems =
        f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
    in
    {
      devShells = forAllSystems (
        pkgs:
        let
          # Tools needed for building and developing modlobby
          buildPackages = with pkgs; [
            pkg-config
            rustc
            cargo
            rustfmt
            clippy
            bun
            nodejs
            just
          ];

          # Native libraries required by Tauri on Linux
          tauriLibraries = with pkgs; [
            webkitgtk_4_1
            gtk3
            cairo
            gdk-pixbuf
            glib
            dbus
            openssl
            librsvg
            libsoup_3
            libayatana-appindicator
            xdotool
          ];

          # Runtime libraries required by the downloaded engine (Spring/Recoil) and pr-downloader
          engineLibraries = with pkgs; [
            stdenv.cc.cc.lib
            zlib
            SDL2
            libGL
            openal
            libx11
            libxcursor
            libxrandr
            libxi
            libxinerama
            curl
          ];

          allLibraries = tauriLibraries ++ engineLibraries;
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs =
              buildPackages
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux allLibraries;

            shellHook = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath allLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              export NIX_LD="${pkgs.glibc}/lib/ld-linux-x86-64.so.2"
              export NIX_LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath allLibraries}''${NIX_LD_LIBRARY_PATH:+:$NIX_LD_LIBRARY_PATH}"
              export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
            '';
          };
        }
      );
    };
}
