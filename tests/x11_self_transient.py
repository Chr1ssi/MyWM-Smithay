"""An X11 window that is transient for itself (WM_TRANSIENT_FOR names its own id), for xwayland_smoke.py.

Xlib through ctypes: the smoke shell has libX11 (LD_LIBRARY_PATH) but no Python X bindings.
"""
import ctypes, time
from ctypes import POINTER, Structure, byref, c_char_p, c_int, c_uint, c_ulong, c_void_p


class XClassHint(Structure):
    _fields_ = [("res_name", c_char_p), ("res_class", c_char_p)]


x = ctypes.CDLL("libX11.so.6")
x.XOpenDisplay.argtypes = [c_char_p]
x.XOpenDisplay.restype = c_void_p
x.XDefaultRootWindow.argtypes = [c_void_p]
x.XDefaultRootWindow.restype = c_ulong
x.XCreateSimpleWindow.argtypes = [c_void_p, c_ulong, c_int, c_int, c_uint, c_uint, c_uint, c_ulong, c_ulong]
x.XCreateSimpleWindow.restype = c_ulong
x.XSetClassHint.argtypes = [c_void_p, c_ulong, POINTER(XClassHint)]
x.XSetTransientForHint.argtypes = [c_void_p, c_ulong, c_ulong]
x.XMapWindow.argtypes = [c_void_p, c_ulong]
x.XFlush.argtypes = [c_void_p]

display = x.XOpenDisplay(None)
assert display, "no X display"
window = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 0, 0, 200, 100, 0, 0, 0)
x.XSetClassHint(display, window, byref(XClassHint(b"selftransient", b"SelfTransient")))
x.XSetTransientForHint(display, window, window)
x.XMapWindow(display, window)
x.XFlush(display)
time.sleep(60)
