#!/usr/bin/env python3
"""Route the standard FileManager1 folder/reveal API to Kitty/Yazi."""
import asyncio
import os
from pathlib import Path
import subprocess
from urllib.parse import unquote, urlsplit


def local_path(uri):
    parsed = urlsplit(uri)
    if parsed.scheme != "file" or parsed.netloc not in ("", "localhost"):
        raise ValueError("Yazi FileManager1 accepts only local file:// URIs")
    path = unquote(parsed.path, errors="strict")
    if not path.startswith("/") or "\x00" in path:
        raise ValueError("invalid local file URI")
    return path


def launch(uris, folders=False):
    # Validate the entire request before opening any windows.
    paths = list(dict.fromkeys(local_path(uri) for uri in uris))
    for path in paths:
        if folders and not Path(path).is_dir():
            raise ValueError("ShowFolders requires a directory")
    for path in paths:
        # A file path tells Yazi to browse its parent with that file hovered.
        subprocess.Popen([os.environ["DOJO_KITTY"], "--title", "Yazi",
                          os.environ["DOJO_YAZI"], path], start_new_session=True)


async def main():
    from dbus_next import BusType, DBusError, NameFlag, RequestNameReply
    from dbus_next.aio import MessageBus
    from dbus_next.service import ServiceInterface, method

    class FileManager(ServiceInterface):
        def __init__(self):
            super().__init__("org.freedesktop.FileManager1")

        def dispatch(self, uris, folders=False):
            try:
                launch(uris, folders)
            except (ValueError, UnicodeError, OSError) as error:
                raise DBusError("org.freedesktop.DBus.Error.InvalidArgs", str(error))

        @method()
        def ShowFolders(self, uris: 'as', startup_id: 's'):
            self.dispatch(uris, folders=True)

        @method()
        def ShowItems(self, uris: 'as', startup_id: 's'):
            self.dispatch(uris)

        @method()
        def ShowItemProperties(self, uris: 'as', startup_id: 's'):
            # Yazi has no graphical properties dialog: reveal the item instead.
            self.dispatch(uris)

    bus = await MessageBus(bus_type=BusType.SESSION).connect()
    bus.export("/org/freedesktop/FileManager1", FileManager())
    reply = await bus.request_name("org.freedesktop.FileManager1", NameFlag.DO_NOT_QUEUE)
    if reply != RequestNameReply.PRIMARY_OWNER:
        raise RuntimeError("another file manager already owns FileManager1")
    await bus.wait_for_disconnect()


if __name__ == "__main__":
    asyncio.run(main())
