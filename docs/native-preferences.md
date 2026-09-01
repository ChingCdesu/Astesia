# Native desktop preferences

The GPUI desktop shell owns a versioned JSON preferences file at
`~/Library/Application Support/com.astesia.app/preferences.json` on macOS. It does not read or
import WebView `localStorage`.

The initial native defaults are:

- theme: follow the operating system, using the bundled One Light and One Dark themes;
- language: Simplified Chinese (`zh-CN`);
- connection sidebar: visible;
- skipped update version: unset;
- MCP endpoint: unset until the native service starts, then allocated on loopback with a new token.

Only theme, language, and sidebar visibility are persisted in the Milestone 3 shell. A missing file
uses these defaults. If the file is unreadable, malformed, or from an unsupported schema version,
Astesia shows a warning and starts with the defaults instead of importing legacy state.
