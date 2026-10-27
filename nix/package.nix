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
  linux-pam, # the built-in lock screen's password check (splice-shell-services --authenticate)
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
    linux-pam
  ];

  # Only what cargo reads, so that changes to the scripts, tests, docs or the shell input do not
  # rebuild the compositor.
  unwrapped = rustPlatform.buildRustPackage {
    pname = "splice-unwrapped";
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
    cargoBuildFlags = [ "-p" "splice-compositor" "-p" "splice-portal" "-p" "splice-shell-services" ];
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
      patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/splice-compositor
      patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/splice-portal
      patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/splice-shell-services
    '';

    inherit meta;
  };

  meta = {
    description = "Smithay-based tiling Wayland compositor with its built-in shell and screen-cast portal";
    homepage = "https://github.com/Chr1ssi/MyWM-Smithay";
    license = lib.licenses.mit;
    mainProgram = "splice-compositor";
    platforms = lib.platforms.linux;
  };
in
# The binaries, the session scripts, the portal files and the helpers they start. A cheap copy next to
# the Rust build, so that the scripts can change without compiling Rust again.
stdenvNoCC.mkDerivation {
  pname = "splice";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../scripts
      ../portal
      ../assets
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
    mkdir -p $out/bin $out/share/splice $out/share/xdg-desktop-portal/portals $out/share/dbus-1/services $out/lib/systemd/user
    # splice-shell-services next to the compositor, which starts it from there.
    cp ${unwrapped}/bin/splice-compositor ${unwrapped}/bin/splice-portal ${unwrapped}/bin/splice-shell-services $out/bin/
    cp -r scripts $out/share/splice/
    install -Dm644 portal/splice.portal $out/share/xdg-desktop-portal/portals/splice.portal
    substitute portal/org.freedesktop.impl.portal.desktop.splice.service \
      $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.splice.service \
      --replace-fail "/usr/bin/env splice-portal" "$out/bin/splice-portal"
    substitute portal/splice-portal.service $out/lib/systemd/user/splice-portal.service \
      --replace-fail "/usr/bin/env splice-portal" "$out/bin/splice-portal"
    install -Dm644 assets/splice.svg $out/share/icons/hicolor/scalable/apps/splice.svg
    install -Dm755 scripts/splice-session $out/bin/splice-session
    install -Dm755 scripts/session-environment $out/bin/session-environment
    runHook postInstall
  '';

  # The helpers the compositor and the session start.
  postFixup = ''
    wrapProgram $out/bin/splice-compositor \
      --prefix PATH : ${lib.makeBinPath [
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
    wrapProgram $out/bin/splice-session \
      --set-default SPLICE_BINARY $out/bin/splice-compositor \
      --set-default SPLICE_SESSION_ENVIRONMENT $out/bin/session-environment \
      --prefix PATH : ${lib.makeBinPath [ dbus systemd coreutils util-linux ]}
  '';

  passthru = { inherit unwrapped; };

  inherit meta;
}
