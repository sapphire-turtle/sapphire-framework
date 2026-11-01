# sapphire-bridge-desktop

Sapphire Bridge's desktop app: run bare, it opens the bridge's window — its status,
workgroup, devices, embedding, appearance, logs and About. `--page <status|workgroup|devices|embedding|appearance|logs|about>`
opens it on a page, or brings the window already open to the front on it; the apps' "Open
Sapphire Bridge" buttons start it that way.

Given a subcommand it is `sapphire-bridge`: `sapphire-bridge-desktop serve` runs the bridge,
and the service manager may start it so. The window is a client of the bridge; closing it
stops nothing.

Released as a binary (`sapphire-bridge-desktop-<os>-<arch>`) with each framework release.
The command line alone, for servers, is [`sapphire-bridge`](../headless/README.md).
