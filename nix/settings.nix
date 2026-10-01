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
  pname = "mywm-settings";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: _type: !(builtins.elem (builtins.baseNameOf path) [ ".git" "target" "quickshell" ]);
  };

  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [ "-p" "mywm-settings" ];
  # Logic tests run in development (`cargo test -p mywm-settings`); the window test needs a display.
  doCheck = false;

  nativeBuildInputs = [ pkg-config makeWrapper ];
  buildInputs = runtimeLibraries;

  postInstall = ''
    install -Dm644 scripts/mywm-settings.desktop $out/share/applications/mywm-settings.desktop
  '';

  # The window toolkit opens Wayland, X11 and GL at run time.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/mywm-settings
  '';

  meta = {
    description = "Graphical settings and keybinding editor for mywm";
    homepage = "https://github.com/Chr1ssi/MyWM-Smithay";
    mainProgram = "mywm-settings";
    platforms = lib.platforms.linux;
  };
}
