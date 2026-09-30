"""Check which Wayland globals the nested compositor advertises.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/globals_smoke.py
Speaks just enough of the wire protocol to read the registry, so it needs no client library.
"""
import os, re, socket, struct
from smoke_support import Compositor, wait

# Advertised with every backend.
EXPECTED = {
    "wl_compositor", "wl_shm", "wl_seat", "xdg_wm_base", "wl_output", "zxdg_output_manager_v1",
    "wl_data_device_manager", "zwp_primary_selection_device_manager_v1",
    "wp_viewporter", "wp_fractional_scale_manager_v1", "wp_cursor_shape_manager_v1",
    "wp_content_type_manager_v1", "zwp_relative_pointer_manager_v1", "zwp_pointer_constraints_v1",
    "zxdg_decoration_manager_v1", "wp_tearing_control_manager_v1",
    "zwlr_layer_shell_v1", "ext_session_lock_manager_v1", "ext_idle_notifier_v1",
    "zwp_idle_inhibit_manager_v1", "zwlr_output_power_manager_v1", "zwlr_screencopy_manager_v1", "zwlr_data_control_manager_v1",
    "zwlr_output_manager_v1", "ext_foreign_toplevel_list_v1", "ext_image_copy_capture_manager_v1",
    "ext_output_image_capture_source_manager_v1", "ext_foreign_toplevel_image_capture_source_manager_v1",
}
# Only with the hardware backend: they need the GPU.
HARDWARE_ONLY = {"zwp_linux_dmabuf_v1", "wp_presentation", "wp_linux_drm_syncobj_manager_v1"}


def list_globals(path):
    sock = socket.socket(socket.AF_UNIX)
    sock.connect(path)
    # wl_display (object 1): get_registry -> object 2, sync -> object 3.
    sock.sendall(struct.pack("<III", 1, (12 << 16) | 1, 2) + struct.pack("<III", 1, (12 << 16) | 0, 3))
    found, data = {}, b""
    while True:
        data += sock.recv(65536)
        while len(data) >= 8:
            object_id, size_opcode = struct.unpack_from("<II", data)
            size, opcode = size_opcode >> 16, size_opcode & 0xFFFF
            if len(data) < size:
                break
            body, data = data[8:size], data[size:]
            if object_id == 2 and opcode == 0:  # wl_registry.global(name, interface, version)
                length = struct.unpack_from("<I", body, 4)[0]
                interface = body[8:8 + length - 1].decode()
                version = struct.unpack_from("<I", body, 8 + ((length + 3) & ~3))[0]
                found[interface] = version
            elif object_id == 3:  # wl_callback.done
                return found


comp = Compositor()
try:
    assert wait(lambda: re.search(r'WAYLAND_DISPLAY="([^"]+)"', open(comp.dir + "/log").read()))
    display = re.search(r'WAYLAND_DISPLAY="([^"]+)"', open(comp.dir + "/log").read()).group(1)
    globals_ = list_globals(os.path.join(os.environ["XDG_RUNTIME_DIR"], display))
    print(sorted(globals_))
    missing = EXPECTED - set(globals_)
    assert not missing, f"missing globals: {sorted(missing)}"
    unexpected = HARDWARE_ONLY & set(globals_)
    assert not unexpected, f"hardware-only globals advertised while nested: {sorted(unexpected)}"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
