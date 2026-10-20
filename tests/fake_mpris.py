#!/usr/bin/env python3
"""A media player for the MPRIS tests on the session bus in $DBUS_SESSION_BUS_ADDRESS: it plays the title given as
the first argument, by an artist and with a cover (`file://` URL) if given as the second and third. Play, Pause,
Next and Previous are appended to the file named by the fourth argument, if any, one line each, and change what
it plays; `org.mywm.Test.SetPlaying(b)` on /org/mpris/MediaPlayer2 starts or pauses it from outside."""
import sys

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

PROPERTIES = "org.freedesktop.DBus.Properties"
PLAYER = "org.mpris.MediaPlayer2.Player"
ROOT = "org.mpris.MediaPlayer2"

args = sys.argv[1:] + [""] * 4
title, artist, cover, calls = args[:4]
log = open(calls, "a", buffering=1) if calls else None
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()


class Player(dbus.service.Object):
    def __init__(self):
        super().__init__(bus, "/org/mpris/MediaPlayer2")
        metadata = {"xesam:title": dbus.String(title)}
        if artist:
            metadata["xesam:artist"] = dbus.Array([dbus.String(artist)], signature="s")
        if cover:
            metadata["mpris:artUrl"] = dbus.String(cover)
        self.interfaces = {
            ROOT: {"Identity": dbus.String("Testplayer")},
            PLAYER: {
                "PlaybackStatus": dbus.String("Playing"),
                "Metadata": dbus.Dictionary(metadata, signature="sv"),
                "CanControl": dbus.Boolean(True),
                "CanPlay": dbus.Boolean(True),
                "CanPause": dbus.Boolean(True),
                "CanGoNext": dbus.Boolean(True),
                "CanGoPrevious": dbus.Boolean(False),
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

    def playing(self, playing):
        status = dbus.String("Playing" if playing else "Paused")
        self.interfaces[PLAYER]["PlaybackStatus"] = status
        self.PropertiesChanged(PLAYER, dbus.Dictionary({"PlaybackStatus": status}, signature="sv"), dbus.Array([], signature="s"))

    def called(self, name):
        if log:
            log.write(name + "\n")

    @dbus.service.method(PLAYER)
    def Play(self):
        self.called("Play")
        self.playing(True)

    @dbus.service.method(PLAYER)
    def Pause(self):
        self.called("Pause")
        self.playing(False)

    @dbus.service.method(PLAYER)
    def Next(self):
        self.called("Next")

    @dbus.service.method(PLAYER)
    def Previous(self):
        self.called("Previous")

    @dbus.service.method("org.mywm.Test", in_signature="b")
    def SetPlaying(self, playing):
        self.playing(playing)


Player()
name = dbus.service.BusName("org.mpris.MediaPlayer2.mywmtest", bus)
GLib.MainLoop().run()
