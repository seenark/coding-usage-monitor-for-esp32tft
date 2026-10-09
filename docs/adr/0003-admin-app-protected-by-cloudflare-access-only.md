# Protect the Admin app with Cloudflare Access only

The Admin app has no login of its own. The operator publishes it through a Cloudflare Tunnel and relies on Cloudflare Access to decide who can reach it. The app shows the broker host, port, username, and password needed for Device setup, and it can change Server settings and Display layout.

If the Access policy is missing or wrong, anyone can read the broker password and change settings. The password only works from the LAN and only reads usage topics, which limits the damage. The Admin app is published on the host through one configured address (`ADMIN_BIND_ADDRESS`), never on every interface.
