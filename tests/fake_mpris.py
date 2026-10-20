#!/usr/bin/env python3
"""A media player for the MPRIS tests on the session bus in $DBUS_SESSION_BUS_ADDRESS: it plays the title given as
the first argument, and `org.mywm.Test.SetPlaying(b)` on /org/mpris/MediaPlayer2 starts or pauses it."""
import sys

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

PROPERTIES = "org.freedesktop.DBus.Properties"
PLAYER = "org.mpris.MediaPlayer2.Player"
ROOT = "org.mpris.MediaPlayer2"

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()


class Player(dbus.service.Object):
    def __init__(self):
        super().__init__(bus, "/org/mpris/MediaPlayer2")
        self.interfaces = {
            ROOT: {"Identity": dbus.String("Testplayer")},
            PLAYER: {
                "PlaybackStatus": dbus.String("Playing"),
                "Metadata": dbus.Dictionary({"xesam:title": dbus.String(sys.argv[1])}, signature="sv"),
            },
        }

    @dbus.service.method(PROPERTIES, in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return self.interfaces[interface][name]

    @dbus.service.method(PROPERTIES, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return dbus.Dictionary(self.interfaces.get(interface, {}), signature="sv")

    @dbus.service.signal(PROPERTIES, signature="sa{sv}as")
    def PropertiesChanged(self, interface, changed, invalidated):
        pass

    @dbus.service.method("org.mywm.Test", in_signature="b")
    def SetPlaying(self, playing):
        status = dbus.String("Playing" if playing else "Paused")
        self.interfaces[PLAYER]["PlaybackStatus"] = status
        self.PropertiesChanged(PLAYER, dbus.Dictionary({"PlaybackStatus": status}, signature="sv"), dbus.Array([], signature="s"))


Player()
name = dbus.service.BusName("org.mpris.MediaPlayer2.mywmtest", bus)
GLib.MainLoop().run()
