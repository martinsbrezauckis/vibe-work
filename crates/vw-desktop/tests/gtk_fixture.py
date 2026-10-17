#!/usr/bin/env python3
# GTK accessibility fixture; depends on PyGObject/GTK 3, spawned by the Rust live-bus test.
# Only test controls are created; the product app and UI never load this file.

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk

window = Gtk.Window(title="Desktop pilot fixture")
window.set_default_size(420, 240)
window.connect("destroy", Gtk.main_quit)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, margin=16)
window.add(box)

presses = 0
button = Gtk.Button(label="Press me")


def pressed(control):
    global presses
    presses += 1
    control.set_label(f"Pressed {presses}")


button.connect("clicked", pressed)
box.pack_start(button, False, False, 0)

ignored = Gtk.Button(label="Ignore press")
box.pack_start(ignored, False, False, 0)

entry = Gtk.Entry()
entry.get_accessible().set_name("Message")
box.pack_start(entry, False, False, 0)

reverting = Gtk.Entry()
reverting.set_text("kept")
reverting.get_accessible().set_name("Reverts text")
restoring = False


def restore_text(control):
    global restoring
    if not restoring:
        restoring = True
        control.set_text("kept")
        restoring = False


reverting.connect("changed", restore_text)
box.pack_start(reverting, False, False, 0)

label = Gtk.Label(label="Read only label")
box.pack_start(label, False, False, 0)
window.show_all()


def focus_window():
    window.get_window().focus(Gdk.CURRENT_TIME)
    print("fixture ready", flush=True)
    return GLib.SOURCE_REMOVE


GLib.idle_add(focus_window)
Gtk.main()
