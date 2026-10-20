{
  lib,
  stdenvNoCC,
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
  pixman, # cursor plane on rotated outputs (smithay's renderer_pixman)
  quickshell,
  swayidle,
  wlopm,
  wl-clipboard,
  gammastep,
  wireplumber, # wpctl: the built-in bar's volume
  iproute2,
  fontconfig, # fc-match: the font of the built-in shell's Qt controls
  libnotify,
  xwayland,
  dbus,
  coreutils,
  util-linux, # waitpid: the session's startup script follows the compositor without polling
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
    pixman
  ];

  # Only what cargo reads, so that changes to the scripts, tests, docs or the shell input do not
  # rebuild the compositor.
  unwrapped = rustPlatform.buildRustPackage {
    pname = "mywm-unwrapped";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../src
        ../crates
        ../vendor/smithay
      ];
    };

    cargoLock.lockFile = ../Cargo.lock;

    # The compositor, the screen-cast portal and the built-in shell's services; the test clients are not installed.
    cargoBuildFlags = [ "-p" "mywm-compositor" "-p" "mywm-portal" "-p" "mywm-shell-services" ];
    # The tests run in development (`cargo test --workspace`, the smoke tests in `tests/`) before
    # a dependent flake.lock follows a commit; here they would only compile everything a second time.
    doCheck = false;

    nativeBuildInputs = [
      pkg-config
      llvmPackages.clang
    ];
    buildInputs = runtimeLibraries;
    # bindgen (pipewire-sys) needs libclang.
    LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

    # EGL, libinput and libxkbcommon are opened at run time.
    postFixup = ''
      patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/mywm-compositor
      patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/mywm-portal
      patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/mywm-shell-services
    '';

    inherit meta;
  };

  meta = {
    description = "Smithay-based tiling Wayland compositor with its Quickshell shell and screen-cast portal";
    homepage = "https://github.com/Chr1ssi/MyWM-Smithay";
    license = lib.licenses.mit;
    mainProgram = "mywm-compositor";
    platforms = lib.platforms.linux;
  };
in
# The binaries with the shell, the session scripts, the portal files and the helpers they start.
# A cheap copy, so that the shell and the helpers (`.override { quickshell = ...; }`) can change
# without compiling Rust again.
stdenvNoCC.mkDerivation {
  pname = "mywm";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../scripts
      ../portal
    ];
  };

  nativeBuildInputs = [ makeWrapper ];
  # Like buildRustPackage: the scripts keep their `/usr/bin/env bash` shebang.
  strictDeps = true;
  # Already stripped, and shrinking the RPATH would drop the libraries opened at run time.
  dontStrip = true;
  dontPatchELF = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin $out/share/mywm $out/share/xdg-desktop-portal/portals $out/share/dbus-1/services $out/lib/systemd/user
    # mywm-shell-services next to the compositor, which starts it from there.
    cp ${unwrapped}/bin/mywm-compositor ${unwrapped}/bin/mywm-portal ${unwrapped}/bin/mywm-shell-services $out/bin/
    cp -r scripts $out/share/mywm/
    cp -r ${shellSrc}/quickshell $out/share/mywm/quickshell
    install -Dm644 portal/mywm.portal $out/share/xdg-desktop-portal/portals/mywm.portal
    substitute portal/org.freedesktop.impl.portal.desktop.mywm.service \
      $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mywm.service \
      --replace-fail "/usr/bin/env mywm-portal" "$out/bin/mywm-portal"
    substitute portal/mywm-portal.service $out/lib/systemd/user/mywm-portal.service \
      --replace-fail "/usr/bin/env mywm-portal" "$out/bin/mywm-portal"
    install -Dm755 scripts/mywm-session $out/bin/mywm-session
    install -Dm755 scripts/session-environment $out/bin/session-environment
    runHook postInstall
  '';

  # The helpers the compositor and the session start.
  postFixup = ''
    wrapProgram $out/bin/mywm-compositor \
      --set-default MYWM_SHELL_DIR $out/share/mywm/quickshell \
      --prefix PATH : ${lib.makeBinPath [
        quickshell
        swayidle
        wlopm
        wl-clipboard
        gammastep # the shell's night light
        wireplumber # wpctl: the built-in bar's volume
        iproute2 # the shell's local addresses
        fontconfig # fc-match: the font of the shell's form fields
        libnotify
        xwayland
        coreutils
      ]}
    wrapProgram $out/bin/mywm-session \
      --set-default MYWM_BINARY $out/bin/mywm-compositor \
      --set-default MYWM_SESSION_ENVIRONMENT $out/bin/session-environment \
      --prefix PATH : ${lib.makeBinPath [ dbus systemd coreutils util-linux ]}
  '';

  passthru = { inherit unwrapped; };

  inherit meta;
}
