"""A layer-shell client that commits a zero width without both anchors must survive.

Quickshell does this briefly when it recreates a panel, and the compositor used to disconnect the
whole client for it (killing the bar). Speaks the wire protocol directly, so no client library is
needed. Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/layer_smoke.py
"""
import os, re, socket, struct, time
from smoke_support import Compositor, wait

TOP, BOTTOM, LEFT, RIGHT = 1, 2, 4, 8


def message(object_id, opcode, *args):
    body = b""
    for arg in args:
        if isinstance(arg, str):
            data = arg.encode() + b"\0"
            body += struct.pack("<I", len(data)) + data + b"\0" * (-len(data) % 4)
        else:
            body += struct.pack("<I", arg)
    return struct.pack("<II", object_id, ((8 + len(body)) << 16) | opcode) + body


class Client:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(path)
        self.data = b""
        self.globals = {}
        self.error = None
        self.next_id = 3

    def events(self):
        while len(self.data) >= 8:
            object_id, size_opcode = struct.unpack_from("<II", self.data)
            size, opcode = size_opcode >> 16, size_opcode & 0xFFFF
            if len(self.data) < size:
                return
            body, self.data = self.data[8:size], self.data[size:]
            yield object_id, opcode, body

    def roundtrip(self):
        """Wait for a wl_display.sync callback; returns False if the compositor hung up or errored."""
        callback = self.next_id
        self.next_id += 1
        self.sock.sendall(message(1, 0, callback))
        while True:
            for object_id, opcode, body in self.events():
                if object_id == 1 and opcode == 0:  # wl_display.error(object, code, message)
                    length = struct.unpack_from("<I", body, 8)[0]
                    self.error = body[12:12 + length - 1].decode()
                    return False
                if object_id == 2 and opcode == 0:  # wl_registry.global
                    name, length = struct.unpack_from("<II", body)
                    interface = body[8:8 + length - 1].decode()
                    version = struct.unpack_from("<I", body, 8 + ((length + 3) & ~3))[0]
                    self.globals[interface] = (name, version)
                if object_id == callback:
                    return True
            chunk = self.sock.recv(65536)
            if not chunk:
                return False
            self.data += chunk


comp = Compositor()
try:
    display = re.search(r'WAYLAND_DISPLAY="([^"]+)"', open(comp.dir + "/log").read() if wait(lambda: 'WAYLAND_DISPLAY' in open(comp.dir + "/log").read()) else "").group(1)
    client = Client(os.path.join(os.environ["XDG_RUNTIME_DIR"], display))
    client.sock.sendall(message(1, 1, 2))  # get_registry -> object 2
    assert client.roundtrip()
    compositor, shell = client.globals["wl_compositor"], client.globals["zwlr_layer_shell_v1"]
    client.next_id = 8  # objects 4 to 7 are created below
    client.sock.sendall(message(2, 0, compositor[0], "wl_compositor", 1, 4))
    client.sock.sendall(message(2, 0, shell[0], "zwlr_layer_shell_v1", 1, 5))
    client.sock.sendall(message(4, 0, 6))  # wl_compositor.create_surface -> object 6
    # get_layer_surface(id 7, surface 6, output null, layer top, namespace)
    client.sock.sendall(message(5, 0, 7, 6, 0, 2, "test-marker"))
    # zero width, anchored top, bottom and right only: invalid by the letter of the protocol
    client.sock.sendall(message(7, 1, TOP | BOTTOM | RIGHT))  # set_anchor
    client.sock.sendall(message(7, 0, 0, 0))  # set_size(0, 0)
    client.sock.sendall(message(6, 6))  # wl_surface.commit
    assert client.roundtrip(), f"the compositor disconnected the client: {client.error}"
    # The client fixes its size afterwards, as Quickshell does.
    client.sock.sendall(message(7, 0, 5, 0))
    client.sock.sendall(message(6, 6))
    assert client.roundtrip(), f"disconnected after the corrected size: {client.error}"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
