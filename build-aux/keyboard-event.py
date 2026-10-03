#!/usr/bin/env python3
"""Deliver real X11 key events only to the isolated Brooklet regression window."""
import ctypes as C
import ctypes.util
import sys

x11 = C.CDLL(ctypes.util.find_library("X11"))
xtst = C.CDLL(ctypes.util.find_library("Xtst"))
Window = C.c_ulong
Display = C.c_void_p
x11.XOpenDisplay.argtypes = [C.c_char_p]
x11.XOpenDisplay.restype = Display
x11.XDefaultRootWindow.argtypes = [Display]
x11.XDefaultRootWindow.restype = Window
x11.XQueryTree.argtypes = [Display, Window, C.POINTER(Window), C.POINTER(Window), C.POINTER(C.POINTER(Window)), C.POINTER(C.c_uint)]
x11.XFetchName.argtypes = [Display, Window, C.POINTER(C.c_char_p)]
x11.XFree.argtypes = [C.c_void_p]
x11.XSetInputFocus.argtypes = [Display, Window, C.c_int, C.c_ulong]
x11.XStringToKeysym.argtypes = [C.c_char_p]
x11.XStringToKeysym.restype = C.c_ulong
x11.XKeysymToKeycode.argtypes = [Display, C.c_ulong]
x11.XKeysymToKeycode.restype = C.c_uint
x11.XSync.argtypes = [Display, C.c_int]
x11.XCloseDisplay.argtypes = [Display]
xtst.XTestFakeKeyEvent.argtypes = [Display, C.c_uint, C.c_int, C.c_ulong]

window_title, key, mode, *modifiers = sys.argv[1:]
if not window_title.startswith("Brooklet Keyboard Regression "):
    raise SystemExit("Refusing to target a window outside the keyboard regression")

display = x11.XOpenDisplay(None)
if not display:
    raise SystemExit("The keyboard event regression requires an X11 display")

def find(window):
    name = C.c_char_p()
    if x11.XFetchName(display, window, C.byref(name)) and name.value:
        matched = name.value == window_title.encode()
        x11.XFree(name)
        if matched:
            return window
    root, parent, children, count = Window(), Window(), C.POINTER(Window)(), C.c_uint()
    if x11.XQueryTree(display, window, C.byref(root), C.byref(parent), C.byref(children), C.byref(count)):
        try:
            for child in children[:count.value]:
                result = find(child)
                if result:
                    return result
        finally:
            if children:
                x11.XFree(children)
    return None

window = find(x11.XDefaultRootWindow(display))
if not window:
    raise SystemExit("The isolated keyboard regression window is unavailable")
x11.XSetInputFocus(display, window, 2, 0)
if mode == "focus":
    # Let the app process FocusIn before the first physical key is injected.
    x11.XSync(display, False)
    x11.XCloseDisplay(display)
    raise SystemExit(0)

def event(name, pressed):
    keycode = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
    if not keycode or not xtst.XTestFakeKeyEvent(display, keycode, pressed, 0):
        raise SystemExit(f"Could not deliver {name}")

if mode != "release":
    for modifier in modifiers:
        event(modifier, True)
    event(key, True)
if mode != "press":
    event(key, False)
    for modifier in reversed(modifiers):
        event(modifier, False)
x11.XSync(display, False)
x11.XCloseDisplay(display)
