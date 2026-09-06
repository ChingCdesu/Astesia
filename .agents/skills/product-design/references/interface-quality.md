# Interface quality

Load for visual implementation, material visual change, or full design review. Read
[gpui-kit-design.md](gpui-kit-design.md) first. The locked GPUI Kit source owns visual primitives; `DESIGN.md`
owns only Astesia-specific composition and semantic extensions.

Astesia is a desktop-native GPUI application for macOS, Windows, and Linux. Browser DOM detectors,
web-responsive conventions, and iOS or Android platform references do not validate it. Use current
GPUI Kit component APIs, native desktop behavior, repository acceptance evidence, and the running
application.

## Native operator standard

- Keep the query, data, and system state dominant. Brand expression lives in precise typography,
  spacing, engine identity, and interaction detail rather than decorative chrome.
- Use the GPUI Kit component that owns the interaction before composing raw GPUI elements. Configure
  its semantic style, size, density, elevation, tooltip, and accessibility APIs instead of copying
  its rendered values.
- Use active-theme Kit surface, element, border, text, and status roles. Engine colors identify
  database types; status colors accompany words or icons and do not carry meaning alone.
- Keep persistent panes on the active theme background and surface roles. Use popover roles
  and dialog roles for the temporary content those roles name.
- Preserve the sidebar, tab strip, work area, and status hierarchy. Hiding the sidebar may increase
  space; the active tab and runtime context remain visible.
- Keep dense operator controls scannable and keyboard reachable. Pointer affordances supplement,
  rather than replace, focus and shortcuts.
- Preserve platform-native window, prompt, file, clipboard, and text-input behavior. Astesia owns
  the workspace around the GPUI Kit editor.

## Content stress

Inspect both languages and compact and wide layouts with long profile names,
qualified object names, endpoints, values, errors, and task details. Growing content scrolls or
truncates within its owning region; it does not resize the application hierarchy or hide the primary
action. The supported window floor is 960 by 600; also inspect a representative wide window.

## Bounded visual verification

Build the complete requested change before the first visual pass. Inspect affected states, light and
dark appearance, compact and wide windows, focus, and overflow together. Fix all verified defects in
one batch, then use at most one confirmation pass. Record the exact platform and states inspected.

Static source review can validate component and token selection, but only a rendered native surface
can validate alignment, hierarchy, clipping, density, focus visibility, and motion. When native
rendering is unavailable, finish a read-only review as source-verified and enumerate the visual and
interaction claims that remain unverified; treat a requested runtime conclusion as blocked.
