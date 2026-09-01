# Rebuild the desktop shell with GPUI

Astesia will replace its React/Tauri Legacy Shell in place with a standalone GPUI Shell and preserve the Tauri-independent Application Core. The internal application will use the pinned Zed editor stack through an isolated, local-only integration on Rust 1.97.1; the Legacy Shell remains only in version control and prior signed artifacts.

The shell embeds Zed's standalone `Editor` rather than initializing Zed `Client`, `Project`, or `Workspace` services. Astesia owns the surrounding workspace and application state so unused Zed authentication, collaboration, telemetry, and network paths remain dormant.

The rebuild deliberately provides no WebView migration bridge. Upgrade eligibility is determined by a Native State Probe, WebView-only state is unsupported, and existing MCP clients may require reconfiguration. External distribution, licensing, signing, and public updater guarantees are outside the internal rebuild; they must be reconsidered if that scope changes.
