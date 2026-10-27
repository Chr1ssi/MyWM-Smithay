{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  wayland,
  libxkbcommon,
  libglvnd,
  libx11,
  libxcursor,
  libxrandr,
  libxi,
  wl-clipboard,
}:

let
  runtimeLibraries = [
    wayland
    libxkbcommon
    libglvnd
    libx11
    libxcursor
    libxrandr
    libxi
  ];
in
rustPlatform.buildRustPackage {
  pname = "splice-settings";
  version = "0.1.0";

  # Only what cargo reads and the desktop entry, so that changes to the tests, docs or flake inputs
  # do not rebuild it.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../crates
      ../vendor/smithay
      ../scripts/splice-settings.desktop
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [ "-p" "splice-settings" ];
  # Logic tests run in development (`cargo test -p splice-settings`); the window test needs a display.
  doCheck = false;

  nativeBuildInputs = [ pkg-config makeWrapper ];
  buildInputs = runtimeLibraries;

  postInstall = ''
    install -Dm644 scripts/splice-settings.desktop $out/share/applications/splice-settings.desktop
  '';

  # The window toolkit opens Wayland, X11 and GL at run time.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/splice-settings
  '';

  meta = {
    description = "Graphical settings and keybinding editor for splice";
    homepage = "https://github.com/Chr1ssi/Splice-Compositor";
    mainProgram = "splice-settings";
    platforms = lib.platforms.linux;
  };
}
