+++
title = "OS keychain (macOS Keychain, Windows Credential Manager, Secret Service)"
level = "external"
technology = "keyring crate"
+++
Holds the key `kitsu login` got, and nothing else of Kitsu's. When there's
no keychain there's no login: Kitsu never keeps the key in a file instead.
