# Security and build status

- **Binaries:** none published. Axentra OneDrive 0.1.0 is built from [source](../source/) on the computer that runs it, so there is no download to scan or sign.
- **Network:** connects only to Microsoft's sign-in service (`login.microsoftonline.com`), Microsoft Graph (`graph.microsoft.com`) and the Microsoft storage URLs Graph returns for downloads and uploads. No telemetry.
- **Sign-in:** OAuth 2.0 with PKCE through your own app registration; no client secret. Tokens are stored in `~/.local/share/axentra/onedrive/tokens.json` with owner-only permissions (0600).
- **Local access:** the OneDrive folder is a FUSE mount owned by your user, with kernel permission checks; other users can't open it.
