#!/usr/bin/env python3
"""A tray item (StatusNotifierItem with a DBusMenu) on the session bus in $DBUS_SESSION_BUS_ADDRESS: a green 16 pixel
icon, and a menu with "_Öffnen", a separator, the submenu "Mehr" with "Unterpunkt", and a hidden entry. It registers
with the watcher (again whenever one appears). Calls are appended to the file named by the first argument, one line
each: "Activate", "SecondaryActivate", "Scroll <delta> <orientation>", "AboutToShow <id>", "Event <id> <event>"."""
import os
import sys

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

ITEM = "org.kde.StatusNotifierItem"
MENU = "com.canonical.dbusmenu"
PROPERTIES = "org.freedesktop.DBus.Properties"
WATCHER = "org.kde.StatusNotifierWatcher"

log = open(sys.argv[1], "a", buffering=1)
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
NAME = f"org.kde.StatusNotifierItem-{os.getpid()}-1"
GREEN = bytes([255, 0, 255, 0]) * 16 * 16  # ARGB


class Item(dbus.service.Object):
    properties = {
        "Category": dbus.String("ApplicationStatus"),
        "Id": dbus.String("fake-tray"),
        "Title": dbus.String("Testapp"),
        "Status": dbus.String("Active"),
        "IconName": dbus.String(""),
        "IconPixmap": dbus.Array([dbus.Struct((dbus.Int32(16), dbus.Int32(16), dbus.ByteArray(GREEN)))], signature="(iiay)"),
        "ItemIsMenu": dbus.Boolean(False),
        "Menu": dbus.ObjectPath("/Menu"),
    }

    @dbus.service.method(PROPERTIES, in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return self.properties[name]

    @dbus.service.method(PROPERTIES, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return dbus.Dictionary(self.properties, signature="sv")

    @dbus.service.method(ITEM, in_signature="ii")
    def Activate(self, x, y):
        log.write("Activate\n")

    @dbus.service.method(ITEM, in_signature="ii")
    def SecondaryActivate(self, x, y):
        log.write("SecondaryActivate\n")

    @dbus.service.method(ITEM, in_signature="is")
    def Scroll(self, delta, orientation):
        log.write(f"Scroll {delta} {orientation}\n")


def node(id, properties, children=()):
    return dbus.Struct((dbus.Int32(id), dbus.Dictionary(properties, signature="sv"), dbus.Array(children, signature="v")))


class Menu(dbus.service.Object):
    @dbus.service.method(MENU, in_signature="iias", out_signature="u(ia{sv}av)")
    def GetLayout(self, parent, depth, names):
        submenu = node(3, {"label": dbus.String("Mehr"), "children-display": dbus.String("submenu")},
                       [node(4, {"label": dbus.String("Unterpunkt")})])
        root = node(0, {"children-display": dbus.String("submenu")}, [
            node(1, {"label": dbus.String("_Öffnen")}),
            node(2, {"type": dbus.String("separator")}),
            submenu,
            node(5, {"label": dbus.String("Versteckt"), "visible": dbus.Boolean(False)}),
        ])
        return dbus.UInt32(1), root

    @dbus.service.method(MENU, in_signature="i", out_signature="b")
    def AboutToShow(self, id):
        log.write(f"AboutToShow {id}\n")
        return False

    @dbus.service.method(MENU, in_signature="isvu")
    def Event(self, id, event, data, timestamp):
        log.write(f"Event {id} {event}\n")


Item(bus, "/StatusNotifierItem")
Menu(bus, "/Menu")
name = dbus.service.BusName(NAME, bus)


def register(*_):
    try:
        bus.get_object(WATCHER, "/StatusNotifierWatcher").RegisterStatusNotifierItem(NAME, dbus_interface=WATCHER)
    except dbus.DBusException:
        pass  # No watcher yet: it is registered when one appears.


bus.watch_name_owner(WATCHER, lambda owner: owner and register())
GLib.MainLoop().run()
