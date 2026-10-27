#!/usr/bin/env python3
"""A small stand-in for BlueZ on the bus in $DBUS_SYSTEM_BUS_ADDRESS: one adapter (off), a paired headset and a
speaker that only a search finds. Method calls and property writes are appended to the file named by the first
argument, one line each, so a test can see what the shell asked for. (The same as splice-shell's tests/fake_bluez.py.)"""
import sys

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

PROPERTIES = "org.freedesktop.DBus.Properties"
OBJECT_MANAGER = "org.freedesktop.DBus.ObjectManager"
ADAPTER = "org.bluez.Adapter1"
DEVICE = "org.bluez.Device1"
BATTERY = "org.bluez.Battery1"
ADAPTER_PATH = "/org/bluez/hci0"

log = open(sys.argv[1], "a", buffering=1)
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SystemBus()
objects = {}


def no_strings():
    return dbus.Array([], signature="s")


class Root(dbus.service.Object):
    @dbus.service.method(OBJECT_MANAGER, out_signature="a{oa{sa{sv}}}")
    def GetManagedObjects(self):
        return {path: obj.interfaces for path, obj in objects.items()}

    @dbus.service.signal(OBJECT_MANAGER, signature="oa{sa{sv}}")
    def InterfacesAdded(self, path, interfaces):
        pass

    @dbus.service.signal(OBJECT_MANAGER, signature="oas")
    def InterfacesRemoved(self, path, interfaces):
        pass


class Object(dbus.service.Object):
    def __init__(self, path, interfaces):
        super().__init__(bus, path)
        self.path = path
        self.interfaces = interfaces
        objects[path] = self

    def update(self, interface, changes):
        self.interfaces[interface].update(changes)
        self.PropertiesChanged(interface, dbus.Dictionary(changes, signature="sv"), no_strings())

    @dbus.service.method(PROPERTIES, in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return self.interfaces[interface][name]

    @dbus.service.method(PROPERTIES, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return dbus.Dictionary(self.interfaces.get(interface, {}), signature="sv")

    @dbus.service.method(PROPERTIES, in_signature="ssv")
    def Set(self, interface, name, value):
        log.write(f"set {self.path.rsplit('/', 1)[-1]} {name} {bool(value) if isinstance(value, dbus.Boolean) else value}\n")
        changes = {name: value}
        if name == "Powered":
            changes["PowerState"] = dbus.String("on" if value else "off")
        self.update(interface, changes)

    @dbus.service.signal(PROPERTIES, signature="sa{sv}as")
    def PropertiesChanged(self, interface, changed, invalidated):
        pass


class Adapter(Object):
    @dbus.service.method(ADAPTER)
    def StartDiscovery(self):
        log.write("call hci0 StartDiscovery\n")
        self.update(ADAPTER, {"Discovering": dbus.Boolean(True)})
        if "dev_22_22_22_22_22_22" not in [p.rsplit("/", 1)[-1] for p in objects]:
            speaker = device("22:22:22:22:22:22", "Lautsprecher", paired=False)
            root.InterfacesAdded(speaker.path, speaker.interfaces)

    @dbus.service.method(ADAPTER)
    def StopDiscovery(self):
        log.write("call hci0 StopDiscovery\n")
        self.update(ADAPTER, {"Discovering": dbus.Boolean(False)})

    @dbus.service.method(ADAPTER, in_signature="a{sv}")
    def SetDiscoveryFilter(self, properties):
        pass

    @dbus.service.method(ADAPTER, in_signature="o")
    def RemoveDevice(self, path):
        log.write(f"call hci0 RemoveDevice {path}\n")


class Device(Object):
    def name(self):
        return self.path.rsplit("/", 1)[-1]

    @dbus.service.method(DEVICE)
    def Connect(self):
        log.write(f"call {self.name()} Connect\n")
        self.update(DEVICE, {"Connected": dbus.Boolean(True)})

    @dbus.service.method(DEVICE)
    def Disconnect(self):
        log.write(f"call {self.name()} Disconnect\n")
        self.update(DEVICE, {"Connected": dbus.Boolean(False)})

    @dbus.service.method(DEVICE)
    def Pair(self):
        log.write(f"call {self.name()} Pair\n")
        self.update(DEVICE, {"Paired": dbus.Boolean(True), "Bonded": dbus.Boolean(True)})

    @dbus.service.method(DEVICE)
    def CancelPairing(self):
        log.write(f"call {self.name()} CancelPairing\n")


def device(address, alias, paired, battery=None):
    path = f"{ADAPTER_PATH}/dev_{address.replace(':', '_')}"
    interfaces = {
        DEVICE: {
            "Address": dbus.String(address), "Name": dbus.String(alias), "Alias": dbus.String(alias),
            "Icon": dbus.String("audio-headset"), "Paired": dbus.Boolean(paired), "Bonded": dbus.Boolean(paired),
            "Trusted": dbus.Boolean(paired), "Blocked": dbus.Boolean(False), "Connected": dbus.Boolean(False),
            "WakeAllowed": dbus.Boolean(False), "Adapter": dbus.ObjectPath(ADAPTER_PATH), "UUIDs": no_strings(),
        },
    }
    if battery is not None:
        interfaces[BATTERY] = {"Percentage": dbus.Byte(battery)}
    return Device(path, interfaces)


root = Root(bus, "/")
Adapter(ADAPTER_PATH, {
    ADAPTER: {
        "Address": dbus.String("00:11:22:33:44:55"), "Name": dbus.String("hci0"), "Alias": dbus.String("Desktop"),
        "Powered": dbus.Boolean(False), "PowerState": dbus.String("off"), "Discovering": dbus.Boolean(False),
        "Discoverable": dbus.Boolean(False), "DiscoverableTimeout": dbus.UInt32(180), "Pairable": dbus.Boolean(True),
        "PairableTimeout": dbus.UInt32(0), "UUIDs": no_strings(),
    },
})
device("11:11:11:11:11:11", "Kopfhörer", paired=True, battery=80)
# Taken last: once the name is on the bus, everything above is there.
name = dbus.service.BusName("org.bluez", bus)
log.write("ready\n")
GLib.MainLoop().run()
