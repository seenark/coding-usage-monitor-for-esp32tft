# Keep Device setup credentials in unencrypted NVS

The Device stores its Wi-Fi network and broker details in NVS so a reboot or power loss does not force Device setup again. NVS encryption and flash encryption stay off: both are irreversible on this board, and a mistake can require reflashing. This reverses the earlier plan to defer credential storage to a final security phase.

Anyone with physical access to the Device can read the Wi-Fi password from flash. The broker password is a read-only, LAN-only account, so its exposure is low. Re-running Device setup from the Device replaces the stored values.
