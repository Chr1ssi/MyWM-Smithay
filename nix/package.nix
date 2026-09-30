{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  llvmPackages,
  wayland,
  libxkbcommon,
  libinput,
  seatd,
  libgbm,
  libdrm,
  systemd, # libudev
  libglvnd,
  pipewire,
  quickshell,
  swaylock,
  swayidle,
  wlopm,
  wl-clipboard,
  libnotify,
  xwayland,
  dbus,
  coreutils,
  shellSrc,
}:

let
  runtimeLibraries = [
    wayland
    libxkbcommon
    libinput
    seatd
    libgbm
    libdrm
    systemd
    libglvnd
    pipewire
  ];
in
rustPlatform.buildRustPackage {
  pname = "mywm";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: _type: !(builtins.elem (builtins.baseNameOf path) [ ".git" "target" "quickshell" ]);
  };

  cargoLock.lockFile = ../Cargo.lock;

  # The compositor and the screen-cast portal; the test clients are not installed.
  cargoBuildFlags = [ "-p" "mywm-compositor" "-p" "mywm-portal" ];
  # The tests need a display and more; `cargo test` runs them in development.
  doCheck = false;

  nativeBuildInputs = [
    pkg-config
    makeWrapper
    llvmPackages.clang
  ];
  buildInputs = runtimeLibraries;
  # bindgen (pipewire-sys) needs libclang.
  LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

  postInstall = ''
    mkdir -p $out/share/mywm $out/share/xdg-desktop-portal/portals $out/share/dbus-1/services
    cp -r scripts $out/share/mywm/
    cp -r ${shellSrc}/quickshell $out/share/mywm/quickshell
    install -Dm644 portal/mywm.portal $out/share/xdg-desktop-portal/portals/mywm.portal
    substitute portal/org.freedesktop.impl.portal.desktop.mywm.service \
      $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mywm.service \
      --replace-fail "/usr/bin/env mywm-portal" "$out/bin/mywm-portal"
    install -Dm755 scripts/mywm-session $out/bin/mywm-session
    install -Dm755 scripts/session-environment $out/bin/session-environment
  '';

  # EGL, libinput and libxkbcommon are opened at run time; the helpers it starts come along.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/mywm-compositor
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/mywm-portal
    wrapProgram $out/bin/mywm-compositor \
      --set-default MYWM_SHELL_DIR $out/share/mywm/quickshell \
      --prefix PATH : ${lib.makeBinPath [
        quickshell
        swaylock
        swayidle
        wlopm
        wl-clipboard
        libnotify
        xwayland
        coreutils
      ]}
    wrapProgram $out/bin/mywm-session \
      --set-default MYWM_BINARY $out/bin/mywm-compositor \
      --set-default MYWM_SESSION_ENVIRONMENT $out/bin/session-environment \
      --prefix PATH : ${lib.makeBinPath [ dbus systemd coreutils ]}
  '';

  meta = {
    description = "Smithay-based tiling Wayland compositor with its Quickshell shell and screen-cast portal";
    homepage = "https://github.com/Chr1ssi/MyWM-smithey";
    mainProgram = "mywm-compositor";
    platforms = lib.platforms.linux;
  };
}
